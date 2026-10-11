use std::collections::BTreeMap;
use std::error::Error;
use std::fmt;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::Duration;

use rusqlite::{Connection, OpenFlags, OptionalExtension, TransactionBehavior, params};
use serde::{Deserialize, Serialize};

use crate::domain::{Effect, SkillEntry, SkillId, Snapshot, TargetId, TargetState};
use crate::engine::{
    MutationReceipt, ReceiptIdentity, ReceiptKey, ReceiptReader, ReceiptStage, ReceiptWriter,
    RecoveryDirection,
};

impl ReceiptReader for StateStore {
    fn load_receipt(&self, key: &ReceiptKey) -> Result<Option<MutationReceipt>, StorageError> {
        StateStore::load_receipt(self, key)
    }
}

const STATE_SCHEMA_VERSION: u32 = 1;

pub struct StateStore {
    connection: Connection,
    database_path: PathBuf,
}

#[derive(Debug)]
pub enum StorageError {
    Io(io::Error),
    Sqlite(rusqlite::Error),
    Json(serde_json::Error),
    SchemaVersion(u32),
    Receipt(String),
}

impl fmt::Display for StorageError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => write!(formatter, "storage I/O failed: {error}"),
            Self::Sqlite(error) => write!(formatter, "storage database failed: {error}"),
            Self::Json(error) => write!(formatter, "storage state encoding failed: {error}"),
            Self::Receipt(message) => write!(formatter, "invalid recovery receipt: {message}"),
            Self::SchemaVersion(version) => {
                write!(formatter, "unsupported state schema version {version}")
            }
        }
    }
}

impl Error for StorageError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Io(error) => Some(error),
            Self::Sqlite(error) => Some(error),
            Self::Json(error) => Some(error),
            Self::SchemaVersion(_) | Self::Receipt(_) => None,
        }
    }
}

impl From<io::Error> for StorageError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

impl From<rusqlite::Error> for StorageError {
    fn from(error: rusqlite::Error) -> Self {
        Self::Sqlite(error)
    }
}

impl From<serde_json::Error> for StorageError {
    fn from(error: serde_json::Error) -> Self {
        Self::Json(error)
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub(crate) enum JournalStatus {
    Applying,
    RollingBack,
    NeedsRecovery,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub(crate) struct TargetJournal {
    pub id: String,
    pub target: TargetId,
    pub status: JournalStatus,
    pub before: TargetState,
    pub after: TargetState,
    pub entries_before: BTreeMap<SkillId, Vec<SkillEntry>>,
    pub entries_after: BTreeMap<SkillId, Vec<SkillEntry>>,
    pub planned_target_states: BTreeMap<TargetId, TargetState>,
    pub effects: Vec<Effect>,
    pub failure: Option<String>,
}

#[derive(Serialize, Deserialize)]
struct StateEnvelope {
    version: u32,
    snapshot: Snapshot,
}

impl StateStore {
    pub fn open(state_dir: &Path) -> Result<Self, StorageError> {
        fs::create_dir_all(state_dir)?;
        let database_path = state_dir.join("packcaptain.sqlite3");
        let connection = Connection::open(&database_path)?;
        connection.busy_timeout(Duration::from_secs(10))?;
        connection.execute_batch(
            "PRAGMA journal_mode = WAL;
             PRAGMA synchronous = FULL;
             CREATE TABLE IF NOT EXISTS app_state (
                 singleton INTEGER PRIMARY KEY CHECK (singleton = 1),
                 payload TEXT NOT NULL
             );
             CREATE TABLE IF NOT EXISTS operation_journal (
                 operation_id TEXT PRIMARY KEY,
                 payload TEXT NOT NULL
             );
             CREATE TABLE IF NOT EXISTS recovery_receipts (
                 operation_id TEXT NOT NULL,
                 direction TEXT NOT NULL CHECK (direction IN ('forward', 'rollback')),
                 route_ordinal INTEGER NOT NULL CHECK (route_ordinal BETWEEN 0 AND 4294967295),
                 identity TEXT NOT NULL,
                 stage INTEGER NOT NULL CHECK (stage IN (0, 1)),
                 PRIMARY KEY (operation_id, direction, route_ordinal)
             );",
        )?;
        Ok(Self {
            connection,
            database_path,
        })
    }

    pub fn database_path(&self) -> &Path {
        &self.database_path
    }

    pub fn load_existing_snapshot(state_dir: &Path) -> Result<Snapshot, StorageError> {
        let Some(connection) = open_existing_read_only(state_dir)? else {
            return Ok(Snapshot::default());
        };
        let payload: Option<String> = connection
            .query_row(
                "SELECT payload FROM app_state WHERE singleton = 1",
                [],
                |row| row.get(0),
            )
            .optional()?;
        match payload {
            Some(value) => decode_snapshot(&value),
            None => Ok(Snapshot::default()),
        }
    }

    pub(crate) fn load_existing_journals(
        state_dir: &Path,
    ) -> Result<Vec<TargetJournal>, StorageError> {
        let Some(connection) = open_existing_read_only(state_dir)? else {
            return Ok(Vec::new());
        };
        let mut statement =
            connection.prepare("SELECT payload FROM operation_journal ORDER BY operation_id")?;
        let rows = statement.query_map([], |row| row.get::<_, String>(0))?;
        rows.map(|row| {
            let payload = row?;
            serde_json::from_str(&payload).map_err(StorageError::from)
        })
        .collect()
    }

    pub fn load_snapshot(&self) -> Result<Snapshot, StorageError> {
        let payload: Option<String> = self
            .connection
            .query_row(
                "SELECT payload FROM app_state WHERE singleton = 1",
                [],
                |row| row.get(0),
            )
            .optional()?;
        payload
            .map(|value| decode_snapshot(&value))
            .transpose()
            .map(Option::unwrap_or_default)
    }

    pub fn save_snapshot(&mut self, snapshot: &Snapshot) -> Result<(), StorageError> {
        let payload = encode_snapshot(snapshot)?;
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        transaction.execute(
            "INSERT INTO app_state (singleton, payload) VALUES (1, ?1)
             ON CONFLICT(singleton) DO UPDATE SET payload = excluded.payload",
            [&payload],
        )?;
        transaction.commit()?;
        Ok(())
    }

    pub(crate) fn insert_journal(&self, journal: &TargetJournal) -> Result<(), StorageError> {
        let payload = serde_json::to_string(journal)?;
        self.connection.execute(
            "INSERT INTO operation_journal (operation_id, payload) VALUES (?1, ?2)",
            params![journal.id, payload],
        )?;
        Ok(())
    }

    pub(crate) fn update_journal(&self, journal: &TargetJournal) -> Result<(), StorageError> {
        let payload = serde_json::to_string(journal)?;
        self.connection.execute(
            "UPDATE operation_journal SET payload = ?2 WHERE operation_id = ?1",
            params![journal.id, payload],
        )?;
        Ok(())
    }

    pub(crate) fn pending_journals(&self) -> Result<Vec<TargetJournal>, StorageError> {
        let mut statement = self
            .connection
            .prepare("SELECT payload FROM operation_journal ORDER BY operation_id")?;
        let rows = statement.query_map([], |row| row.get::<_, String>(0))?;
        rows.map(|row| {
            let payload = row?;
            serde_json::from_str(&payload).map_err(StorageError::from)
        })
        .collect()
    }

    /// Loads only the exact operation, direction and route while its journal exists.
    pub fn load_receipt(&self, key: &ReceiptKey) -> Result<Option<MutationReceipt>, StorageError> {
        let row: Option<(String, i64)> = self
            .connection
            .query_row(
                "SELECT identity, stage FROM recovery_receipts
             WHERE operation_id = ?1 AND direction = ?2 AND route_ordinal = ?3
             AND EXISTS (SELECT 1 FROM operation_journal WHERE operation_id = ?1)",
                params![
                    key.operation_id,
                    direction_key(key.direction),
                    key.route_ordinal
                ],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?;
        row.map(|(identity, stage)| {
            Ok(MutationReceipt {
                key: key.clone(),
                identity: serde_json::from_str(&identity)?,
                stage: match stage {
                    0 => ReceiptStage::Prepared,
                    1 => ReceiptStage::VerifiedApplied,
                    _ => return Err(StorageError::Receipt("unknown stage".to_owned())),
                },
            })
        })
        .transpose()
    }

    /// Commits metadata and removes this operation's journal and receipts atomically.
    /// The operation must have a pending journal for the supplied target.
    pub fn commit_target_and_finish_journal(
        &mut self,
        target: &TargetId,
        state: &TargetState,
        entries: &BTreeMap<SkillId, Vec<SkillEntry>>,
        operation_id: &str,
    ) -> Result<(), StorageError> {
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let journal: Option<String> = transaction
            .query_row(
                "SELECT payload FROM operation_journal WHERE operation_id = ?1",
                [operation_id],
                |row| row.get(0),
            )
            .optional()?;
        let journal: TargetJournal = serde_json::from_str(&journal.ok_or_else(|| {
            StorageError::Receipt("operation has no pending journal".to_owned())
        })?)?;
        if &journal.target != target {
            return Err(StorageError::Receipt("journal target mismatch".to_owned()));
        }
        let payload: Option<String> = transaction
            .query_row(
                "SELECT payload FROM app_state WHERE singleton = 1",
                [],
                |row| row.get(0),
            )
            .optional()?;
        let mut snapshot = payload
            .map(|value| decode_snapshot(&value))
            .transpose()?
            .unwrap_or_default();
        snapshot.targets.insert(target.clone(), state.clone());
        snapshot.entries.insert(target.clone(), entries.clone());
        let payload = encode_snapshot(&snapshot)?;
        transaction.execute(
            "INSERT INTO app_state (singleton, payload) VALUES (1, ?1)
             ON CONFLICT(singleton) DO UPDATE SET payload = excluded.payload",
            [&payload],
        )?;
        transaction.execute(
            "DELETE FROM recovery_receipts WHERE operation_id = ?1",
            [operation_id],
        )?;
        transaction.execute(
            "DELETE FROM operation_journal WHERE operation_id = ?1",
            [operation_id],
        )?;
        transaction.commit()?;
        Ok(())
    }
}

impl ReceiptWriter for StateStore {
    fn save_receipt(&self, receipt: &MutationReceipt) -> Result<(), StorageError> {
        let transaction =
            rusqlite::Transaction::new_unchecked(&self.connection, TransactionBehavior::Immediate)?;
        let journal: Option<String> = transaction
            .query_row(
                "SELECT payload FROM operation_journal WHERE operation_id = ?1",
                [&receipt.key.operation_id],
                |row| row.get(0),
            )
            .optional()?;
        let journal: TargetJournal = serde_json::from_str(&journal.ok_or_else(|| {
            StorageError::Receipt("operation has no pending journal".to_owned())
        })?)?;
        if journal.target != receipt.identity.target {
            return Err(StorageError::Receipt("journal target mismatch".to_owned()));
        }
        let existing: Option<(String, i64)> = transaction
            .query_row(
                "SELECT identity, stage FROM recovery_receipts
             WHERE operation_id = ?1 AND direction = ?2 AND route_ordinal = ?3",
                params![
                    receipt.key.operation_id,
                    direction_key(receipt.key.direction),
                    receipt.key.route_ordinal
                ],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?;
        let stage = match receipt.stage {
            ReceiptStage::Prepared => 0,
            ReceiptStage::VerifiedApplied => 1,
        };
        if let Some((identity, previous_stage)) = existing {
            let identity: ReceiptIdentity = serde_json::from_str(&identity)?;
            if identity != receipt.identity {
                return Err(StorageError::Receipt(
                    "immutable identity cannot be rebound".to_owned(),
                ));
            }
            if stage < previous_stage {
                return Err(StorageError::Receipt("stage cannot regress".to_owned()));
            }
        } else if receipt.stage != ReceiptStage::Prepared {
            return Err(StorageError::Receipt(
                "first stage must be prepared".to_owned(),
            ));
        }
        let identity = serde_json::to_string(&receipt.identity)?;
        transaction.execute(
            "INSERT INTO recovery_receipts (operation_id, direction, route_ordinal, identity, stage)
             VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT(operation_id, direction, route_ordinal) DO UPDATE SET stage = excluded.stage",
            params![receipt.key.operation_id, direction_key(receipt.key.direction), receipt.key.route_ordinal, identity, stage],
        )?;
        transaction.commit()?;
        Ok(())
    }
}

fn direction_key(direction: RecoveryDirection) -> &'static str {
    match direction {
        RecoveryDirection::Forward => "forward",
        RecoveryDirection::Rollback => "rollback",
    }
}

fn open_existing_read_only(state_dir: &Path) -> Result<Option<Connection>, StorageError> {
    let database_path = state_dir.join("packcaptain.sqlite3");
    if !database_path.exists() {
        return Ok(None);
    }
    let connection = Connection::open_with_flags(
        database_path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )?;
    Ok(Some(connection))
}

fn encode_snapshot(snapshot: &Snapshot) -> Result<String, StorageError> {
    serde_json::to_string(&StateEnvelope {
        version: STATE_SCHEMA_VERSION,
        snapshot: snapshot.clone(),
    })
    .map_err(StorageError::from)
}

fn decode_snapshot(payload: &str) -> Result<Snapshot, StorageError> {
    let envelope: StateEnvelope = serde_json::from_str(payload)?;
    if envelope.version != STATE_SCHEMA_VERSION {
        return Err(StorageError::SchemaVersion(envelope.version));
    }
    Ok(envelope.snapshot)
}
