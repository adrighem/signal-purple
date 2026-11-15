// SPDX-License-Identifier: AGPL-3.0-only
use presage::libsignal_service::protocol::SignalProtocolError;
use presage::store::TransientStoreError;
use presage_store_sqlite::SqliteStoreError;

pub(crate) fn signal_protocol_error_is_transient(error: &SignalProtocolError) -> bool {
    match error {
        SignalProtocolError::InvalidState(scope, message) => {
            let scope_lower = scope.to_ascii_lowercase();
            let message_lower = message.to_ascii_lowercase();
            (scope_lower == "sqlite"
                || scope_lower == "presage sqlite store error"
                || scope_lower.contains("sqlite"))
                && (message_lower.contains("pool timed out")
                    || message_lower.contains("timed out")
                    || message_lower.contains("locked")
                    || message_lower.contains("busy")
                    || message_lower.contains("code: 5")
                    || message_lower.contains("code: 6")
                    || message_lower.contains("code: 1032"))
        }
        SignalProtocolError::ApplicationCallbackError(_, source) => {
            let source: &(dyn std::error::Error + 'static) = source.as_ref();
            source.is::<TransientStoreError>()
        }
        _ => false,
    }
}

/// Store error transience comes from `SqliteStoreError::is_transient()` in
/// presage-store-sqlite (single source of truth). Only the `Protocol` variant is
/// classified here, because the store cannot see signal-purple's string-based
/// handling of old-format `InvalidState` errors.
pub(crate) fn sqlite_store_error_is_transient(error: &SqliteStoreError) -> bool {
    match error {
        SqliteStoreError::Protocol(error) => signal_protocol_error_is_transient(error),
        other => other.is_transient(),
    }
}

/// An error from a `StorageRepository` operation, distinguishing errors that are
/// worth retrying (a busy/locked database, a pool timeout, a row not yet visible)
/// from ones that are not, so callers no longer have to guess from a `String`.
#[derive(Debug)]
pub(crate) enum StorageError {
    Store {
        context: &'static str,
        source: SqliteStoreError,
    },
    NotFound(&'static str),
}

impl StorageError {
    pub(crate) fn store(context: &'static str, source: SqliteStoreError) -> Self {
        StorageError::Store { context, source }
    }

    /// Whether retrying the operation is worthwhile: a transient store error, or a
    /// row that may simply not be visible yet.
    pub(crate) fn is_transient(&self) -> bool {
        match self {
            StorageError::Store { source, .. } => sqlite_store_error_is_transient(source),
            StorageError::NotFound(_) => true,
        }
    }
}

impl std::fmt::Display for StorageError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            StorageError::Store { context, source } => write!(f, "{context}: {source}"),
            StorageError::NotFound(context) => write!(f, "{context}"),
        }
    }
}

impl std::error::Error for StorageError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_sqlite_pool_and_lock_contention_as_transient() {
        assert!(sqlite_store_error_is_transient(&SqliteStoreError::Db(
            sqlx::Error::PoolTimedOut
        )));
        assert!(!sqlite_store_error_is_transient(&SqliteStoreError::Db(
            sqlx::Error::PoolClosed
        )));
        assert!(!sqlite_store_error_is_transient(
            &SqliteStoreError::InvalidFormat
        ));
        assert!(!sqlite_store_error_is_transient(&SqliteStoreError::Db(
            sqlx::Error::RowNotFound
        )));
    }

    #[test]
    fn storage_error_transience_matches_its_source() {
        let transient = StorageError::store(
            "Could not read synchronized Signal contacts",
            SqliteStoreError::Db(sqlx::Error::PoolTimedOut),
        );
        assert!(transient.is_transient());

        let not_found = StorageError::NotFound("row not visible yet");
        assert!(not_found.is_transient());
    }

    #[test]
    fn classifies_signal_protocol_pool_timeouts_as_transient() {
        let pool_timeout = SignalProtocolError::InvalidState(
            "sqlite",
            "pool timed out while waiting for an open connection".into(),
        );
        let presage_pool_timeout = SignalProtocolError::InvalidState(
            "presage sqlite store error",
            "Pool timed out while waiting for an open connection".into(),
        );
        let permanent = SignalProtocolError::InvalidProtobufEncoding;

        assert!(signal_protocol_error_is_transient(&pool_timeout));
        assert!(signal_protocol_error_is_transient(&presage_pool_timeout));
        assert!(!signal_protocol_error_is_transient(&permanent));
    }

    #[test]
    fn classifies_transient_store_callback_errors_as_transient() {
        let transient = SignalProtocolError::ApplicationCallbackError(
            "presage sqlite store",
            Box::new(TransientStoreError("pool timed out".into())),
        );
        let other_callback = SignalProtocolError::ApplicationCallbackError(
            "presage sqlite store",
            Box::new(std::fmt::Error),
        );

        assert!(signal_protocol_error_is_transient(&transient));
        assert!(!signal_protocol_error_is_transient(&other_callback));
    }

    #[test]
    fn classifies_protocol_wrapped_store_errors() {
        let transient = SqliteStoreError::Protocol(SignalProtocolError::ApplicationCallbackError(
            "presage sqlite store",
            Box::new(TransientStoreError("pool timed out".into())),
        ));
        let permanent = SqliteStoreError::Protocol(SignalProtocolError::InvalidProtobufEncoding);

        assert!(sqlite_store_error_is_transient(&transient));
        assert!(!sqlite_store_error_is_transient(&permanent));
    }
}
