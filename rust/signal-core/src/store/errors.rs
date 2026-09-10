// SPDX-License-Identifier: AGPL-3.0-only
use presage::libsignal_service::protocol::SignalProtocolError;
use presage_store_sqlite::SqliteStoreError;

fn sqlx_error_is_transient(db_error: &sqlx::Error) -> bool {
    match db_error {
        sqlx::Error::PoolTimedOut => true,
        sqlx::Error::Database(err) => {
            if let Some(code) = err.code() {
                // Extended result codes in SQLite:
                // Primary: 5 (SQLITE_BUSY), 6 (SQLITE_LOCKED)
                // Extended: 261 (BUSY_RECOVERY), 517 (LOCKED_SHAREDCACHE),
                //           773 (BUSY_SNAPSHOT), 1029 (LOCKED_VTAB), 1032 (BUSY_TIMEOUT)
                if code == "5"
                    || code == "6"
                    || code.starts_with("5_")
                    || code.starts_with("6_")
                    || code == "261"
                    || code == "517"
                    || code == "773"
                    || code == "1029"
                    || code == "1032"
                {
                    return true;
                }
            }
            let message = err.message();
            message.contains("pool timed out")
                || message.contains("timed out")
                || message.contains("locked")
                || message.contains("busy")
        }
        sqlx::Error::Io(_) => true,
        _ => {
            let message = db_error.to_string();
            message.contains("pool timed out")
                || message.contains("timed out")
                || message.contains("locked")
                || message.contains("busy")
        }
    }
}

pub(crate) fn signal_protocol_error_is_transient(error: &SignalProtocolError) -> bool {
    match error {
        SignalProtocolError::InvalidState(scope, message) => {
            (*scope == "sqlite" || *scope == "presage sqlite store error")
                && (message.contains("pool timed out")
                    || message.contains("timed out")
                    || message.contains("locked")
                    || message.contains("busy")
                    || message.contains("code: 5")
                    || message.contains("code: 6")
                    || message.contains("code: 1032"))
        }
        _ => false,
    }
}

pub(crate) fn sqlite_store_error_is_transient(error: &SqliteStoreError) -> bool {
    match error {
        SqliteStoreError::Db(db_error) => sqlx_error_is_transient(db_error),
        SqliteStoreError::Io(_) => true,
        SqliteStoreError::Protocol(error) => signal_protocol_error_is_transient(error),
        _ => false,
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
}
