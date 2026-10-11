#![cfg(unix)]

use packcaptain::{
    adapters::NativeAdapters,
    domain::{Command, SkillEntry, SkillId, Snapshot, TargetId},
    engine::{
        ApplyFailure, EffectExecutor, Engine, MutationReceipt, ReceiptKey, RecoveryDirection,
        TargetMutation,
    },
    storage::StateStore,
};
use rusqlite::{Connection, params};
use serde_json::Value;
use std::os::unix::{
    fs::{MetadataExt, symlink},
    process::ExitStatusExt,
};
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    process::{Command as Process, Output, Stdio},
    thread,
    time::{Duration, Instant},
};
use tempfile::TempDir;

struct Fixture {
    _temp: TempDir,
    root: PathBuf,
    home: PathBuf,
    state: PathBuf,
    target: TargetId,
    preset: String,
    path: PathBuf,
}

fn cli(home: &Path, state: &Path, args: &[&str]) -> Output {
    Process::new(env!("CARGO_BIN_EXE_packcaptain"))
        .env("HOME", home)
        .env("XDG_CONFIG_HOME", home.join("config"))
        .env("XDG_STATE_HOME", home.join("xdg-state"))
        .args([
            "--agent-home",
            home.to_str().unwrap(),
            "--state-dir",
            state.to_str().unwrap(),
            "--json",
        ])
        .args(args)
        .output()
        .unwrap()
}
fn ok(output: Output) -> Value {
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stdout)
    );
    serde_json::from_slice::<Value>(&output.stdout).unwrap()["data"].clone()
}
impl Fixture {
    fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let root = fs::canonicalize(temp.path()).unwrap();
        let home = root.join("home");
        let state = root.join("state");
        let project = root.join("project");
        fs::create_dir(&project).unwrap();
        let source = home.join(".claude/skills/a");
        fs::create_dir_all(&source).unwrap();
        fs::write(
            source.join("SKILL.md"),
            "---\nname: a\ndescription: rollback safety\n---\n",
        )
        .unwrap();
        let id = ok(cli(
            &home,
            &state,
            &["projects", "add", project.to_str().unwrap()],
        ));
        let target: TargetId = format!("claude-code:project:{}", id["id"].as_str().unwrap())
            .parse()
            .unwrap();
        let skills = ok(cli(&home, &state, &["skills", "list"]));
        let skill = skills[0]["id"].as_str().unwrap();
        let value = ok(cli(
            &home,
            &state,
            &["presets", "create", "--name", "rollback", "--skill", skill],
        ));
        let preset = value["plan"]["preset_updates"]
            .as_object()
            .unwrap()
            .keys()
            .next()
            .unwrap()
            .clone();
        let path = project.join(format!(".claude/skills/pc-{skill}"));
        Self {
            _temp: temp,
            root,
            home,
            state,
            target,
            preset,
            path,
        }
    }
    fn retry(&self) -> Output {
        cli(
            &self.home,
            &self.state,
            &[
                "presets",
                "enable",
                &self.preset,
                "--target",
                &self.target.to_string(),
            ],
        )
    }
    fn snapshot(&self) -> Snapshot {
        StateStore::load_existing_snapshot(&self.state).unwrap()
    }
    fn database(&self) -> Connection {
        Connection::open(self.state.join("packcaptain.sqlite3")).unwrap()
    }
    fn journal(&self) -> Value {
        let text: String = self
            .database()
            .query_row("SELECT payload FROM operation_journal", [], |row| {
                row.get(0)
            })
            .unwrap();
        serde_json::from_str(&text).unwrap()
    }
    fn receipt(&self, direction: RecoveryDirection) -> MutationReceipt {
        StateStore::open(&self.state)
            .unwrap()
            .load_receipt(&ReceiptKey {
                operation_id: self.journal()["id"].as_str().unwrap().to_owned(),
                direction,
                route_ordinal: 0,
            })
            .unwrap()
            .unwrap()
    }
    fn crash(&self, rollback: bool) {
        fs::write(
            self.root.join("command.json"),
            serde_json::to_vec(&Command::SetPresetEnabled {
                preset_id: self.preset.as_str().into(),
                targets: vec![self.target.clone()],
                enabled: !rollback,
            })
            .unwrap(),
        )
        .unwrap();
        let mut child = Process::new(std::env::current_exe().unwrap())
            .args(["--exact", "rollback_safety_child", "--nocapture"])
            .env("PACKCAPTAIN_ROLLBACK_SAFETY_CHILD", &self.root)
            .env(
                "PACKCAPTAIN_SAFETY_ROLLBACK",
                if rollback { "1" } else { "0" },
            )
            .env("HOME", &self.home)
            .stdout(Stdio::from(
                fs::File::create(self.root.join("stdout")).unwrap(),
            ))
            .stderr(Stdio::from(
                fs::File::create(self.root.join("stderr")).unwrap(),
            ))
            .spawn()
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(10);
        while !self.root.join("gate").exists() {
            if let Some(status) = child.try_wait().unwrap() {
                panic!(
                    "child {status}: {}",
                    fs::read_to_string(self.root.join("stderr")).unwrap()
                );
            }
            if Instant::now() > deadline {
                child.kill().unwrap();
                child.wait().unwrap();
                panic!("native gate timeout");
            }
            thread::sleep(Duration::from_millis(5));
        }
        assert_eq!(
            fs::read_to_string(self.root.join("gate")).unwrap(),
            "native completed"
        );
        child.kill().unwrap();
        assert_eq!(child.wait().unwrap().signal(), Some(9));
        assert_eq!(
            self.journal()["status"],
            if rollback { "RollingBack" } else { "Applying" }
        );
        if rollback {
            let journal = self.journal();
            let route = journal["entries_after"]
                .as_object()
                .unwrap()
                .values()
                .next()
                .unwrap();
            assert_eq!(route[0]["entry_fingerprint"], "missing");
            assert_eq!(route[0]["present"], false);
        }
    }
    fn rollback_crash(&self) -> Snapshot {
        ok(self.retry());
        let before = self.snapshot();
        self.crash(true);
        before
    }
    fn refused(&self, before: &Snapshot, inode: u64) {
        let id = self.journal()["id"].clone();
        let result = self.retry();
        assert!(!result.status.success());
        assert!(String::from_utf8_lossy(&result.stdout).contains("needs recovery"));
        let after = self.snapshot();
        assert_eq!(before.targets, after.targets);
        assert_eq!(before.entries, after.entries);
        assert_eq!(fs::symlink_metadata(&self.path).unwrap().ino(), inode);
        assert_eq!(self.journal()["id"], id);
        assert_eq!(self.journal()["status"], "NeedsRecovery");
    }
}
struct GatedNative {
    rollback: bool,
    calls: usize,
}
impl EffectExecutor for GatedNative {
    fn prepare_target(
        &self,
        m: TargetMutation<'_>,
        s: &Snapshot,
        h: &Path,
    ) -> Result<BTreeMap<SkillId, Vec<SkillEntry>>, String> {
        NativeAdapters.prepare_target(m, s, h)
    }
    fn apply_target(
        &mut self,
        m: TargetMutation<'_>,
        s: &Snapshot,
        h: &Path,
    ) -> Result<BTreeMap<SkillId, Vec<SkillEntry>>, ApplyFailure> {
        self.calls += 1;
        let actual = NativeAdapters.apply_target(m, s, h)?;
        if self.rollback && self.calls == 1 {
            return Err(ApplyFailure {
                message: "failure after real disable".into(),
                actual_entries: Some(actual),
            });
        }
        let root = h.parent().unwrap();
        fs::write(root.join("gate-ready"), "native completed").unwrap();
        fs::rename(root.join("gate-ready"), root.join("gate")).unwrap();
        loop {
            thread::sleep(Duration::from_millis(20));
        }
    }
}
#[test]
fn rollback_safety_child() {
    let Ok(root) = std::env::var("PACKCAPTAIN_ROLLBACK_SAFETY_CHILD") else {
        return;
    };
    let root = PathBuf::from(root);
    let home = root.join("home");
    let state = root.join("state");
    let command: Command =
        serde_json::from_slice(&fs::read(root.join("command.json")).unwrap()).unwrap();
    let snapshot = StateStore::load_existing_snapshot(&state).unwrap();
    let discovery = NativeAdapters.discover(&home, &snapshot);
    Engine::new(
        home,
        state,
        GatedNative {
            rollback: std::env::var("PACKCAPTAIN_SAFETY_ROLLBACK").unwrap() == "1",
            calls: 0,
        },
    )
    .execute_with_discovery(&command, &discovery)
    .unwrap();
}
#[test]
fn rollback_same_source_replacement_is_not_owned() {
    let f = Fixture::new();
    let before = f.rollback_crash();
    let receipt = f.receipt(RecoveryDirection::Rollback);
    fs::rename(&f.path, f.root.join("original-link")).unwrap();
    symlink(&receipt.identity.source, &f.path).unwrap();
    let inode = fs::symlink_metadata(&f.path).unwrap().ino();
    assert_ne!(inode, receipt.identity.no_follow.inode);
    f.refused(&before, inode);
    assert_eq!(fs::read_link(&f.path).unwrap(), receipt.identity.source);
    assert!(receipt.identity.anchor.symlink_metadata().is_ok());
}
#[test]
fn rollback_foreign_directory_is_not_touched() {
    let f = Fixture::new();
    let before = f.rollback_crash();
    let receipt = f.receipt(RecoveryDirection::Rollback);
    fs::rename(&f.path, f.root.join("original-link")).unwrap();
    fs::create_dir(&f.path).unwrap();
    fs::write(f.path.join("foreign"), "external").unwrap();
    f.refused(&before, fs::symlink_metadata(&f.path).unwrap().ino());
    assert_eq!(fs::read(f.path.join("foreign")).unwrap(), b"external");
    assert!(receipt.identity.anchor.symlink_metadata().is_ok());
}
#[test]
fn rollback_replaced_anchor_is_not_proof() {
    let f = Fixture::new();
    let before = f.rollback_crash();
    let receipt = f.receipt(RecoveryDirection::Rollback);
    fs::rename(&receipt.identity.anchor, f.root.join("original-anchor")).unwrap();
    symlink(&receipt.identity.source, &receipt.identity.anchor).unwrap();
    let inode = fs::symlink_metadata(&receipt.identity.anchor)
        .unwrap()
        .ino();
    assert_ne!(inode, receipt.identity.no_follow.inode);
    f.refused(&before, fs::symlink_metadata(&f.path).unwrap().ino());
    assert_eq!(
        fs::symlink_metadata(&receipt.identity.anchor)
            .unwrap()
            .ino(),
        inode
    );
}
#[test]
fn direction_receipts_cannot_authorize_each_other() {
    for rollback in [true, false] {
        let f = Fixture::new();
        let before = if rollback {
            f.rollback_crash()
        } else {
            let before = f.snapshot();
            f.crash(false);
            before
        };
        let direction = if rollback {
            RecoveryDirection::Rollback
        } else {
            RecoveryDirection::Forward
        };
        let receipt = f.receipt(direction);
        f.database()
            .execute(
                "UPDATE recovery_receipts SET direction = ?1",
                [if rollback { "forward" } else { "rollback" }],
            )
            .unwrap();
        f.refused(&before, fs::symlink_metadata(&f.path).unwrap().ino());
        assert!(receipt.identity.anchor.symlink_metadata().is_ok());
    }
}
#[test]
fn rollback_verified_pin_is_reused_by_fresh_cli_and_retry() {
    let f = Fixture::new();
    let before = f.rollback_crash();
    let receipt = f.receipt(RecoveryDirection::Rollback);
    let inode = fs::symlink_metadata(&f.path).unwrap().ino();
    assert_eq!(inode, receipt.identity.no_follow.inode);
    ok(f.retry());
    assert_eq!(fs::symlink_metadata(&f.path).unwrap().ino(), inode);
    assert_eq!(f.snapshot().targets, before.targets);
    assert!(fs::symlink_metadata(&receipt.identity.anchor).is_err());
    let stored = f.snapshot();
    let observed = NativeAdapters.discover(&f.home, &stored);
    for (skill, routes) in &stored.entries[&f.target] {
        assert_eq!(
            routes[0].entry_fingerprint,
            observed.entries[&f.target][skill][0].entry_fingerprint
        );
    }
    ok(f.retry());
    assert_eq!(stored.entries, f.snapshot().entries);
    assert_eq!(fs::symlink_metadata(&f.path).unwrap().ino(), inode);
}
#[test]
fn rollback_without_receipt_keeps_recreated_link_unowned() {
    let f = Fixture::new();
    let before = f.rollback_crash();
    let receipt = f.receipt(RecoveryDirection::Rollback);
    f.database()
        .execute("DELETE FROM recovery_receipts", [])
        .unwrap();
    f.refused(&before, fs::symlink_metadata(&f.path).unwrap().ino());
    assert!(receipt.identity.anchor.symlink_metadata().is_ok());
}
#[test]
fn rollback_prepared_pin_commits_new_fingerprint_and_cleans_both_directions() {
    let f = Fixture::new();
    f.rollback_crash();
    let receipt = f.receipt(RecoveryDirection::Rollback);
    let database = f.database();
    let forward_directory = receipt
        .identity
        .anchor
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join(format!(".packcaptain-{}-0", receipt.key.operation_id));
    fs::create_dir(&forward_directory).unwrap();
    let mut forward = receipt.clone();
    forward.key.direction = RecoveryDirection::Forward;
    forward.identity.anchor = forward_directory.join("anchor");
    fs::hard_link(&receipt.identity.anchor, &forward.identity.anchor).unwrap();
    database.execute("INSERT INTO recovery_receipts (operation_id, direction, route_ordinal, identity, stage) VALUES (?1, 'forward', 0, ?2, 1)",
        params![forward.key.operation_id, serde_json::to_string(&forward.identity).unwrap()]).unwrap();
    fs::remove_file(&f.path).unwrap();
    database
        .execute(
            "UPDATE recovery_receipts SET stage = 0 WHERE direction = 'rollback'",
            [],
        )
        .unwrap();
    let foreign = receipt.identity.anchor.parent().unwrap().join("foreign");
    fs::write(&foreign, "external").unwrap();
    ok(f.retry());
    assert_eq!(
        fs::symlink_metadata(&f.path).unwrap().ino(),
        receipt.identity.no_follow.inode
    );
    assert!(fs::symlink_metadata(&receipt.identity.anchor).is_err());
    assert!(fs::symlink_metadata(&forward.identity.anchor).is_err());
    assert_eq!(fs::read(&foreign).unwrap(), b"external");
    let count: usize = database
        .query_row("SELECT count(*) FROM operation_journal", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(count, 0);
    let count: usize = database
        .query_row("SELECT count(*) FROM recovery_receipts", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(count, 0);
    let stored = f.snapshot();
    let observed = NativeAdapters.discover(&f.home, &stored);
    for (skill, routes) in &stored.entries[&f.target] {
        assert_ne!(routes[0].entry_fingerprint, "missing");
        assert_eq!(
            routes[0].entry_fingerprint,
            observed.entries[&f.target][skill][0].entry_fingerprint
        );
    }
    ok(f.retry());
    assert_eq!(stored.entries, f.snapshot().entries);
}
