//! SQLite checkpoint store.

use std::{
    path::Path,
    sync::{Arc, Mutex},
};

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use rusqlite::{Connection, OptionalExtension, params};
use wyse_core::{RunId, TurnId};

use crate::{CheckpointError, CheckpointKind, CheckpointRecord, CheckpointStatus, CheckpointStore};

/// SQLite-backed latest checkpoint store.
#[derive(Clone)]
#[non_exhaustive]
pub struct SqliteCheckpointStore {
    connection: Arc<Mutex<Connection>>,
}

impl SqliteCheckpointStore {
    /// Opens a SQLite checkpoint store at `path`.
    ///
    /// # Errors
    ///
    /// Returns an error when SQLite cannot open or initialize the database.
    pub fn open(path: impl AsRef<Path>) -> Result<Self, CheckpointError> {
        let connection = Connection::open(path).map_err(CheckpointError::Sqlite)?;
        init(&connection)?;
        Ok(Self {
            connection: Arc::new(Mutex::new(connection)),
        })
    }

    /// Opens an in-memory SQLite checkpoint store.
    ///
    /// # Errors
    ///
    /// Returns an error when SQLite cannot initialize the database.
    pub fn open_in_memory() -> Result<Self, CheckpointError> {
        let connection = Connection::open_in_memory().map_err(CheckpointError::Sqlite)?;
        init(&connection)?;
        Ok(Self {
            connection: Arc::new(Mutex::new(connection)),
        })
    }
}

#[async_trait]
impl CheckpointStore for SqliteCheckpointStore {
    async fn put_latest(&self, record: CheckpointRecord) -> Result<(), CheckpointError> {
        let last_seq =
            i64::try_from(record.last_seq).map_err(|_| CheckpointError::InvalidSequence {
                value: record.last_seq,
            })?;
        let connection = Arc::clone(&self.connection);
        tokio::task::spawn_blocking(move || {
            let connection = connection
                .lock()
                .expect("sqlite checkpoint store mutex should not be poisoned");
            connection
                .execute(
                    r#"
                    INSERT INTO checkpoints (
                        run_id,
                        turn_id,
                        checkpoint_id,
                        kind,
                        status,
                        state_version,
                        state,
                        last_seq,
                        updated_at
                    )
                    VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
                    ON CONFLICT(run_id, turn_id, kind) DO UPDATE SET
                        checkpoint_id = excluded.checkpoint_id,
                        status = excluded.status,
                        state_version = excluded.state_version,
                        state = excluded.state,
                        last_seq = excluded.last_seq,
                        updated_at = excluded.updated_at
                    "#,
                    params![
                        record.run_id.to_string(),
                        record.turn_id.to_string(),
                        record.checkpoint_id.to_string(),
                        record.kind.as_str(),
                        record.status.as_str(),
                        i64::from(record.state_version),
                        record.state,
                        last_seq,
                        record.updated_at.to_rfc3339(),
                    ],
                )
                .map(|_| ())
                .map_err(CheckpointError::from)
        })
        .await
        .map_err(|source| CheckpointError::BlockingTask { source })?
    }

    async fn latest_turn(
        &self,
        run_id: RunId,
        turn_id: TurnId,
        kind: CheckpointKind,
    ) -> Result<Option<CheckpointRecord>, CheckpointError> {
        let connection = Arc::clone(&self.connection);
        tokio::task::spawn_blocking(move || {
            let connection = connection
                .lock()
                .expect("sqlite checkpoint store mutex should not be poisoned");
            connection
                .query_row(
                    r#"
                    SELECT
                        run_id,
                        turn_id,
                        checkpoint_id,
                        kind,
                        status,
                        state_version,
                        state,
                        last_seq,
                        updated_at
                    FROM checkpoints
                    WHERE run_id = ?1 AND turn_id = ?2 AND kind = ?3
                    "#,
                    params![run_id.to_string(), turn_id.to_string(), kind.as_str()],
                    row_to_record,
                )
                .optional()
                .map_err(CheckpointError::from)
        })
        .await
        .map_err(|source| CheckpointError::BlockingTask { source })?
    }
}

fn init(connection: &Connection) -> Result<(), CheckpointError> {
    connection.execute_batch(
        r#"
        CREATE TABLE IF NOT EXISTS checkpoints (
            run_id TEXT NOT NULL,
            turn_id TEXT NOT NULL,
            checkpoint_id TEXT NOT NULL,
            kind TEXT NOT NULL,
            status TEXT NOT NULL,
            state_version INTEGER NOT NULL,
            state BLOB NOT NULL,
            last_seq INTEGER NOT NULL,
            updated_at TEXT NOT NULL,
            PRIMARY KEY (run_id, turn_id, kind)
        );
        "#,
    )?;
    Ok(())
}

fn row_to_record(row: &rusqlite::Row<'_>) -> Result<CheckpointRecord, rusqlite::Error> {
    let run_id: String = row.get(0)?;
    let turn_id: String = row.get(1)?;
    let checkpoint_id: String = row.get(2)?;
    let kind: String = row.get(3)?;
    let status: String = row.get(4)?;
    let state_version: u32 = row.get(5)?;
    let state: Vec<u8> = row.get(6)?;
    let last_seq: i64 = row.get(7)?;
    let updated_at: String = row.get(8)?;

    Ok(CheckpointRecord {
        run_id: run_id.parse().map_err(|err| {
            rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, Box::new(err))
        })?,
        turn_id: turn_id.parse().map_err(|err| {
            rusqlite::Error::FromSqlConversionFailure(1, rusqlite::types::Type::Text, Box::new(err))
        })?,
        checkpoint_id: checkpoint_id.parse().map_err(|err| {
            rusqlite::Error::FromSqlConversionFailure(2, rusqlite::types::Type::Text, Box::new(err))
        })?,
        kind: CheckpointKind::from_db(&kind).map_err(|err| {
            rusqlite::Error::FromSqlConversionFailure(3, rusqlite::types::Type::Text, Box::new(err))
        })?,
        status: CheckpointStatus::from_db(&status).map_err(|err| {
            rusqlite::Error::FromSqlConversionFailure(4, rusqlite::types::Type::Text, Box::new(err))
        })?,
        state_version,
        state,
        last_seq: u64::try_from(last_seq).map_err(|err| {
            rusqlite::Error::FromSqlConversionFailure(
                7,
                rusqlite::types::Type::Integer,
                Box::new(err),
            )
        })?,
        updated_at: DateTime::parse_from_rfc3339(&updated_at)
            .map_err(|err| {
                rusqlite::Error::FromSqlConversionFailure(
                    8,
                    rusqlite::types::Type::Text,
                    Box::new(err),
                )
            })?
            .with_timezone(&Utc),
    })
}

#[cfg(test)]
mod tests {
    use chrono::Utc;
    use wyse_core::{RunId, TurnId};

    use super::*;
    use crate::{CheckpointId, CheckpointStatus};

    fn record(
        run_id: RunId,
        turn_id: TurnId,
        status: CheckpointStatus,
        state: &[u8],
        last_seq: u64,
    ) -> CheckpointRecord {
        CheckpointRecord {
            run_id,
            turn_id,
            checkpoint_id: CheckpointId::new(),
            kind: CheckpointKind::Agent,
            status,
            state_version: 1,
            state: state.to_vec(),
            last_seq,
            updated_at: Utc::now(),
        }
    }

    #[tokio::test]
    async fn sqlite_store_upserts_latest_turn_checkpoint() {
        let store = SqliteCheckpointStore::open_in_memory().expect("store opens");
        let run_id = RunId::new();
        let turn_id = TurnId::new();
        let first = record(run_id, turn_id, CheckpointStatus::Running, br#"{"n":1}"#, 1);
        let second = record(
            run_id,
            turn_id,
            CheckpointStatus::WaitingRetry,
            br#"{"n":2}"#,
            2,
        );

        store.put_latest(first).await.expect("first put");
        store.put_latest(second.clone()).await.expect("second put");

        let loaded = store
            .latest_turn(run_id, turn_id, CheckpointKind::Agent)
            .await
            .expect("latest loads")
            .expect("record exists");

        assert_eq!(loaded.checkpoint_id, second.checkpoint_id);
        assert_eq!(loaded.status, CheckpointStatus::WaitingRetry);
        assert_eq!(loaded.state, br#"{"n":2}"#);
        assert_eq!(loaded.last_seq, 2);
    }

    #[tokio::test]
    async fn sqlite_store_returns_none_for_missing_turn() {
        let store = SqliteCheckpointStore::open_in_memory().expect("store opens");

        let loaded = store
            .latest_turn(RunId::new(), TurnId::new(), CheckpointKind::Agent)
            .await
            .expect("latest query works");

        assert!(loaded.is_none());
    }
}
