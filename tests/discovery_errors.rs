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
        let binary = std::env::var_os("PACKCAPTAIN_DISCOVERY_BINARY")
            .unwrap_or_else(|| env!("CARGO_BIN_EXE_packcaptain").into());
        let output = Command::new(binary)
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
        if let Some(root) = std::env::var_os("PACKCAPTAIN_DISCOVERY_EVIDENCE") {
            let directory = PathBuf::from(root).join(uuid::Uuid::new_v4().to_string());
            fs::create_dir_all(&directory).unwrap();
            fs::write(directory.join("stdout.json"), &output.stdout).unwrap();
            fs::write(directory.join("stderr.txt"), &output.stderr).unwrap();
            fs::write(
                directory.join("command.json"),
                serde_json::to_vec(&serde_json::json!({
                    "args": args, "exit": output.status.code(), "project": self.project,
                    "agent_home": self.home, "state_dir": self.state,
                }))
                .unwrap(),
            )
            .unwrap();
        }
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
        .map(Result::unwrap)
        .find(|entry| {
            entry.file_type().unwrap().is_symlink()
                && entry.file_name().to_string_lossy().starts_with("pc-")
        })
        .unwrap()
        .path();
    let metadata = fs::symlink_metadata(&path).unwrap();
    assert!(metadata.file_type().is_symlink());
    (metadata.dev(), metadata.ino(), fs::read_link(path).unwrap())
}

#[cfg(unix)]
struct PermissionsGuard(PathBuf, fs::Permissions);
#[cfg(unix)]
impl PermissionsGuard {
    fn deny(path: &Path) -> Self {
        use std::os::unix::fs::PermissionsExt;
        let guard = Self(
            path.to_path_buf(),
            fs::metadata(path).unwrap().permissions(),
        );
        fs::set_permissions(path, fs::Permissions::from_mode(0)).unwrap();
        guard
    }
}
#[cfg(unix)]
impl Drop for PermissionsGuard {
    fn drop(&mut self) {
        fs::set_permissions(&self.0, self.1.clone()).unwrap();
    }
}

#[cfg(unix)]
fn assert_unknown(f: &Fixture, before: &packcaptain::domain::Snapshot) {
    let target: TargetId = f.target.parse().unwrap();
    let link = link_identity(&f.project);
    let database = f.state.join("packcaptain.sqlite3");
    let database_before = fs::read(&database).unwrap();
    let discovery = NativeAdapters.discover(&f.home, before);
    assert!(
        discovery.target_errors.contains_key(&target),
        "{discovery:?}"
    );
    assert_eq!(
        discovery.merge_into(before).targets[&target],
        before.targets[&target]
    );
    assert_eq!(
        discovery.merge_into(before).entries[&target],
        before.entries[&target]
    );
    assert_eq!(discovery.merge_into(before).skills, before.skills);
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
        serde_json::to_value(&before.targets[&target]).unwrap()
    );
    for preview in [true, false] {
        let mut args = vec!["presets", "disable", &f.preset, "--target", &f.target];
        if preview {
            args.push("--preview");
        }
        let (ok, result) = f.call(&args);
        assert!(!ok, "{result}");
        assert_eq!(result["ok"], false);
        assert!(result.to_string().contains("unknown"), "{result}");
    }
    assert_eq!(f.engine().snapshot().unwrap(), *before);
    assert!(journal_payloads(&f.state).is_empty());
    assert_eq!(link_identity(&f.project), link);
    assert_eq!(fs::read(&database).unwrap(), database_before);
}

#[cfg(unix)]
#[test]
fn unreadable_source_definition_is_unknown_not_disabled() {
    let f = Fixture::new();
    let before = f.engine().snapshot().unwrap();
    let definition = f.source.join("SKILL.md");
    let guard = PermissionsGuard::deny(&definition);
    assert_eq!(
        fs::read(&definition).unwrap_err().kind(),
        std::io::ErrorKind::PermissionDenied
    );
    assert_unknown(&f, &before);
    drop(guard);
    f.ok(&["presets", "disable", &f.preset, "--target", &f.target]);
}

#[cfg(unix)]
#[test]
fn walkdir_permission_failure_is_unknown() {
    let f = Fixture::new();
    let blocked = f.project.join(".claude/skills/z-unreadable");
    fs::create_dir(&blocked).unwrap();
    let before = f.engine().snapshot().unwrap();
    let guard = PermissionsGuard::deny(&blocked);
    assert_eq!(
        fs::read_dir(&blocked).unwrap_err().kind(),
        std::io::ErrorKind::PermissionDenied
    );
    assert_unknown(&f, &before);
    drop(guard);
}

#[cfg(unix)]
#[test]
fn source_fingerprint_loop_is_unknown() {
    let f = Fixture::new();
    let before = f.engine().snapshot().unwrap();
    std::os::unix::fs::symlink(&f.source, f.source.join("recursive")).unwrap();
    assert_unknown(&f, &before);
}

#[cfg(unix)]
#[test]
fn canonicalize_loop_is_unknown() {
    let f = Fixture::new();
    let before = f.engine().snapshot().unwrap();
    let loop_path = f.project.join(".claude/skills/z-loop");
    std::os::unix::fs::symlink(&loop_path, &loop_path).unwrap();
    assert!(fs::canonicalize(&loop_path).is_err());
    assert_unknown(&f, &before);
}

#[test]
fn non_directory_root_and_parent_are_unknown_but_absent_roots_are_normal() {
    use packcaptain::domain::{AgentId, Scope, Snapshot};
    let root = tempfile::tempdir().unwrap();
    let target = TargetId {
        agent: AgentId::Codex,
        scope: Scope::Global,
    };
    assert!(
        NativeAdapters
            .discover(root.path(), &Snapshot::default())
            .target_errors
            .is_empty()
    );
    fs::create_dir(root.path().join(".agents")).unwrap();
    fs::write(root.path().join(".agents/skills"), "not a directory").unwrap();
    assert!(
        NativeAdapters
            .discover(root.path(), &Snapshot::default())
            .target_errors
            .contains_key(&target)
    );
    fs::remove_file(root.path().join(".agents/skills")).unwrap();
    fs::remove_dir(root.path().join(".agents")).unwrap();
    fs::write(root.path().join(".agents"), "not a directory").unwrap();
    assert!(
        NativeAdapters
            .discover(root.path(), &Snapshot::default())
            .target_errors
            .contains_key(&target)
    );
}

#[cfg(unix)]
#[test]
fn missing_source_remains_missing_and_restoration_cleans_old_member() {
    use packcaptain::domain::SkillAvailability;
    let f = Fixture::new();
    let before = f.engine().snapshot().unwrap();
    fs::remove_file(f.source.join("SKILL.md")).unwrap();
    let discovery = NativeAdapters.discover(&f.home, &before);
    assert!(discovery.target_errors.is_empty());
    assert_eq!(
        discovery.skills.values().next().unwrap().availability,
        SkillAvailability::Missing
    );
    fs::write(
        f.source.join("SKILL.md"),
        "---\nname: s\ndescription: fixture\n---\n",
    )
    .unwrap();
    f.ok(&["presets", "disable", &f.preset, "--target", &f.target]);
    assert_eq!(
        fs::read_dir(f.project.join(".claude/skills"))
            .unwrap()
            .count(),
        0
    );
}
#[cfg(unix)]
#[test]
fn scan_error_rejects_confirmed_execution_and_preserves_pending_journal_bytes() {
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
    let loop_path = f.project.join(".claude/skills/z-loop");
    std::os::unix::fs::symlink(&loop_path, &loop_path).unwrap();
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

#[cfg(unix)]
#[test]
fn two_unknown_targets_preserve_intent_while_healthy_confirmation_applies() {
    use packcaptain::{domain::Effect, engine::TargetOutcomeStatus};
    let f = Fixture::new();
    let project_id = f.target.strip_prefix("claude-code:project:").unwrap();
    let unknown: Vec<TargetId> = ["codex", "pi"]
        .iter()
        .map(|agent| format!("{agent}:project:{project_id}").parse().unwrap())
        .collect();
    for target in &unknown {
        f.ok(&[
            "presets",
            "enable",
            &f.preset,
            "--target",
            &target.to_string(),
        ]);
    }
    let healthy: TargetId = f.target.parse().unwrap();
    let command = DomainCommand::SetPresetEnabled {
        preset_id: PresetId::from(f.preset.as_str()),
        targets: vec![unknown[0].clone(), unknown[1].clone(), healthy.clone()],
        enabled: false,
    };
    let mut engine = f.engine();
    let confirmed = engine.preview_with_native_discovery(&command).unwrap();
    let before = engine.snapshot().unwrap();
    let shared_root = f.project.join(".agents/skills");
    fs::create_dir_all(&shared_root).unwrap();
    let loop_path = shared_root.join("loop");
    std::os::unix::fs::symlink(&loop_path, &loop_path).unwrap();
    let report = engine
        .execute_confirmed_with_native_discovery(&command, &confirmed)
        .unwrap();
    let after = engine.snapshot().unwrap();
    for target in &unknown {
        assert_eq!(after.targets[target], before.targets[target]);
        assert_eq!(after.entries[target], before.entries[target]);
        assert!(!report.plan.target_updates.contains_key(target));
        assert!(report.plan.effects.iter().all(|effect| match effect {
            Effect::SetSkillEnabled {
                target: affected, ..
            } => affected != target,
        }));
        assert_eq!(
            report
                .targets
                .iter()
                .find(|row| &row.target == target)
                .unwrap()
                .status,
            TargetOutcomeStatus::Failed
        );
        for entry in after.entries[target].values().flatten() {
            assert!(
                fs::symlink_metadata(&entry.path)
                    .unwrap()
                    .file_type()
                    .is_symlink()
            );
        }
    }
    assert_eq!(
        report
            .targets
            .iter()
            .find(|row| row.target == healthy)
            .unwrap()
            .status,
        TargetOutcomeStatus::Applied
    );
    assert_eq!(
        fs::read_dir(f.project.join(".claude/skills"))
            .unwrap()
            .count(),
        0
    );
    assert!(journal_payloads(&f.state).is_empty());
}

#[cfg(unix)]
#[test]
fn dangling_owned_link_does_not_claim_same_source_foreign_replacement() {
    use packcaptain::domain::SkillAvailability;
    let f = Fixture::new();
    let before = f.engine().snapshot().unwrap();
    let target: TargetId = f.target.parse().unwrap();
    let entry = before.entries[&target].values().next().unwrap()[0]
        .path
        .clone();
    fs::remove_dir_all(&f.source).unwrap();
    let discovery = NativeAdapters.discover(&f.home, &before);
    assert!(discovery.target_errors.is_empty());
    assert_eq!(
        discovery.skills.values().next().unwrap().availability,
        SkillAvailability::Missing
    );
    fs::remove_file(&entry).unwrap();
    std::os::unix::fs::symlink(&f.source, &entry).unwrap();
    let foreign = link_identity(&f.project);
    fs::create_dir_all(&f.source).unwrap();
    fs::write(
        f.source.join("SKILL.md"),
        "---\nname: s\ndescription: fixture\n---\n",
    )
    .unwrap();
    let (ok, _) = f.call(&["presets", "disable", &f.preset, "--target", &f.target]);
    assert!(!ok);
    assert_eq!(link_identity(&f.project), foreign);
    let after = f.engine().snapshot().unwrap();
    assert_eq!(after.targets[&target], before.targets[&target]);
    assert_eq!(after.entries[&target], before.entries[&target]);
    assert!(journal_payloads(&f.state).is_empty());
}

#[cfg(unix)]
#[test]
fn unreadable_root_propagates_walkdir_error_without_writes() {
    let f = Fixture::new();
    let before = f.engine().snapshot().unwrap();
    let target: TargetId = f.target.parse().unwrap();
    let identity = link_identity(&f.project);
    let root = f.project.join(".claude/skills");
    let guard = PermissionsGuard::deny(&root);
    assert!(
        walkdir::WalkDir::new(&root)
            .into_iter()
            .any(|item| item.is_err_and(|error| error
                .io_error()
                .is_some_and(|error| error.kind() == std::io::ErrorKind::PermissionDenied)))
    );
    let discovery = NativeAdapters.discover(&f.home, &before);
    assert!(
        discovery.target_errors[&target]
            .to_string()
            .contains("cannot scan")
    );
    assert_eq!(
        discovery.merge_into(&before).entries[&target],
        before.entries[&target]
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
        serde_json::to_value(&before.targets[&target]).unwrap()
    );
    for preview in [true, false] {
        let mut args = vec!["presets", "disable", &f.preset, "--target", &f.target];
        if preview {
            args.push("--preview");
        }
        assert!(!f.call(&args).0);
    }
    assert_eq!(f.engine().snapshot().unwrap(), before);
    assert!(journal_payloads(&f.state).is_empty());
    drop(guard);
    assert_eq!(link_identity(&f.project), identity);
}
