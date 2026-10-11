#![cfg(unix)]

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command as Process, Output, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use packcaptain::adapters::NativeAdapters;
use packcaptain::domain::{Command, Effect, SkillEntry, SkillId, Snapshot, TargetId};
use packcaptain::engine::{
    ApplyFailure, EffectExecutor, Engine, EngineError, TargetMutation, TargetOutcomeStatus,
};
use packcaptain::storage::StateStore;
use rusqlite::{Connection, params};
use serde_json::{Value, json};
use tempfile::TempDir;

struct Fixture {
    _temp: TempDir,
    root: PathBuf,
    home: PathBuf,
    state: PathBuf,
    target: TargetId,
    skills: BTreeMap<String, SkillId>,
    preset: String,
}

impl Fixture {
    fn new() -> Self {
        let temp = tempfile::tempdir_in("/private/tmp").unwrap();
        let root = fs::canonicalize(temp.path()).unwrap();
        let home = root.join("home");
        let state = root.join("state");
        let project = root.join("project");
        fs::create_dir_all(&project).unwrap();
        for name in ["a", "r"] {
            let source = home.join(format!(".claude/skills/{name}"));
            fs::create_dir_all(&source).unwrap();
            fs::write(
                source.join("SKILL.md"),
                format!("---\nname: {name}\ndescription: recovery regression\n---\n"),
            )
            .unwrap();
        }
        let project_record = ok(cli(
            &home,
            &state,
            &["projects", "add", project.to_str().unwrap()],
        ));
        let target: TargetId = format!(
            "claude-code:project:{}",
            project_record["id"].as_str().unwrap()
        )
        .parse()
        .unwrap();
        let inventory = ok(cli(&home, &state, &["skills", "list"]));
        let skills: BTreeMap<_, _> = inventory
            .as_array()
            .unwrap()
            .iter()
            .map(|s| {
                (
                    s["name"].as_str().unwrap().to_owned(),
                    SkillId::from(s["id"].as_str().unwrap()),
                )
            })
            .collect();
        let created = ok(cli(
            &home,
            &state,
            &[
                "presets",
                "create",
                "--name",
                "bundle",
                "--skill",
                skills["a"].as_str(),
                "--skill",
                skills["r"].as_str(),
            ],
        ));
        let preset = created["plan"]["preset_updates"]
            .as_object()
            .unwrap()
            .keys()
            .next()
            .unwrap()
            .clone();
        Self {
            _temp: temp,
            root,
            home,
            state,
            target,
            skills,
            preset,
        }
    }

    fn snapshot(&self) -> Snapshot {
        StateStore::load_existing_snapshot(&self.state).unwrap()
    }

    fn enable(&self) {
        ok(cli(
            &self.home,
            &self.state,
            &[
                "presets",
                "enable",
                &self.preset,
                "--target",
                &self.target.to_string(),
            ],
        ));
    }

    fn path(&self, name: &str) -> PathBuf {
        self.root.join(format!(
            "project/.claude/skills/pc-{}",
            self.skills[name].as_str()
        ))
    }

    fn update(&self, target: &str) {
        ok(cli(
            &self.home,
            &self.state,
            &[
                "presets",
                "update",
                &self.preset,
                "--remove-skill",
                self.skills["r"].as_str(),
                "--sync-target",
                target,
            ],
        ));
    }
}

fn cli(home: &Path, state: &Path, args: &[&str]) -> Output {
    Process::new(env!("CARGO_BIN_EXE_packcaptain"))
        .env("HOME", home)
        .env("XDG_CONFIG_HOME", home.join("xdg-config"))
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
        "exit={:?} stdout={} stderr={}",
        output.status.code(),
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let value: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(value["ok"], true);
    value["data"].clone()
}

#[test]
fn reopening_lagging_preset_removes_old_entry_on_disk() {
    let f = Fixture::new();
    f.enable();
    f.update("claude-code:global");
    assert!(f.path("r").exists());
    f.enable();
    assert!(
        !f.path("r").exists(),
        "removed member survives explicit reopen"
    );
    assert_eq!(
        f.snapshot().targets[&f.target].enabled_skills,
        BTreeSet::from([f.skills["a"].clone()])
    );
}

#[test]
fn missing_removed_member_keeps_history_until_source_returns() {
    let f = Fixture::new();
    f.enable();
    let source = f.home.join(".claude/skills/r");
    fs::rename(&source, f.root.join("temporarily-away")).unwrap();
    f.update(&f.target.to_string());
    fs::rename(f.root.join("temporarily-away"), &source).unwrap();
    f.update(&f.target.to_string());
    assert!(
        !f.path("r").exists(),
        "missing old member lost history and revived"
    );
    assert_eq!(
        f.snapshot().targets[&f.target].enabled_skills,
        BTreeSet::from([f.skills["a"].clone()])
    );
}

#[test]
fn missing_retained_manual_close_survives_real_reopen() {
    let f = Fixture::new();
    f.enable();
    ok(cli(
        &f.home,
        &f.state,
        &[
            "skills",
            "disable",
            f.skills["r"].as_str(),
            "--target",
            &f.target.to_string(),
        ],
    ));
    let source = f.home.join(".claude/skills/r");
    fs::rename(&source, f.root.join("temporarily-away")).unwrap();
    ok(cli(
        &f.home,
        &f.state,
        &[
            "presets",
            "update",
            &f.preset,
            "--sync-target",
            &f.target.to_string(),
        ],
    ));
    fs::rename(f.root.join("temporarily-away"), &source).unwrap();
    let result = cli(
        &f.home,
        &f.state,
        &[
            "presets",
            "update",
            &f.preset,
            "--sync-target",
            &f.target.to_string(),
        ],
    );
    assert!(!result.status.success());
    assert_eq!(
        serde_json::from_slice::<Value>(&result.stdout).unwrap()["error"]["code"],
        "closed-members-policy-required"
    );
    assert!(
        f.snapshot().targets[&f.target]
            .manual_closed
            .contains(&f.skills["r"])
    );
    assert!(!f.path("r").exists());
}

struct InterruptingNative {
    native: NativeAdapters,
    calls: usize,
    mode: String,
    marker: PathBuf,
}

impl EffectExecutor for InterruptingNative {
    fn prepare_target(
        &self,
        m: TargetMutation<'_>,
        s: &Snapshot,
        h: &Path,
    ) -> Result<BTreeMap<SkillId, Vec<SkillEntry>>, String> {
        self.native.prepare_target(m, s, h)
    }
    fn apply_target(
        &mut self,
        m: TargetMutation<'_>,
        s: &Snapshot,
        h: &Path,
    ) -> Result<BTreeMap<SkillId, Vec<SkillEntry>>, ApplyFailure> {
        self.calls += 1;
        if self.mode == "before-forward" {
            fs::write(&self.marker, "journal persisted before native mutation").unwrap();
            loop {
                thread::sleep(Duration::from_millis(20));
            }
        }
        if self.calls == 2 && self.mode == "rollback-return" {
            let journal = journal_payload(&h.parent().unwrap().join("state"));
            assert_eq!(journal["status"], "RollingBack");
            let actual: BTreeMap<SkillId, Vec<SkillEntry>> =
                serde_json::from_value(journal["entries_after"].clone()).unwrap();
            assert_eq!(&actual, m.entries_before);
            for routes in m.entries_before.values() {
                for route in routes {
                    assert!(!route.present && !route.active);
                    assert_eq!(route.entry_fingerprint, "missing");
                    assert!(fs::symlink_metadata(&route.path).is_err());
                }
            }
        }
        let entries = self.native.apply_target(m, s, h)?;
        if self.calls == 1 && self.mode.starts_with("rollback") {
            return Err(ApplyFailure {
                message: "failure after real filesystem mutation".into(),
                actual_entries: (self.mode != "rollback-unproven").then_some(entries),
            });
        }
        if self.mode == "rollback-return" {
            return Ok(entries);
        }
        fs::write(&self.marker, "real native mutation completed").unwrap();
        loop {
            thread::sleep(Duration::from_millis(20));
        }
    }
}

#[test]
fn crash_child() {
    let Ok(root) = std::env::var("PACKCAPTAIN_RECOVERY_CHILD") else {
        return;
    };
    let root = PathBuf::from(root);
    let snapshot = StateStore::load_existing_snapshot(&root.join("state")).unwrap();
    let command: Command =
        serde_json::from_slice(&fs::read(root.join("command.json")).unwrap()).unwrap();
    let executor = InterruptingNative {
        native: NativeAdapters,
        calls: 0,
        mode: std::env::var("PACKCAPTAIN_RECOVERY_MODE").unwrap(),
        marker: root.join("mutation.marker"),
    };
    let mut engine = Engine::new(root.join("home"), root.join("state"), executor);
    let discovery = NativeAdapters.discover(&root.join("home"), &snapshot);
    engine.execute_with_discovery(&command, &discovery).unwrap();
}

fn stop_after_mutation(f: &Fixture, mode: &str, enabled: bool) {
    fs::write(
        f.root.join("command.json"),
        serde_json::to_vec(&Command::SetPresetEnabled {
            preset_id: f.preset.as_str().into(),
            targets: vec![f.target.clone()],
            enabled,
        })
        .unwrap(),
    )
    .unwrap();
    let mut child = Process::new(std::env::current_exe().unwrap())
        .args(["--exact", "crash_child", "--nocapture"])
        .env("HOME", &f.home)
        .env("PACKCAPTAIN_RECOVERY_CHILD", &f.root)
        .env("PACKCAPTAIN_RECOVERY_MODE", mode)
        .stdout(Stdio::from(
            fs::File::create(f.root.join("child.stdout")).unwrap(),
        ))
        .stderr(Stdio::from(
            fs::File::create(f.root.join("child.stderr")).unwrap(),
        ))
        .spawn()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    while !f.root.join("mutation.marker").exists() {
        if let Some(status) = child.try_wait().unwrap() {
            panic!(
                "child ended before mutation: {status}; stdout={}; stderr={}",
                fs::read_to_string(f.root.join("child.stdout")).unwrap(),
                fs::read_to_string(f.root.join("child.stderr")).unwrap()
            );
        }
        if Instant::now() > deadline {
            child.kill().unwrap();
            child.wait().unwrap();
            panic!("child did not reach mutation");
        }
        thread::sleep(Duration::from_millis(10));
    }
    let pid = child.id();
    child.kill().unwrap();
    let status = child.wait().unwrap();
    eprintln!(
        "mode={mode} child_pid={pid} exit={status} restart_state={} pending_journals={}",
        f.state.display(),
        journal_count(&f.state)
    );
    eprintln!(
        "gate={}",
        fs::read_to_string(f.root.join("mutation.marker")).unwrap()
    );
    eprintln!("persistent_journal={}", journal_payload(&f.state));
    if let Ok(evidence) = std::env::var("PACKCAPTAIN_RECOVERY_EVIDENCE") {
        let destination = PathBuf::from(evidence).join(format!("{mode}-{pid}"));
        assert!(
            Process::new("cp")
                .arg("-R")
                .arg(&f.root)
                .arg(&destination)
                .status()
                .unwrap()
                .success()
        );
        eprintln!("crash_fixture={}", destination.display());
    }
    assert!(!status.success());
    assert_eq!(
        std::os::unix::process::ExitStatusExt::signal(&status),
        Some(9)
    );
    assert_eq!(journal_count(&f.state), 1);
}

fn journal_payload(state: &Path) -> Value {
    let connection = Connection::open(state.join("packcaptain.sqlite3")).unwrap();
    let payload: String = connection
        .query_row("SELECT payload FROM operation_journal", [], |r| r.get(0))
        .unwrap();
    serde_json::from_str(&payload).unwrap()
}

fn journal_count(state: &Path) -> usize {
    Connection::open(state.join("packcaptain.sqlite3"))
        .unwrap()
        .query_row("SELECT count(*) FROM operation_journal", [], |r| r.get(0))
        .unwrap()
}

#[test]
fn killed_forward_process_recovers_and_retry_converges() {
    let f = Fixture::new();
    stop_after_mutation(&f, "forward", true);
    assert!(f.path("a").exists() && f.path("r").exists());
    f.enable();
    assert_eq!(journal_count(&f.state), 0);
    let stored = f.snapshot();
    f.enable();
    assert_eq!(stored.targets, f.snapshot().targets);
    assert_eq!(stored.entries, f.snapshot().entries);
}

#[test]
fn killed_rollback_process_recovers_recreated_fingerprints() {
    let f = Fixture::new();
    f.enable();
    stop_after_mutation(&f, "rollback", false);
    assert!(f.path("a").exists() && f.path("r").exists());
    f.enable();
    assert_eq!(journal_count(&f.state), 0);
    ok(cli(
        &f.home,
        &f.state,
        &[
            "presets",
            "disable",
            &f.preset,
            "--target",
            &f.target.to_string(),
        ],
    ));
    assert!(!f.path("a").exists() && !f.path("r").exists());
}

#[test]
fn rollback_fingerprints_persist_after_real_native_recreation() {
    let f = Fixture::new();
    f.enable();
    let old = f.snapshot();
    let mut engine = Engine::new(
        f.home.clone(),
        f.state.clone(),
        InterruptingNative {
            native: NativeAdapters,
            calls: 0,
            mode: "rollback-return".into(),
            marker: f.root.join("unused"),
        },
    );
    let report = engine
        .execute(&Command::SetPresetEnabled {
            preset_id: f.preset.as_str().into(),
            targets: vec![f.target.clone()],
            enabled: false,
        })
        .unwrap();
    assert_eq!(report.targets[0].status, TargetOutcomeStatus::Failed);
    let restored = f.snapshot();
    let observed = NativeAdapters.discover(&f.home, &restored);
    assert_eq!(restored.targets, old.targets);
    for skill in f.skills.values() {
        assert_eq!(
            restored.entries[&f.target][skill][0].entry_fingerprint,
            observed.entries[&f.target][skill][0].entry_fingerprint
        );
    }
    assert_eq!(journal_count(&f.state), 0);
    ok(cli(
        &f.home,
        &f.state,
        &[
            "presets",
            "disable",
            &f.preset,
            "--target",
            &f.target.to_string(),
        ],
    ));
}

#[test]
fn unproven_actual_result_keeps_needs_recovery_without_inverse() {
    let f = Fixture::new();
    f.enable();
    let old = f.snapshot();
    let mut engine = Engine::new(
        f.home.clone(),
        f.state.clone(),
        InterruptingNative {
            native: NativeAdapters,
            calls: 0,
            mode: "rollback-unproven".into(),
            marker: f.root.join("unused"),
        },
    );
    let error = engine
        .execute(&Command::SetPresetEnabled {
            preset_id: f.preset.as_str().into(),
            targets: vec![f.target.clone()],
            enabled: false,
        })
        .unwrap_err();
    assert!(matches!(error, EngineError::NeedsRecovery { .. }));
    assert_eq!(f.snapshot().targets, old.targets);
    assert!(!f.path("a").exists() && !f.path("r").exists());
    assert_eq!(journal_count(&f.state), 1);
    let journal = journal_payload(&f.state);
    assert_eq!(journal["status"], "NeedsRecovery");
    assert!(
        journal["failure"]
            .as_str()
            .unwrap()
            .contains("failure after real filesystem mutation")
    );
}

#[test]
fn invalid_cli_does_not_recover_persisted_pending_mutation() {
    let f = Fixture::new();
    stop_after_mutation(&f, "forward", true);
    let before = f.snapshot();
    let connection = Connection::open(f.state.join("packcaptain.sqlite3")).unwrap();
    let journal: String = connection
        .query_row("SELECT payload FROM operation_journal", [], |r| r.get(0))
        .unwrap();
    let result = cli(
        &f.home,
        &f.state,
        &["skills", "enable", f.skills["a"].as_str()],
    );
    assert!(!result.status.success());
    assert_eq!(f.snapshot(), before);
    assert_eq!(
        connection
            .query_row::<String, _, _>("SELECT payload FROM operation_journal", [], |r| r.get(0))
            .unwrap(),
        journal
    );
    assert!(f.path("a").exists());
    assert_eq!(journal_count(&f.state), 1);
}

#[test]
fn conflicting_policy_does_not_apply_pending_journal() {
    let f = Fixture::new();
    f.enable();
    ok(cli(
        &f.home,
        &f.state,
        &[
            "skills",
            "disable",
            f.skills["a"].as_str(),
            "--target",
            &f.target.to_string(),
        ],
    ));
    let before = f.snapshot();
    let mut after = before.targets[&f.target].clone();
    after.manual_closed.insert(f.skills["r"].clone());
    after.enabled_skills.remove(&f.skills["r"]);
    let entries_before = &before.entries[&f.target];
    let mut entries_after = entries_before.clone();
    for entry in entries_after.get_mut(&f.skills["r"]).unwrap() {
        entry.active = false;
        entry.present = false;
    }
    let journal = json!({ "id": "pending", "target": f.target, "status": "Applying", "before": before.targets[&f.target], "after": after, "entries_before": entries_before, "entries_after": entries_after, "planned_target_states": {f.target.to_string(): after}, "effects": [Effect::SetSkillEnabled { target: f.target.clone(), skill: f.skills["r"].clone(), enabled: false }], "failure": null });
    let connection = Connection::open(f.state.join("packcaptain.sqlite3")).unwrap();
    connection
        .execute(
            "INSERT INTO operation_journal VALUES (?1, ?2)",
            params!["pending", journal.to_string()],
        )
        .unwrap();
    drop(connection);
    let result = cli(
        &f.home,
        &f.state,
        &[
            "presets",
            "update",
            &f.preset,
            "--sync-target",
            &f.target.to_string(),
        ],
    );
    assert!(!result.status.success());
    assert_eq!(
        serde_json::from_slice::<Value>(&result.stdout).unwrap()["error"]["code"],
        "closed-members-policy-required"
    );
    assert_eq!(f.snapshot(), before);
    assert!(f.path("r").exists());
    assert_eq!(journal_count(&f.state), 1);
}

#[test]
fn killed_before_native_mutation_recovers_in_fresh_cli() {
    let f = Fixture::new();
    stop_after_mutation(&f, "before-forward", true);
    assert!(!f.path("a").exists() && !f.path("r").exists());
    f.enable();
    assert_eq!(journal_count(&f.state), 0);
    assert!(f.path("a").exists() && f.path("r").exists());
    let recovered = f.snapshot();
    let observed = NativeAdapters.discover(&f.home, &recovered);
    for skill in f.skills.values() {
        assert_eq!(
            recovered.entries[&f.target][skill][0].entry_fingerprint,
            observed.entries[&f.target][skill][0].entry_fingerprint
        );
    }
    let target_state = recovered.targets[&f.target].clone();
    f.enable();
    assert_eq!(f.snapshot().targets[&f.target], target_state);
}

#[test]
fn persisted_actual_rollback_state_recreates_and_commits_fingerprints() {
    let f = Fixture::new();
    f.enable();
    let snapshot = f.snapshot();
    let before = &snapshot.targets[&f.target];
    let mut after = before.clone();
    after.enabled_skills.clear();
    after
        .presets
        .get_mut(&f.preset.as_str().into())
        .unwrap()
        .enabled = false;
    let mut actual = snapshot.entries[&f.target].clone();
    let mut effects = Vec::new();
    for skill in f.skills.values() {
        for entry in actual.get_mut(skill).unwrap() {
            fs::remove_file(&entry.path).unwrap();
            entry.present = false;
            entry.active = false;
            entry.entry_fingerprint = "missing".into();
        }
        effects.push(Effect::SetSkillEnabled {
            target: f.target.clone(),
            skill: skill.clone(),
            enabled: false,
        });
    }
    let journal = json!({ "id": "00000000-0000-4000-8000-000000000001", "target": f.target, "status": "RollingBack", "before": before, "after": after, "entries_before": snapshot.entries[&f.target], "entries_after": actual, "planned_target_states": {f.target.to_string(): before}, "effects": effects, "failure": "interrupted after forward metadata persisted" });
    let connection = Connection::open(f.state.join("packcaptain.sqlite3")).unwrap();
    connection
        .execute(
            "INSERT INTO operation_journal VALUES (?1, ?2)",
            params!["00000000-0000-4000-8000-000000000001", journal.to_string()],
        )
        .unwrap();
    drop(connection);
    f.enable();
    let restored = f.snapshot();
    let observed = NativeAdapters.discover(&f.home, &restored);
    for skill in f.skills.values() {
        assert!(restored.entries[&f.target][skill][0].present);
        assert_eq!(
            restored.entries[&f.target][skill][0].entry_fingerprint,
            observed.entries[&f.target][skill][0].entry_fingerprint
        );
    }
    assert_eq!(journal_count(&f.state), 0);
    ok(cli(
        &f.home,
        &f.state,
        &[
            "presets",
            "disable",
            &f.preset,
            "--target",
            &f.target.to_string(),
        ],
    ));
}
