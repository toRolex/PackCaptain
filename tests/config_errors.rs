use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
};

use packcaptain::{
    adapters::NativeAdapters,
    domain::{Command as DomainCommand, PresetId, TargetId},
    engine::Engine,
};
use serde_json::Value;
use tempfile::TempDir;

fn journal_payloads(state: &Path) -> Vec<String> {
    let connection = rusqlite::Connection::open(state.join("packcaptain.sqlite3")).unwrap();
    let mut statement = connection
        .prepare("SELECT payload FROM operation_journal ORDER BY operation_id")
        .unwrap();
    statement
        .query_map([], |row| row.get(0))
        .unwrap()
        .map(Result::unwrap)
        .collect()
}

struct Fixture {
    _root: TempDir,
    home: PathBuf,
    state: PathBuf,
    project: PathBuf,
    preset: String,
    target: String,
    source: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let root = tempfile::tempdir().unwrap();
        let home = root.path().join("home");
        let state = root.path().join("state");
        let project = root.path().join("project");
        let source = home.join(".claude/skills/s");
        fs::create_dir_all(&source).unwrap();
        fs::create_dir_all(&project).unwrap();
        fs::write(
            source.join("SKILL.md"),
            "---\nname: s\ndescription: fixture\n---\n",
        )
        .unwrap();
        let mut f = Self {
            _root: root,
            home,
            state,
            project,
            source,
            preset: String::new(),
            target: String::new(),
        };
        let project = f.ok(&["projects", "add", f.project.to_str().unwrap()]);
        f.target = format!("claude-code:project:{}", project["id"].as_str().unwrap());
        let skills = f.ok(&["skills", "list"]);
        let id = skills[0]["id"].as_str().unwrap();
        f.ok(&["presets", "create", "--name", "A", "--skill", id]);
        f.preset = f.ok(&["presets", "list"])[0]["id"]
            .as_str()
            .unwrap()
            .to_owned();
        f.ok(&["presets", "enable", &f.preset, "--target", &f.target]);
        f
    }
    fn call(&self, args: &[&str]) -> (bool, Value) {
        let output = Command::new(env!("CARGO_BIN_EXE_packcaptain"))
            .env("HOME", &self.home)
            .env("XDG_CONFIG_HOME", self.home.join("xdg-config"))
            .env("XDG_STATE_HOME", self.home.join("xdg-state"))
            .current_dir(&self.project)
            .args([
                "--agent-home",
                self.home.to_str().unwrap(),
                "--state-dir",
                self.state.to_str().unwrap(),
                "--json",
            ])
            .args(args)
            .output()
            .unwrap();
        (
            output.status.success(),
            serde_json::from_slice(&output.stdout).unwrap(),
        )
    }
    fn ok(&self, args: &[&str]) -> Value {
        let (ok, value) = self.call(args);
        assert!(ok, "{value}");
        value["data"].clone()
    }
    fn config(&self) -> PathBuf {
        self.project.join(".claude/settings.local.json")
    }
    fn engine(&self) -> Engine<NativeAdapters> {
        Engine::new(self.home.clone(), self.state.clone(), NativeAdapters)
    }
    fn command(&self) -> DomainCommand {
        DomainCommand::SetPresetEnabled {
            preset_id: PresetId::from(self.preset.as_str()),
            targets: vec![self.target.parse().unwrap()],
            enabled: false,
        }
    }
}

#[cfg(unix)]
fn link_identity(project: &Path) -> (u64, u64, PathBuf) {
    use std::os::unix::fs::MetadataExt;
    let path = fs::read_dir(project.join(".claude/skills"))
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    let metadata = fs::symlink_metadata(&path).unwrap();
    assert!(metadata.file_type().is_symlink());
    (metadata.dev(), metadata.ino(), fs::read_link(path).unwrap())
}

#[cfg(unix)]
fn reject_and_retry(read_failure: bool) {
    let f = Fixture::new();
    let snapshot = f.engine().snapshot().unwrap();
    let link = link_identity(&f.project);
    let source = fs::read(f.source.join("SKILL.md")).unwrap();
    if read_failure {
        fs::create_dir(f.config()).unwrap();
    } else {
        fs::write(f.config(), "{broken-json").unwrap();
    }
    let config_before = if read_failure {
        None
    } else {
        Some(fs::read(f.config()).unwrap())
    };
    let target: TargetId = f.target.parse().unwrap();
    let discovery = NativeAdapters.discover(&f.home, &snapshot);
    assert!(discovery.target_errors.contains_key(&target));
    assert_eq!(
        discovery.merge_into(&snapshot).targets[&target],
        snapshot.targets[&target]
    );
    assert_eq!(
        discovery.merge_into(&snapshot).entries[&target],
        snapshot.entries[&target]
    );
    let rows = f.ok(&["targets", "list"]);
    let row = rows
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["id"] == f.target)
        .unwrap();
    assert_eq!(row["observation"], "unknown");
    assert_eq!(
        row["state"],
        serde_json::to_value(&snapshot.targets[&target]).unwrap()
    );
    assert!(
        row["diagnostic"]
            .as_str()
            .unwrap()
            .contains(if read_failure {
                "cannot read"
            } else {
                "invalid"
            })
    );
    let (ok, preview) = f.call(&[
        "presets",
        "disable",
        &f.preset,
        "--target",
        &f.target,
        "--preview",
    ]);
    assert!(!ok);
    assert_eq!(preview["ok"], false);
    assert!(
        f.engine()
            .preview_with_native_discovery(&f.command())
            .is_err()
    );
    let (ok, disabled) = f.call(&["presets", "disable", &f.preset, "--target", &f.target]);
    assert!(!ok);
    assert_eq!(disabled["ok"], false);
    assert!(disabled.to_string().contains("unknown"));
    assert_eq!(f.engine().snapshot().unwrap(), snapshot);
    assert!(journal_payloads(&f.state).is_empty());
    assert_eq!(link_identity(&f.project), link);
    assert_eq!(fs::read(f.source.join("SKILL.md")).unwrap(), source);
    if let Some(bytes) = config_before {
        assert_eq!(fs::read(f.config()).unwrap(), bytes);
    } else {
        assert!(f.config().is_dir());
    }
    if read_failure {
        fs::remove_dir(f.config()).unwrap();
    }
    fs::write(f.config(), "{}").unwrap();
    f.ok(&["presets", "disable", &f.preset, "--target", &f.target]);
    assert!(
        !f.engine().snapshot().unwrap().targets[&target].presets
            [&PresetId::from(f.preset.as_str())]
            .enabled
    );
    assert_eq!(
        fs::read_dir(f.project.join(".claude/skills"))
            .unwrap()
            .count(),
        0
    );
    assert_eq!(fs::read(f.source.join("SKILL.md")).unwrap(), source);
}

#[cfg(unix)]
#[test]
fn invalid_config_preserves_intent_ownership_and_retry_converges() {
    reject_and_retry(false);
}

#[cfg(unix)]
#[test]
fn config_read_failure_preserves_intent_ownership_and_retry_converges() {
    reject_and_retry(true);
}

#[cfg(unix)]
#[test]
fn multi_target_config_failure_keeps_failed_target_and_applies_healthy_target() {
    let f = Fixture::new();
    let second = f.project.parent().unwrap().join("healthy");
    fs::create_dir(&second).unwrap();
    let record = f.ok(&["projects", "add", second.to_str().unwrap()]);
    let healthy = format!("claude-code:project:{}", record["id"].as_str().unwrap());
    f.ok(&["presets", "enable", &f.preset, "--target", &healthy]);
    let before = f.engine().snapshot().unwrap();
    let target: TargetId = f.target.parse().unwrap();
    let link = link_identity(&f.project);
    fs::write(f.config(), "{broken-json").unwrap();
    let (ok, result) = f.call(&[
        "presets", "disable", &f.preset, "--target", &f.target, "--target", &healthy,
    ]);
    assert!(!ok);
    assert_eq!(result["error"]["code"], "partial-failure");
    let outcomes = result["error"]["impact"]["targets"].as_array().unwrap();
    assert_eq!(
        outcomes
            .iter()
            .find(|row| row["target"] == f.target)
            .unwrap()["status"],
        "failed"
    );
    assert_eq!(
        outcomes
            .iter()
            .find(|row| row["target"] == healthy)
            .unwrap()["status"],
        "applied"
    );
    let after = f.engine().snapshot().unwrap();
    assert_eq!(after.targets[&target], before.targets[&target]);
    assert_eq!(after.entries[&target], before.entries[&target]);
    assert_eq!(link_identity(&f.project), link);
    assert_eq!(
        fs::read_dir(second.join(".claude/skills")).unwrap().count(),
        0
    );
    assert!(journal_payloads(&f.state).is_empty());
}

#[cfg(unix)]
#[test]
fn config_error_rejects_confirmed_execution_and_preserves_pending_journal_bytes() {
    let f = Fixture::new();
    let mut engine = f.engine();
    let command = f.command();
    let confirmed = engine.preview_with_native_discovery(&command).unwrap();
    let snapshot = engine.snapshot().unwrap();
    let target: TargetId = f.target.parse().unwrap();
    let link = link_identity(&f.project);
    let journal = serde_json::json!({
        "id": uuid::Uuid::new_v4().to_string(), "target": target,
        "status": "Applying", "before": snapshot.targets[&target],
        "after": confirmed.target_updates[&target], "entries_before": snapshot.entries[&target],
        "entries_after": snapshot.entries[&target], "planned_target_states": confirmed.target_updates,
        "effects": confirmed.effects, "failure": null,
    });
    let connection = rusqlite::Connection::open(f.state.join("packcaptain.sqlite3")).unwrap();
    connection
        .execute(
            "INSERT INTO operation_journal(operation_id,payload) VALUES (?1,?2)",
            [journal["id"].as_str().unwrap(), &journal.to_string()],
        )
        .unwrap();
    drop(connection);
    let payload = || {
        let connection = rusqlite::Connection::open(f.state.join("packcaptain.sqlite3")).unwrap();
        connection
            .query_row(
                "SELECT payload FROM operation_journal WHERE operation_id = ?1",
                [journal["id"].as_str().unwrap()],
                |row| row.get::<_, String>(0),
            )
            .unwrap()
    };
    let before = payload();
    fs::write(f.config(), "{broken-json").unwrap();
    assert!(
        engine
            .execute_confirmed_with_native_discovery(&command, &confirmed)
            .is_err()
    );
    let (ok, result) = f.call(&["presets", "disable", &f.preset, "--target", &f.target]);
    assert!(!ok);
    assert!(result.to_string().contains("unknown"));
    assert_eq!(payload(), before);
    assert_eq!(engine.snapshot().unwrap(), snapshot);
    assert_eq!(link_identity(&f.project), link);
    assert_eq!(journal_payloads(&f.state), vec![before]);
}

#[test]
fn empty_target_invalid_configs_are_unknown_for_each_adapter() {
    let root = tempfile::tempdir().unwrap();
    let home = root.path();
    for (path, bytes) in [
        (".claude/settings.json", "{broken"),
        (".codex/config.toml", "[broken"),
        (".pi/agent/settings.json", "{broken"),
    ] {
        let path = home.join(path);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, bytes).unwrap();
    }
    let discovery = NativeAdapters.discover(home, &Default::default());
    assert_eq!(discovery.target_errors.len(), 3);
    assert!(
        discovery
            .target_errors
            .values()
            .all(|error| error.to_string().contains("invalid native config"))
    );
}

#[cfg(unix)]
#[test]
fn confirmed_mixed_targets_compare_healthy_plan_and_never_plan_unknown_mutation() {
    use packcaptain::engine::{EngineError, TargetOutcomeStatus};
    let f = Fixture::new();
    let second = f.project.parent().unwrap().join("healthy-confirmed");
    fs::create_dir(&second).unwrap();
    let record = f.ok(&["projects", "add", second.to_str().unwrap()]);
    let healthy: TargetId = format!("claude-code:project:{}", record["id"].as_str().unwrap())
        .parse()
        .unwrap();
    let target: TargetId = f.target.parse().unwrap();
    let command = DomainCommand::SetPresetEnabled {
        preset_id: PresetId::from(f.preset.as_str()),
        targets: vec![target.clone(), healthy.clone()],
        enabled: false,
    };
    let mut engine = f.engine();
    let confirmed = engine.preview_with_native_discovery(&command).unwrap();
    f.ok(&[
        "presets",
        "enable",
        &f.preset,
        "--target",
        &healthy.to_string(),
    ]);
    let snapshot = engine.snapshot().unwrap();
    let link = link_identity(&f.project);
    fs::write(f.config(), "{broken-json").unwrap();
    let error = engine
        .execute_confirmed_with_native_discovery(&command, &confirmed)
        .unwrap_err();
    assert!(matches!(error, EngineError::PreviewChanged(_)));
    assert_eq!(engine.snapshot().unwrap(), snapshot);
    assert_eq!(link_identity(&f.project), link);
    assert!(journal_payloads(&f.state).is_empty());
    fs::write(f.config(), "{}").unwrap();
    let confirmed = engine.preview_with_native_discovery(&command).unwrap();
    fs::write(f.config(), "{broken-json").unwrap();
    let report = engine
        .execute_confirmed_with_native_discovery(&command, &confirmed)
        .unwrap();
    assert!(!report.plan.target_updates.contains_key(&target));
    assert!(report.plan.effects.iter().all(|effect| match effect {
        packcaptain::domain::Effect::SetSkillEnabled {
            target: affected, ..
        } => affected != &target,
    }));
    assert_eq!(
        report
            .targets
            .iter()
            .find(|outcome| outcome.target == target)
            .unwrap()
            .status,
        TargetOutcomeStatus::Failed
    );
    assert_eq!(
        report
            .targets
            .iter()
            .find(|outcome| outcome.target == healthy)
            .unwrap()
            .status,
        TargetOutcomeStatus::Applied
    );
    let after = engine.snapshot().unwrap();
    assert_eq!(after.targets[&target], snapshot.targets[&target]);
    assert_eq!(after.entries[&target], snapshot.entries[&target]);
    assert_eq!(link_identity(&f.project), link);
    assert_eq!(fs::read(f.config()).unwrap(), b"{broken-json");
    assert_eq!(
        fs::read_dir(second.join(".claude/skills")).unwrap().count(),
        0
    );
    assert!(journal_payloads(&f.state).is_empty());
}
