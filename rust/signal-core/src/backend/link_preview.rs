// SPDX-License-Identifier: AGPL-3.0-only
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use futures::FutureExt;
use futures::channel::oneshot;
use presage::model::previews::extract_urls_from_text_block;
use presage::proto::Preview;
use tokio::sync::{Semaphore, mpsc as tokio_mpsc};

use super::command::{Command, OutgoingPreview};
use super::protocol::SignalProtocol;

/// Hard cap on fetching the linked page and uploading its image.
pub(crate) const LINK_PREVIEW_TIMEOUT: Duration = Duration::from_secs(5);
const MAX_CONCURRENT_PREVIEWS: usize = 2;

/// The link Signal clients would preview, exactly as written in `body`.
/// Receivers drop a preview whose URL is not one of the body's links.
pub(crate) fn first_preview_link(body: &str) -> Option<String> {
    extract_urls_from_text_block(body).into_iter().next()
}

fn preview_matches_body(preview: &Preview, body: &str) -> bool {
    preview
        .url
        .as_deref()
        .is_some_and(|url| first_preview_link(body).as_deref() == Some(url))
}

/// Builds a preview for `body`, or `None` when it has no previewable link or
/// generation fails or exceeds `limit`. Never fails the send it belongs to.
pub(crate) async fn generate_link_preview<M: SignalProtocol>(
    manager: &M,
    body: &str,
    limit: Duration,
) -> Option<Preview> {
    first_preview_link(body)?;
    let generation = std::panic::AssertUnwindSafe(manager.link_preview(body)).catch_unwind();
    match tokio::time::timeout(limit, generation).await {
        Ok(Ok(Ok(Some(preview)))) if preview_matches_body(&preview, body) => Some(preview),
        Ok(Ok(Ok(Some(_)))) => {
            tracing::debug!("Discarding a link preview whose link is not in the message");
            None
        }
        Ok(Ok(Ok(None))) => None,
        Ok(Ok(Err(error))) => {
            tracing::debug!(%error, "Sending without a link preview");
            None
        }
        Ok(Err(_)) => {
            tracing::debug!("Link preview generation panicked; sending without it");
            None
        }
        Err(_) => {
            tracing::debug!("Link preview generation timed out; sending without it");
            None
        }
    }
}

/// Like [`generate_link_preview`], but waits for a fetch slot only when the
/// body has a link, so link-free sends are never held behind other fetches.
async fn bounded_link_preview<M: SignalProtocol>(
    manager: &M,
    permits: &Semaphore,
    body: &str,
    limit: Duration,
) -> Option<Preview> {
    first_preview_link(body)?;
    let _permit = permits.acquire().await.ok()?;
    generate_link_preview(manager, body, limit).await
}

/// Whether `command` should wait for a link preview before it is sent. Group
/// sends that will be refused anyway skip the fetch.
pub(crate) fn needs_link_preview(command: &Command, groups_authoritative: bool) -> bool {
    match command {
        Command::SendMessage {
            preview: OutgoingPreview::Unresolved,
            ..
        } => true,
        Command::SendGroupMessage {
            preview: OutgoingPreview::Unresolved,
            ..
        } => groups_authoritative,
        _ => false,
    }
}

type ConversationKey = (bool, String);

fn text_send(command: &Command) -> Option<(ConversationKey, &str)> {
    match command {
        Command::SendMessage {
            recipient, message, ..
        } => Some(((false, recipient.clone()), message)),
        Command::SendGroupMessage {
            group_key, message, ..
        } => Some(((true, group_key.clone()), message)),
        _ => None,
    }
}

fn with_preview(command: Command, resolved: Option<Preview>) -> Command {
    let resolved = OutgoingPreview::Resolved(resolved.map(Box::new));
    match command {
        Command::SendMessage {
            request_id,
            recipient,
            message,
            ..
        } => Command::SendMessage {
            request_id,
            recipient,
            message,
            preview: resolved,
        },
        Command::SendGroupMessage {
            request_id,
            group_key,
            message,
            ..
        } => Command::SendGroupMessage {
            request_id,
            group_key,
            message,
            preview: resolved,
        },
        command => command,
    }
}

/// Text sends waiting for their link preview, off the actor so receiving and
/// other work continue meanwhile. Previews are generated concurrently, but
/// each conversation's sends are released in submission order. One shared
/// manager keeps Presage's lazily built HTTP client for the core's lifetime.
pub(crate) struct LinkPreviewPipeline<M> {
    manager: Rc<M>,
    tasks: tokio::task::JoinSet<()>,
    tails: HashMap<ConversationKey, oneshot::Receiver<()>>,
    ready_tx: tokio_mpsc::UnboundedSender<Command>,
    ready_rx: tokio_mpsc::UnboundedReceiver<Command>,
    permits: Arc<Semaphore>,
    limit: Duration,
}

impl<M: SignalProtocol + 'static> LinkPreviewPipeline<M> {
    pub(crate) fn new(manager: M, limit: Duration) -> Self {
        let (ready_tx, ready_rx) = tokio_mpsc::unbounded_channel();
        Self {
            manager: Rc::new(manager),
            tasks: tokio::task::JoinSet::new(),
            tails: HashMap::new(),
            ready_tx,
            ready_rx,
            permits: Arc::new(Semaphore::new(MAX_CONCURRENT_PREVIEWS)),
            limit,
        }
    }

    /// Resolves the preview of a text send on the local task set. Any other
    /// command is released unchanged.
    pub(crate) fn submit(&mut self, command: Command) {
        while self.tasks.try_join_next().is_some() {}
        let Some((conversation, _)) = text_send(&command) else {
            let _ = self.ready_tx.send(command);
            return;
        };
        self.tails
            .retain(|_, tail| matches!(tail.try_recv(), Ok(None)));
        let (done_tx, done_rx) = oneshot::channel();
        let previous = self.tails.insert(conversation, done_rx);
        let manager = Rc::clone(&self.manager);
        let ready = self.ready_tx.clone();
        let permits = Arc::clone(&self.permits);
        let limit = self.limit;
        self.tasks.spawn_local(async move {
            let preview = match text_send(&command) {
                Some((_, body)) => bounded_link_preview(&*manager, &permits, body, limit).await,
                None => None,
            };
            if let Some(previous) = previous {
                let _ = previous.await;
            }
            let _ = ready.send(with_preview(command, preview));
            let _ = done_tx.send(());
        });
    }

    /// The next text send whose preview is resolved.
    pub(crate) async fn next(&mut self) -> Command {
        match self.ready_rx.recv().await {
            Some(command) => command,
            // The pipeline owns a sender, so the channel never closes.
            None => std::future::pending().await,
        }
    }

    pub(crate) fn abort(&mut self) {
        self.tasks.abort_all();
        self.tails.clear();
    }
}

/// The next resolved send from an enabled pipeline; pending forever otherwise.
pub(crate) async fn next_resolved_send<M: SignalProtocol + 'static>(
    pipeline: &mut Option<LinkPreviewPipeline<M>>,
) -> Command {
    match pipeline {
        Some(pipeline) => pipeline.next().await,
        None => std::future::pending().await,
    }
}

#[cfg(test)]
mod tests {
    use presage::libsignal_service::content::ContentBody;

    use super::*;
    use crate::backend::coordinator::{
        CommandContext, DepartedGroups, MessageTimestampAllocator, MetadataCache, handle_command,
    };
    use crate::backend::protocol::fake::FakeSignalProtocol;
    use crate::store::traits::fake::FakeStorageRepository;

    const RECIPIENT: &str = "00000000-0000-4000-8000-000000000042";
    const OTHER_RECIPIENT: &str = "00000000-0000-4000-8000-000000000043";

    fn preview_for(url: &str) -> Preview {
        Preview {
            url: Some(url.to_owned()),
            title: Some("Signal".to_owned()),
            ..Default::default()
        }
    }

    fn send(request_id: u64, recipient: &str, message: &str) -> Command {
        Command::SendMessage {
            request_id,
            recipient: recipient.to_owned(),
            message: message.to_owned(),
            preview: OutgoingPreview::Unresolved,
        }
    }

    fn resolved(command: Command) -> (u64, Option<Preview>) {
        match command {
            Command::SendMessage {
                request_id,
                preview: OutgoingPreview::Resolved(preview),
                ..
            } => (request_id, preview.map(|preview| *preview)),
            other => panic!("expected a resolved direct send, got {other:?}"),
        }
    }

    fn run_local<F: std::future::Future>(future: F) -> F::Output {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        tokio::task::LocalSet::new().block_on(&runtime, future)
    }

    #[test]
    fn finds_the_first_https_link_as_written() {
        assert_eq!(
            first_preview_link("read https://signal.org/blog/ now").as_deref(),
            Some("https://signal.org/blog/")
        );
        assert_eq!(
            first_preview_link("http://signal.org then https://signal.org/download").as_deref(),
            Some("https://signal.org/download")
        );
        assert_eq!(
            first_preview_link("https://signal.org/a and https://signal.org/b").as_deref(),
            Some("https://signal.org/a")
        );
        assert_eq!(first_preview_link("only http://signal.org here"), None);
        assert_eq!(first_preview_link("no links at all"), None);
        assert_eq!(first_preview_link("https://localhost/page"), None);
    }

    #[test]
    fn trims_trailing_punctuation_like_signal() {
        for (body, expected) in [
            ("(see https://signal.org/blog)", "https://signal.org/blog"),
            ("see https://signal.org/blog.", "https://signal.org/blog"),
            ("see https://signal.org/blog, ok", "https://signal.org/blog"),
            ("see https://signal.org/blog!?", "https://signal.org/blog"),
            (
                "wiki https://en.wikipedia.org/wiki/Signal_(software).",
                "https://en.wikipedia.org/wiki/Signal_(software)",
            ),
        ] {
            assert_eq!(
                first_preview_link(body).as_deref(),
                Some(expected),
                "{body}"
            );
        }
    }

    #[test]
    fn previews_must_name_the_link_in_the_body() {
        let body = "look (https://signal.org/blog).";
        assert!(preview_matches_body(
            &preview_for("https://signal.org/blog"),
            body
        ));
        assert!(!preview_matches_body(
            &preview_for("https://signal.org/blog)."),
            body
        ));
        assert!(!preview_matches_body(&Preview::default(), body));
    }

    #[tokio::test]
    async fn generates_a_preview_for_the_exact_body_link() {
        let protocol = FakeSignalProtocol {
            link_preview: Some(Ok(preview_for("https://signal.org/blog"))),
            ..Default::default()
        };
        let body = "news: https://signal.org/blog.";

        let preview = generate_link_preview(&protocol, body, LINK_PREVIEW_TIMEOUT)
            .await
            .unwrap();

        assert_eq!(preview.url.as_deref(), Some("https://signal.org/blog"));
        assert!(body.contains(preview.url.as_deref().unwrap()));
        assert_eq!(*protocol.link_preview_requests.lock().unwrap(), vec![body]);
    }

    #[tokio::test]
    async fn skips_the_fetch_without_a_previewable_link() {
        let protocol = FakeSignalProtocol {
            link_preview: Some(Ok(preview_for("http://signal.org"))),
            ..Default::default()
        };

        assert!(
            generate_link_preview(&protocol, "plain http://signal.org", LINK_PREVIEW_TIMEOUT)
                .await
                .is_none()
        );
        assert!(protocol.link_preview_requests.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn falls_back_without_a_preview_on_failure_mismatch_or_panic() {
        let body = "https://signal.org/blog";
        let failing = FakeSignalProtocol {
            link_preview: Some(Err("connection refused".to_owned())),
            ..Default::default()
        };
        let mismatched = FakeSignalProtocol {
            link_preview: Some(Ok(preview_for("https://signal.org/other"))),
            ..Default::default()
        };
        let panicking = FakeSignalProtocol {
            link_preview_panics: true,
            ..Default::default()
        };

        for protocol in [failing, mismatched, panicking] {
            assert!(
                generate_link_preview(&protocol, body, LINK_PREVIEW_TIMEOUT)
                    .await
                    .is_none()
            );
            assert_eq!(protocol.link_preview_requests.lock().unwrap().len(), 1);
        }
    }

    #[tokio::test]
    async fn times_out_slow_previews() {
        let protocol = FakeSignalProtocol {
            link_preview: Some(Ok(preview_for("https://signal.org"))),
            link_preview_delay: Some(Duration::from_secs(30)),
            ..Default::default()
        };
        let started = std::time::Instant::now();

        let preview =
            generate_link_preview(&protocol, "https://signal.org", Duration::from_millis(50)).await;

        assert!(preview.is_none());
        assert!(started.elapsed() < Duration::from_secs(5));
    }

    #[test]
    fn only_unresolved_text_sends_wait_for_previews() {
        assert!(needs_link_preview(&send(1, RECIPIENT, "hi"), false));
        let group = Command::SendGroupMessage {
            request_id: 2,
            group_key: "00".repeat(32),
            message: "https://signal.org".to_owned(),
            preview: OutgoingPreview::Unresolved,
        };
        assert!(needs_link_preview(&group, true));
        assert!(!needs_link_preview(&group, false));
        let already = with_preview(send(3, RECIPIENT, "hi"), None);
        assert!(!needs_link_preview(&already, true));
        let typing = Command::SetTyping {
            request_id: 4,
            recipient: RECIPIENT.to_owned(),
            typing: true,
        };
        assert!(!needs_link_preview(&typing, true));
    }

    #[test]
    fn keeps_conversation_order_while_other_conversations_proceed() {
        run_local(async {
            let protocol = FakeSignalProtocol {
                link_preview: Some(Ok(preview_for("https://signal.org"))),
                link_preview_delay: Some(Duration::from_millis(200)),
                ..Default::default()
            };
            let mut pipeline = LinkPreviewPipeline::new(protocol, LINK_PREVIEW_TIMEOUT);

            pipeline.submit(send(1, RECIPIENT, "https://signal.org"));
            pipeline.submit(send(2, RECIPIENT, "no link"));
            pipeline.submit(send(3, OTHER_RECIPIENT, "no link"));

            let first = resolved(pipeline.next().await);
            let second = resolved(pipeline.next().await);
            let third = resolved(pipeline.next().await);
            assert_eq!(first, (3, None));
            assert_eq!(second.0, 1);
            assert_eq!(
                second.1.and_then(|preview| preview.url).as_deref(),
                Some("https://signal.org")
            );
            assert_eq!(third, (2, None));
        });
    }

    #[test]
    fn releases_a_send_after_its_preview_times_out() {
        run_local(async {
            let protocol = FakeSignalProtocol {
                link_preview: Some(Ok(preview_for("https://signal.org"))),
                link_preview_delay: Some(Duration::from_secs(30)),
                ..Default::default()
            };
            let mut pipeline = LinkPreviewPipeline::new(protocol, Duration::from_millis(50));

            pipeline.submit(send(1, RECIPIENT, "https://signal.org"));
            pipeline.submit(send(2, RECIPIENT, "after"));

            assert_eq!(resolved(pipeline.next().await), (1, None));
            assert_eq!(resolved(pipeline.next().await), (2, None));
        });
    }

    async fn handle_direct_send(protocol: &mut FakeSignalProtocol, command: Command) {
        let repo = FakeStorageRepository::new();
        let (sink, _events) = crate::event_queue::event_queue(4).unwrap();
        handle_command(
            CommandContext {
                manager: protocol,
                repo: &repo,
                sink: &sink,
                departed_groups: &DepartedGroups::default(),
                groups_authoritative: true,
                metadata_cache: &MetadataCache::default(),
                timestamps: &MessageTimestampAllocator::default(),
            },
            command,
        )
        .await;
    }

    fn sent_previews(protocol: &FakeSignalProtocol) -> Vec<Vec<Preview>> {
        protocol
            .sent_messages
            .lock()
            .unwrap()
            .iter()
            .map(|(_, body, _)| match body {
                ContentBody::DataMessage(message) => message.preview.clone(),
                other => panic!("expected a data message, got {other:?}"),
            })
            .collect()
    }

    #[tokio::test]
    async fn disabled_previews_send_without_fetching() {
        let mut protocol = FakeSignalProtocol {
            link_preview: Some(Ok(preview_for("https://signal.org"))),
            ..Default::default()
        };

        handle_direct_send(&mut protocol, send(1, RECIPIENT, "https://signal.org")).await;

        assert!(protocol.link_preview_requests.lock().unwrap().is_empty());
        assert_eq!(sent_previews(&protocol), vec![Vec::<Preview>::new()]);
    }

    #[tokio::test]
    async fn resolved_previews_are_attached_to_the_sent_message() {
        let mut protocol = FakeSignalProtocol::default();
        let preview = preview_for("https://signal.org");
        let command = with_preview(
            send(1, RECIPIENT, "https://signal.org"),
            Some(preview.clone()),
        );

        handle_direct_send(&mut protocol, command).await;

        assert_eq!(sent_previews(&protocol), vec![vec![preview]]);
    }
}
