use std::collections::{BTreeMap, BTreeSet};

use packcaptain::domain::{SkillId, Snapshot, TargetId, TargetState};
use packcaptain::engine::{
    MutationReceipt, ReceiptAction, ReceiptIdentity, ReceiptKey, ReceiptStage, ReceiptWriter,
    RecoveryDirection, SymlinkIdentity,
};
use packcaptain::storage::StateStore;
use rusqlite::{Connection, params};
use serde_json::json;
use tempfile::tempdir;

fn target() -> TargetId {
    "claude-code:global".parse().unwrap()
}

fn insert_legacy_journal(database: &std::path::Path, id: &str) {
    let connection = Connection::open(database).unwrap();
    let payload = json!({
        "id": id, "target": "claude-code:global", "status": "Applying",
        "before": {}, "after": {}, "entries_before": {},
        "entries_after": {"skill-a": [{
            "path": "/fixture/public", "source_path": "/fixture/source",
            "entry_fingerprint": "pending", "present": false, "active": true,
            "ownership": "pack-captain-link"
        }]},
        "planned_target_states": {}, "effects": [], "failure": null
    });
    connection
        .execute(
            "INSERT INTO operation_journal VALUES (?1, ?2)",
            params![id, payload.to_string()],
        )
        .unwrap();
}

fn prepared(id: &str, direction: RecoveryDirection) -> MutationReceipt {
    MutationReceipt {
        key: ReceiptKey {
            operation_id: id.to_owned(),
            direction,
            route_ordinal: 7,
        },
        identity: ReceiptIdentity {
            target: target(),
            skill: SkillId::from("skill-a"),
            path: "/fixture/public".into(),
            source: "/fixture/source".into(),
            anchor: "/fixture/private/anchor".into(),
            no_follow: SymlinkIdentity {
                device: 42,
                inode: 9001,
                readlink: "../source".into(),
            },
            action: ReceiptAction::Create,
        },
        stage: ReceiptStage::Prepared,
    }
}

fn row_count(database: &std::path::Path, table: &str, id: &str) -> i64 {
    Connection::open(database)
        .unwrap()
        .query_row(
            &format!("SELECT count(*) FROM {table} WHERE operation_id = ?1"),
            [id],
            |row| row.get(0),
        )
        .unwrap()
}

#[test]
fn literal_receipts_survive_reopen_and_are_scoped_by_operation_direction_route() {
    let temporary = tempdir().unwrap();
    let store = StateStore::open(temporary.path()).unwrap();
    insert_legacy_journal(store.database_path(), "operation-a");
    insert_legacy_journal(store.database_path(), "operation-b");
    let forward = prepared("operation-a", RecoveryDirection::Forward);
    let mut rollback = prepared("operation-a", RecoveryDirection::Rollback);
    rollback.identity.no_follow.inode = 9002;
    rollback.identity.action = ReceiptAction::Delete {
        owned_fingerprint: "owned-old".to_owned(),
    };
    store.save_receipt(&forward).unwrap();
    store.save_receipt(&rollback).unwrap();
    drop(store);

    let reopened = StateStore::open(temporary.path()).unwrap();
    let actual = reopened.load_receipt(&forward.key).unwrap().unwrap();
    assert_eq!(
        serde_json::to_value(actual).unwrap(),
        json!({
            "key": {"operation_id": "operation-a", "direction": "forward", "route_ordinal": 7},
            "identity": {
                "target": "claude-code:global", "skill": "skill-a", "path": "/fixture/public",
                "source": "/fixture/source", "anchor": "/fixture/private/anchor",
                "no_follow": {"device": 42, "inode": 9001, "readlink": "../source"},
                "action": {"kind": "create"}
            },
            "stage": "prepared"
        })
    );
    assert_eq!(
        serde_json::to_value(reopened.load_receipt(&rollback.key).unwrap().unwrap()).unwrap(),
        json!({
            "key": {"operation_id": "operation-a", "direction": "rollback", "route_ordinal": 7},
            "identity": {
                "target": "claude-code:global", "skill": "skill-a", "path": "/fixture/public",
                "source": "/fixture/source", "anchor": "/fixture/private/anchor",
                "no_follow": {"device": 42, "inode": 9002, "readlink": "../source"},
                "action": {"kind": "delete", "owned_fingerprint": "owned-old"}
            },
            "stage": "prepared"
        })
    );
    for key in [
        ReceiptKey {
            operation_id: "operation-b".to_owned(),
            ..forward.key.clone()
        },
        ReceiptKey {
            route_ordinal: 8,
            ..forward.key.clone()
        },
        ReceiptKey {
            operation_id: "missing".to_owned(),
            ..forward.key.clone()
        },
    ] {
        assert_eq!(reopened.load_receipt(&key).unwrap(), None);
    }
}

#[test]
fn writes_are_idempotent_identity_is_immutable_and_stage_is_monotonic() {
    let temporary = tempdir().unwrap();
    let store = StateStore::open(temporary.path()).unwrap();
    insert_legacy_journal(store.database_path(), "operation-a");
    let receipt = prepared("operation-a", RecoveryDirection::Forward);
    let mut completed = receipt.clone();
    completed.stage = ReceiptStage::VerifiedApplied;
    assert!(store.save_receipt(&completed).is_err());
    store.save_receipt(&receipt).unwrap();
    store.save_receipt(&receipt).unwrap();
    let identity_changes: Vec<fn(&mut ReceiptIdentity)> = vec![
        |identity| identity.target = "pi:global".parse().unwrap(),
        |identity| identity.skill = "other-skill".into(),
        |identity| identity.path = "/other/path".into(),
        |identity| identity.source = "/other/source".into(),
        |identity| identity.anchor = "/other/anchor".into(),
        |identity| identity.no_follow.device += 1,
        |identity| identity.no_follow.inode += 1,
        |identity| identity.no_follow.readlink = "../other".into(),
        |identity| {
            identity.action = ReceiptAction::Delete {
                owned_fingerprint: "changed".to_owned(),
            }
        },
    ];
    for change in identity_changes {
        let mut rebound = receipt.clone();
        change(&mut rebound.identity);
        assert!(store.save_receipt(&rebound).is_err());
    }
    store.save_receipt(&completed).unwrap();
    store.save_receipt(&completed).unwrap();
    assert!(store.save_receipt(&receipt).is_err());
    let mut missing = receipt.clone();
    missing.key.operation_id = "missing".to_owned();
    assert!(store.save_receipt(&missing).is_err());
    assert_eq!(
        row_count(store.database_path(), "recovery_receipts", "operation-a"),
        1
    );
    drop(store);
    let reopened = StateStore::open(temporary.path()).unwrap();
    assert_eq!(
        reopened.load_receipt(&receipt.key).unwrap().unwrap().stage,
        ReceiptStage::VerifiedApplied
    );
    assert!(reopened.save_receipt(&receipt).is_err());
}

#[test]
fn additive_table_preserves_old_snapshot_and_pending_journal_payload() {
    let temporary = tempdir().unwrap();
    let database = temporary.path().join("packcaptain.sqlite3");
    let connection = Connection::open(&database).unwrap();
    connection.execute_batch("CREATE TABLE app_state (singleton INTEGER PRIMARY KEY, payload TEXT NOT NULL);
        CREATE TABLE operation_journal (operation_id TEXT PRIMARY KEY, payload TEXT NOT NULL);
        INSERT INTO app_state VALUES (1, '{\"version\":1,\"snapshot\":{\"skills\":{},\"presets\":{},\"targets\":{}}}');").unwrap();
    drop(connection);
    insert_legacy_journal(&database, "legacy");
    let read_journal = || {
        Connection::open(&database)
            .unwrap()
            .query_row(
                "SELECT payload FROM operation_journal WHERE operation_id = 'legacy'",
                [],
                |row| row.get::<_, String>(0),
            )
            .unwrap()
    };
    let original_payload = read_journal();
    assert_eq!(
        StateStore::load_existing_snapshot(temporary.path()).unwrap(),
        Snapshot::default()
    );
    let store = StateStore::open(temporary.path()).unwrap();
    let receipt = prepared("legacy", RecoveryDirection::Forward);
    assert_eq!(store.load_snapshot().unwrap(), Snapshot::default());
    assert_eq!(store.load_receipt(&receipt.key).unwrap(), None);
    store.save_receipt(&receipt).unwrap();
    Connection::open(&database)
        .unwrap()
        .execute(
            "UPDATE operation_journal SET payload = ?1 WHERE operation_id = 'legacy'",
            [&original_payload],
        )
        .unwrap();
    drop(store);
    let reopened = StateStore::open(temporary.path()).unwrap();
    assert_eq!(reopened.load_snapshot().unwrap(), Snapshot::default());
    assert_eq!(read_journal(), original_payload);
    assert_eq!(reopened.load_receipt(&receipt.key).unwrap(), Some(receipt));
}

#[test]
fn finish_commits_target_journal_and_receipts_atomically_and_rolls_back_on_failure() {
    let temporary = tempdir().unwrap();
    let mut store = StateStore::open(temporary.path()).unwrap();
    let original = Snapshot::default();
    store.save_snapshot(&original).unwrap();
    insert_legacy_journal(store.database_path(), "operation-a");
    insert_legacy_journal(store.database_path(), "operation-b");
    let forward = prepared("operation-a", RecoveryDirection::Forward);
    let rollback = prepared("operation-a", RecoveryDirection::Rollback);
    let unrelated = prepared("operation-b", RecoveryDirection::Forward);
    for receipt in [&forward, &rollback, &unrelated] {
        store.save_receipt(receipt).unwrap();
    }
    let database = store.database_path().to_owned();
    let connection = Connection::open(&database).unwrap();
    connection.execute_batch("CREATE TRIGGER fail_finish BEFORE DELETE ON operation_journal
        WHEN OLD.operation_id = 'operation-a' BEGIN SELECT RAISE(ABORT, 'injected finish failure'); END;").unwrap();
    drop(connection);
    let state = TargetState {
        enabled_skills: BTreeSet::from(["skill-a".into()]),
        ..TargetState::default()
    };
    let entries = BTreeMap::from([(SkillId::from("skill-a"), Vec::new())]);
    assert!(
        store
            .commit_target_and_finish_journal(&target(), &state, &entries, "operation-a")
            .is_err()
    );
    drop(store);
    let mut reopened = StateStore::open(temporary.path()).unwrap();
    assert_eq!(reopened.load_snapshot().unwrap(), original);
    assert_eq!(row_count(&database, "operation_journal", "operation-a"), 1);
    assert_eq!(row_count(&database, "recovery_receipts", "operation-a"), 2);
    assert_eq!(
        reopened.load_receipt(&forward.key).unwrap(),
        Some(forward.clone())
    );
    assert_eq!(
        reopened.load_receipt(&rollback.key).unwrap(),
        Some(rollback)
    );
    assert!(
        reopened
            .commit_target_and_finish_journal(
                &"pi:global".parse().unwrap(),
                &state,
                &entries,
                "operation-a"
            )
            .is_err()
    );
    assert!(
        reopened
            .commit_target_and_finish_journal(&target(), &state, &entries, "missing")
            .is_err()
    );
    Connection::open(&database)
        .unwrap()
        .execute_batch("DROP TRIGGER fail_finish;")
        .unwrap();
    reopened
        .commit_target_and_finish_journal(&target(), &state, &entries, "operation-a")
        .unwrap();
    drop(reopened);
    let reopened = StateStore::open(temporary.path()).unwrap();
    let expected = Snapshot {
        targets: BTreeMap::from([(target(), state)]),
        entries: BTreeMap::from([(target(), entries)]),
        ..Snapshot::default()
    };
    assert_eq!(reopened.load_snapshot().unwrap(), expected);
    assert_eq!(row_count(&database, "operation_journal", "operation-a"), 0);
    assert_eq!(row_count(&database, "recovery_receipts", "operation-a"), 0);
    assert_eq!(reopened.load_receipt(&forward.key).unwrap(), None);
    assert!(reopened.save_receipt(&forward).is_err());
    assert_eq!(row_count(&database, "operation_journal", "operation-b"), 1);
    assert_eq!(
        reopened.load_receipt(&unrelated.key).unwrap(),
        Some(unrelated)
    );
}
