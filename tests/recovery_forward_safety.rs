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
use rusqlite::Connection;
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
            "---\nname: a\ndescription: safety\n---\n",
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
            &["presets", "create", "--name", "safe", "--skill", skill],
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
    fn journal(&self) -> Value {
        let text: String = Connection::open(self.state.join("packcaptain.sqlite3"))
            .unwrap()
            .query_row("SELECT payload FROM operation_journal", [], |row| {
                row.get(0)
            })
            .unwrap();
        serde_json::from_str(&text).unwrap()
    }
    fn receipt(&self) -> MutationReceipt {
        let id = self.journal()["id"].as_str().unwrap().to_owned();
        StateStore::open(&self.state)
            .unwrap()
            .load_receipt(&ReceiptKey {
                operation_id: id,
                direction: RecoveryDirection::Forward,
                route_ordinal: 0,
            })
            .unwrap()
            .unwrap()
    }
    fn crash(&self) {
        fs::write(
            self.root.join("command.json"),
            serde_json::to_vec(&Command::SetPresetEnabled {
                preset_id: self.preset.as_str().into(),
                targets: vec![self.target.clone()],
                enabled: true,
            })
            .unwrap(),
        )
        .unwrap();
        let mut child = Process::new(std::env::current_exe().unwrap())
            .args(["--exact", "forward_safety_child", "--nocapture"])
            .env("PACKCAPTAIN_FORWARD_SAFETY_CHILD", &self.root)
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
                panic!("gate timed out");
            }
            thread::sleep(Duration::from_millis(10));
        }
        assert_eq!(
            fs::read_to_string(self.root.join("gate")).unwrap(),
            "native completed"
        );
        child.kill().unwrap();
        assert_eq!(child.wait().unwrap().signal(), Some(9));
        assert_eq!(self.journal()["status"], "Applying");
        assert_eq!(self.receipt().key.route_ordinal, 0);
    }
    fn refused(&self, before: &Snapshot, inode: u64) {
        let result = self.retry();
        assert!(!result.status.success());
        assert!(String::from_utf8_lossy(&result.stdout).contains("needs recovery"));
        let after = self.snapshot();
        assert_eq!(before.targets, after.targets);
        assert_eq!(before.entries, after.entries);
        assert_eq!(fs::symlink_metadata(&self.path).unwrap().ino(), inode);
        assert_eq!(self.journal()["status"], "NeedsRecovery");
    }
}

struct GatedNative;
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
        let _actual = NativeAdapters.apply_target(m, s, h)?;
        let root = h.parent().unwrap();
        fs::write(root.join("gate-ready"), "native completed").unwrap();
        fs::rename(root.join("gate-ready"), root.join("gate")).unwrap();
        loop {
            thread::sleep(Duration::from_millis(20));
        }
    }
}
#[test]
fn forward_safety_child() {
    let Ok(root) = std::env::var("PACKCAPTAIN_FORWARD_SAFETY_CHILD") else {
        return;
    };
    let root = PathBuf::from(root);
    let home = root.join("home");
    let state = root.join("state");
    let command: Command =
        serde_json::from_slice(&fs::read(root.join("command.json")).unwrap()).unwrap();
    let snapshot = StateStore::load_existing_snapshot(&state).unwrap();
    let discovery = NativeAdapters.discover(&home, &snapshot);
    Engine::new(home, state, GatedNative)
        .execute_with_discovery(&command, &discovery)
        .unwrap();
}

#[test]
fn crash_same_source_replacement_is_not_owned() {
    let f = Fixture::new();
    let before = f.snapshot();
    f.crash();
    let receipt = f.receipt();
    fs::rename(&f.path, f.root.join("original-link")).unwrap();
    symlink(&receipt.identity.source, &f.path).unwrap();
    let inode = fs::symlink_metadata(&f.path).unwrap().ino();
    assert_ne!(inode, receipt.identity.no_follow.inode);
    f.refused(&before, inode);
    assert_eq!(fs::read_link(&f.path).unwrap(), receipt.identity.source);
    assert!(receipt.identity.anchor.symlink_metadata().is_ok());
}
#[test]
fn replaced_anchor_is_not_proof() {
    let f = Fixture::new();
    let before = f.snapshot();
    f.crash();
    let receipt = f.receipt();
    fs::rename(&receipt.identity.anchor, f.root.join("original-anchor")).unwrap();
    symlink(&receipt.identity.source, &receipt.identity.anchor).unwrap();
    f.refused(&before, fs::symlink_metadata(&f.path).unwrap().ino());
    assert!(receipt.identity.anchor.symlink_metadata().is_ok());
}
#[test]
fn missing_receipt_keeps_pending_unowned() {
    let f = Fixture::new();
    let before = f.snapshot();
    f.crash();
    Connection::open(f.state.join("packcaptain.sqlite3"))
        .unwrap()
        .execute("DELETE FROM recovery_receipts", [])
        .unwrap();
    f.refused(&before, fs::symlink_metadata(&f.path).unwrap().ino());
}
#[test]
fn old_pending_without_receipt_is_refused() {
    let f = Fixture::new();
    let before = f.snapshot();
    f.crash();
    let receipt = f.receipt();
    fs::remove_file(&receipt.identity.anchor).unwrap();
    fs::remove_dir(receipt.identity.anchor.parent().unwrap()).unwrap();
    Connection::open(f.state.join("packcaptain.sqlite3"))
        .unwrap()
        .execute("DELETE FROM recovery_receipts", [])
        .unwrap();
    f.refused(&before, fs::symlink_metadata(&f.path).unwrap().ino());
}
#[test]
fn prepared_pin_can_publish_and_commit_cleanup_preserves_unrelated_file() {
    let f = Fixture::new();
    f.crash();
    let receipt = f.receipt();
    fs::remove_file(&f.path).unwrap();
    Connection::open(f.state.join("packcaptain.sqlite3"))
        .unwrap()
        .execute("UPDATE recovery_receipts SET stage = 0", [])
        .unwrap();
    let foreign = receipt.identity.anchor.parent().unwrap().join("not-ours");
    fs::write(&foreign, "external").unwrap();
    ok(f.retry());
    assert_eq!(
        fs::symlink_metadata(&f.path).unwrap().ino(),
        receipt.identity.no_follow.inode
    );
    assert!(fs::symlink_metadata(&receipt.identity.anchor).is_err());
    assert_eq!(fs::read(&foreign).unwrap(), b"external");
    let stored = f.snapshot();
    ok(f.retry());
    assert_eq!(stored.entries, f.snapshot().entries);
}
#[test]
fn receipt_identity_rebinding_is_refused_before_mutation() {
    let f = Fixture::new();
    let before = f.snapshot();
    f.crash();
    let receipt = f.receipt();
    let database = Connection::open(f.state.join("packcaptain.sqlite3")).unwrap();
    for field in ["target", "skill", "path", "source", "anchor", "action"] {
        let mut identity = serde_json::to_value(&receipt.identity).unwrap();
        identity[field] = if field == "action" {
            serde_json::json!({"kind":"delete", "owned_fingerprint":"wrong"})
        } else {
            Value::String("wrong".into())
        };
        database
            .execute(
                "UPDATE recovery_receipts SET identity = ?1",
                [serde_json::to_string(&identity).unwrap()],
            )
            .unwrap();
        let result = f.retry();
        assert!(!result.status.success(), "accepted field {field}");
        assert_eq!(f.snapshot().targets, before.targets);
        assert_eq!(f.snapshot().entries, before.entries);
        assert_eq!(
            fs::symlink_metadata(&f.path).unwrap().ino(),
            receipt.identity.no_follow.inode
        );
        let mut journal = f.journal();
        journal["status"] = Value::String("Applying".into());
        journal["failure"] = Value::Null;
        database
            .execute(
                "UPDATE operation_journal SET payload = ?1",
                [serde_json::to_string(&journal).unwrap()],
            )
            .unwrap();
    }
}
