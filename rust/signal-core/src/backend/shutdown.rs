// SPDX-License-Identifier: AGPL-3.0-only
use std::future::Future;
use std::time::Duration;

use futures::channel::oneshot;
use futures::pin_mut;
use tokio::sync::watch;

pub(crate) async fn wait_for_shutdown(shutdown: &mut watch::Receiver<bool>) {
    if *shutdown.borrow() {
        return;
    }
    let _ = shutdown.changed().await;
}

pub(crate) async fn await_or_shutdown<F>(
    future: F,
    shutdown: &mut watch::Receiver<bool>,
) -> Option<F::Output>
where
    F: Future,
{
    if *shutdown.borrow() {
        return None;
    }
    pin_mut!(future);
    tokio::select! {
        biased;
        _ = wait_for_shutdown(shutdown) => None,
        output = &mut future => Some(output),
    }
}

pub(crate) async fn finish_shutdown_cleanup<F>(future: F, timeout: Duration) -> bool
where
    F: Future<Output = ()>,
{
    tokio::time::timeout(timeout, future).await.is_ok()
}

pub(crate) async fn run_after_start_signal<F>(start: oneshot::Receiver<()>, operation: F)
where
    F: Future<Output = ()>,
{
    if start.await.is_ok() {
        operation.await;
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};

    use super::*;

    struct DropFlag(Arc<AtomicBool>);

    impl Drop for DropFlag {
        fn drop(&mut self) {
            self.0.store(true, Ordering::Release);
        }
    }

    #[test]
    fn shutdown_boundary_returns_completed_phase_output() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        runtime.block_on(async {
            let (_shutdown_tx, mut shutdown_rx) = watch::channel(false);

            assert_eq!(
                await_or_shutdown(async { 42 }, &mut shutdown_rx).await,
                Some(42)
            );
        });
    }

    #[test]
    fn shutdown_boundary_does_not_poll_after_shutdown() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        runtime.block_on(async {
            let (shutdown_tx, mut shutdown_rx) = watch::channel(false);
            let polled = Arc::new(AtomicBool::new(false));
            let phase_polled = Arc::clone(&polled);
            shutdown_tx.send(true).unwrap();

            let outcome = await_or_shutdown(
                async move {
                    phase_polled.store(true, Ordering::Release);
                    42
                },
                &mut shutdown_rx,
            )
            .await;

            assert_eq!(outcome, None);
            assert!(!polled.load(Ordering::Acquire));
        });
    }

    #[test]
    fn contact_sync_work_waits_for_the_queue_drain_signal() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        runtime.block_on(async {
            let (start_tx, start_rx) = oneshot::channel();
            let polled = Arc::new(AtomicBool::new(false));
            let operation_polled = Arc::clone(&polled);
            let gated = run_after_start_signal(start_rx, async move {
                operation_polled.store(true, Ordering::Release);
            });
            pin_mut!(gated);

            assert!(
                tokio::time::timeout(Duration::from_millis(10), gated.as_mut())
                    .await
                    .is_err()
            );
            assert!(!polled.load(Ordering::Acquire));

            start_tx.send(()).unwrap();
            tokio::time::timeout(Duration::from_secs(1), gated.as_mut())
                .await
                .expect("contact sync gate did not open");
            assert!(polled.load(Ordering::Acquire));
        });
    }

    #[test]
    fn shutdown_boundary_drops_a_pending_phase_promptly() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        runtime.block_on(async {
            let (shutdown_tx, mut shutdown_rx) = watch::channel(false);
            let (started_tx, started_rx) = oneshot::channel();
            let dropped = Arc::new(AtomicBool::new(false));
            let phase_dropped = Arc::clone(&dropped);
            let phase = async move {
                let _drop_flag = DropFlag(phase_dropped);
                let _ = started_tx.send(());
                std::future::pending::<()>().await;
            };
            let signal_shutdown = async move {
                started_rx.await.expect("phase did not start");
                shutdown_tx.send(true).expect("shutdown receiver closed");
            };

            let outcome = tokio::time::timeout(Duration::from_secs(1), async {
                tokio::join!(await_or_shutdown(phase, &mut shutdown_rx), signal_shutdown).0
            })
            .await
            .expect("shutdown boundary did not complete");

            assert_eq!(outcome, None);
            assert!(dropped.load(Ordering::Acquire));
        });
    }

    #[test]
    fn shutdown_cleanup_drops_work_after_its_deadline() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        runtime.block_on(async {
            let dropped = Arc::new(AtomicBool::new(false));
            let cleanup_dropped = Arc::clone(&dropped);
            let cleanup = async move {
                let _drop_flag = DropFlag(cleanup_dropped);
                std::future::pending::<()>().await;
            };

            let completed = finish_shutdown_cleanup(cleanup, Duration::from_millis(10)).await;

            assert!(!completed);
            assert!(dropped.load(Ordering::Acquire));
        });
    }
}
