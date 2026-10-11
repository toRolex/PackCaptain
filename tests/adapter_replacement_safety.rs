#![cfg(unix)]

use packcaptain::{
    adapters::NativeAdapters,
    domain::{
        self, AgentId, Command, ProjectId, ProjectRecord, Scope, SkillId, Snapshot, TargetId,
        TargetState,
    },
    engine::{EffectExecutor, TargetMutation},
};
use serde_json::Value;
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    process::{Command as ProcessCommand, Output},
};
use tempfile::TempDir;

fn cli(home: &Path, state: &Path, args: &[&str]) -> Output {
    ProcessCommand::new(env!("CARGO_BIN_EXE_packcaptain"))
        .env("HOME", home)
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

fn data(output: Output) -> Value {
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stdout)
    );
    let value: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(value["ok"], true);
    value["data"].clone()
}

fn replacement_cli(same_source: bool) {
    let temp = TempDir::new_in("/private/tmp").unwrap();
    let home = temp.path().join("home");
    let state = temp.path().join("state");
    let project = temp.path().join("project");
    fs::create_dir_all(&project).unwrap();
    for name in ["s", "b"] {
        let source = home.join(".claude/skills").join(name);
        fs::create_dir_all(&source).unwrap();
        fs::write(
            source.join("SKILL.md"),
            format!("---\nname: {name}\ndescription: fixture\n---\n"),
        )
        .unwrap();
    }
    let skills = data(cli(&home, &state, &["skills", "list"]));
    let id = |name: &str| {
        skills
            .as_array()
            .unwrap()
            .iter()
            .find(|s| s["name"] == name)
            .unwrap()["id"]
            .as_str()
            .unwrap()
            .to_owned()
    };
    let record = data(cli(
        &home,
        &state,
        &["projects", "add", project.to_str().unwrap()],
    ));
    let target = format!("claude-code:project:{}", record["id"].as_str().unwrap());
    data(cli(
        &home,
        &state,
        &[
            "presets",
            "create",
            "--name",
            "B",
            "--skill",
            &id("s"),
            "--skill",
            &id("b"),
        ],
    ));
    let presets = data(cli(&home, &state, &["presets", "list"]));
    let preset = presets[0]["id"].as_str().unwrap();
    data(cli(
        &home,
        &state,
        &["presets", "enable", preset, "--target", &target],
    ));
    data(cli(
        &home,
        &state,
        &[
            "presets",
            "disable",
            preset,
            "--target",
            &target,
            "--preview",
        ],
    ));
    let entry = project
        .join(".claude/skills")
        .join(format!("pc-{}", id("s")));
    let normal = project
        .join(".claude/skills")
        .join(format!("pc-{}", id("b")));
    let held = temp.path().join("held");
    fs::rename(&entry, &held).unwrap();
    if same_source {
        std::os::unix::fs::symlink(home.join(".claude/skills/s"), &entry).unwrap();
    } else {
        fs::create_dir(&entry).unwrap();
        fs::write(
            entry.join("SKILL.md"),
            "---\nname: foreign\ndescription: foreign\n---\n",
        )
        .unwrap();
    }
    let before = data(cli(&home, &state, &["targets", "list"]));
    let conflict = cli(
        &home,
        &state,
        &["presets", "disable", preset, "--target", &target],
    );
    let response: Value = serde_json::from_slice(&conflict.stdout).unwrap();
    assert!(
        !conflict.status.success(),
        "external replacement was accepted: {response}"
    );
    assert_eq!(response["ok"], false);
    let after = data(cli(&home, &state, &["targets", "list"]));
    let target_state = |rows: &Value| {
        rows.as_array()
            .unwrap()
            .iter()
            .find(|r| r["id"] == target)
            .unwrap()["state"]
            .clone()
    };
    assert_eq!(target_state(&before), target_state(&after));
    assert_eq!(target_state(&after)["presets"][preset]["enabled"], true);
    assert!(normal.is_symlink());
    assert!(held.is_symlink());
    if same_source {
        assert!(entry.is_symlink());
    } else {
        assert!(!entry.is_symlink());
        assert_eq!(
            fs::read_to_string(entry.join("SKILL.md")).unwrap(),
            "---\nname: foreign\ndescription: foreign\n---\n"
        );
    }
}

#[test]
fn cli_rejects_foreign_directory_after_preview_without_losing_intent() {
    replacement_cli(false);
}

#[test]
fn cli_does_not_inherit_ownership_of_same_source_replacement() {
    replacement_cli(true);
}

struct AdapterFixture {
    _temp: TempDir,
    home: PathBuf,
    snapshot: Snapshot,
    target: TargetId,
    skill: SkillId,
}

fn adapter_fixture() -> AdapterFixture {
    let temp = TempDir::new_in("/private/tmp").unwrap();
    let home = temp.path().join("home");
    let source = home.join(".agents/skills/source");
    fs::create_dir_all(&source).unwrap();
    fs::write(
        source.join("SKILL.md"),
        "---\nname: source\ndescription: fixture\n---\n",
    )
    .unwrap();
    let home = fs::canonicalize(home).unwrap();
    let project = temp.path().join("project");
    fs::create_dir(&project).unwrap();
    let project_id = ProjectId::from("fixture");
    let target = TargetId {
        agent: AgentId::Pi,
        scope: Scope::Project(project_id.clone()),
    };
    let seed = Snapshot {
        projects: BTreeMap::from([(
            project_id.clone(),
            ProjectRecord {
                id: project_id,
                name: "fixture".into(),
                root: project,
            },
        )]),
        ..Snapshot::default()
    };
    let mut snapshot = NativeAdapters.discover(&home, &seed).merge_into(&seed);
    snapshot
        .targets
        .insert(target.clone(), TargetState::default());
    AdapterFixture {
        _temp: temp,
        home,
        snapshot,
        target,
        skill: SkillId::from_source_path(&fs::canonicalize(source).unwrap()),
    }
}

#[test]
fn pending_creation_does_not_claim_external_same_source_link() {
    let fixture = adapter_fixture();
    let plan = domain::plan(
        &Command::SetSkillEnabled {
            skill_id: fixture.skill.clone(),
            targets: vec![fixture.target.clone()],
            enabled: true,
        },
        &fixture.snapshot,
    )
    .unwrap();
    let empty = BTreeMap::new();
    let states = BTreeMap::new();
    let mutation = |after| TargetMutation {
        receipt_context: None,
        target: &fixture.target,
        before: &fixture.snapshot.targets[&fixture.target],
        after: &plan.target_updates[&fixture.target],
        effects: &plan.effects,
        entries_before: &empty,
        entries_after: after,
        planned_target_states: &states,
    };
    let planned = NativeAdapters
        .prepare_target(mutation(&empty), &fixture.snapshot, &fixture.home)
        .unwrap();
    let entry = &planned[&fixture.skill][0];
    fs::create_dir_all(entry.path.parent().unwrap()).unwrap();
    std::os::unix::fs::symlink(&entry.source_path, &entry.path).unwrap();
    let result = NativeAdapters.apply_target(mutation(&planned), &fixture.snapshot, &fixture.home);
    assert!(result.is_err(), "pending apply claimed an external link");
    assert!(entry.path.is_symlink());
    assert_eq!(fs::canonicalize(&entry.path).unwrap(), entry.source_path);
}

#[test]
fn mixed_native_and_owned_routes_are_rejected_before_false_disable() {
    let mut fixture = adapter_fixture();
    let enable = domain::plan(
        &Command::SetSkillEnabled {
            skill_id: fixture.skill.clone(),
            targets: vec![fixture.target.clone()],
            enabled: true,
        },
        &fixture.snapshot,
    )
    .unwrap();
    let empty = BTreeMap::new();
    let states = BTreeMap::new();
    let mutation = |after| TargetMutation {
        receipt_context: None,
        target: &fixture.target,
        before: &fixture.snapshot.targets[&fixture.target],
        after: &enable.target_updates[&fixture.target],
        effects: &enable.effects,
        entries_before: &empty,
        entries_after: after,
        planned_target_states: &states,
    };
    let planned = NativeAdapters
        .prepare_target(mutation(&empty), &fixture.snapshot, &fixture.home)
        .unwrap();
    let applied = NativeAdapters
        .apply_target(mutation(&planned), &fixture.snapshot, &fixture.home)
        .unwrap();
    let entry = applied[&fixture.skill][0].clone();
    fixture
        .snapshot
        .entries
        .insert(fixture.target.clone(), applied);
    fixture.snapshot.targets.insert(
        fixture.target.clone(),
        enable.target_updates[&fixture.target].clone(),
    );
    let alias = entry.path.parent().unwrap().join("native-alias");
    std::os::unix::fs::symlink(&entry.source_path, &alias).unwrap();
    // The global route would separately reject Pi filtering and mask the mixed-route bug.
    fixture
        .snapshot
        .entries
        .retain(|target, _| target == &fixture.target);
    let discovered = NativeAdapters
        .discover(&fixture.home, &fixture.snapshot)
        .merge_into(&fixture.snapshot);
    let mut snapshot = discovered;
    snapshot
        .entries
        .retain(|target, _| target == &fixture.target);
    let disable = domain::plan(
        &Command::SetSkillEnabled {
            skill_id: fixture.skill.clone(),
            targets: vec![fixture.target.clone()],
            enabled: false,
        },
        &snapshot,
    )
    .unwrap();
    let before = &snapshot.entries[&fixture.target];
    let result = NativeAdapters.prepare_target(
        TargetMutation {
            receipt_context: None,
            target: &fixture.target,
            before: &snapshot.targets[&fixture.target],
            after: &disable.target_updates[&fixture.target],
            effects: &disable.effects,
            entries_before: before,
            entries_after: before,
            planned_target_states: &states,
        },
        &snapshot,
        &fixture.home,
    );
    assert!(
        result.is_err(),
        "mixed routes would report disabled without changing the owned entry"
    );
    assert!(entry.path.is_symlink());
    assert!(alias.is_symlink());
    assert!(
        !entry
            .path
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .join("settings.json")
            .exists()
    );
}

#[test]
fn native_config_change_after_prepare_is_rejected_without_overwrite() {
    let temp = TempDir::new_in("/private/tmp").unwrap();
    let home = temp.path().join("home");
    let source = home.join(".claude/skills/source");
    fs::create_dir_all(&source).unwrap();
    fs::write(
        source.join("SKILL.md"),
        "---\nname: source\ndescription: fixture\n---\n",
    )
    .unwrap();
    let home = fs::canonicalize(home).unwrap();
    let snapshot = NativeAdapters
        .discover(&home, &Snapshot::default())
        .merge_into(&Snapshot::default());
    let skill = SkillId::from_source_path(&fs::canonicalize(&source).unwrap());
    let target = TargetId {
        agent: AgentId::ClaudeCode,
        scope: Scope::Global,
    };
    let plan = domain::plan(
        &Command::SetSkillEnabled {
            skill_id: skill,
            targets: vec![target.clone()],
            enabled: false,
        },
        &snapshot,
    )
    .unwrap();
    let before = &snapshot.entries[&target];
    let states = BTreeMap::new();
    let mutation = |after| TargetMutation {
        receipt_context: None,
        target: &target,
        before: &snapshot.targets[&target],
        after: &plan.target_updates[&target],
        effects: &plan.effects,
        entries_before: before,
        entries_after: after,
        planned_target_states: &states,
    };
    let planned = NativeAdapters
        .prepare_target(mutation(before), &snapshot, &home)
        .unwrap();
    let config = home.join(".claude/settings.json");
    let foreign = b"{\"foreignWriter\":true}\n";
    fs::write(&config, foreign).unwrap();
    let result = NativeAdapters.apply_target(mutation(&planned), &snapshot, &home);
    assert!(result.is_err(), "changed native config was overwritten");
    assert_eq!(fs::read(&config).unwrap(), foreign);
    assert_eq!(
        fs::read_to_string(source.join("SKILL.md")).unwrap(),
        "---\nname: source\ndescription: fixture\n---\n"
    );
}

#[test]
fn recovery_accepts_actual_owned_fingerprint_but_not_replacement() {
    let fixture = adapter_fixture();
    let enable = domain::plan(
        &Command::SetSkillEnabled {
            skill_id: fixture.skill.clone(),
            targets: vec![fixture.target.clone()],
            enabled: true,
        },
        &fixture.snapshot,
    )
    .unwrap();
    let empty = BTreeMap::new();
    let states = BTreeMap::new();
    let mutation = |after| TargetMutation {
        receipt_context: None,
        target: &fixture.target,
        before: &fixture.snapshot.targets[&fixture.target],
        after: &enable.target_updates[&fixture.target],
        effects: &enable.effects,
        entries_before: &empty,
        entries_after: after,
        planned_target_states: &states,
    };
    let planned = NativeAdapters
        .prepare_target(mutation(&empty), &fixture.snapshot, &fixture.home)
        .unwrap();
    let actual = NativeAdapters
        .apply_target(mutation(&planned), &fixture.snapshot, &fixture.home)
        .unwrap();
    let repeated = NativeAdapters
        .apply_target(mutation(&actual), &fixture.snapshot, &fixture.home)
        .unwrap();
    assert_eq!(repeated, actual);
    let entry = &actual[&fixture.skill][0];
    let held = entry.path.with_extension("held");
    fs::rename(&entry.path, &held).unwrap();
    std::os::unix::fs::symlink(&entry.source_path, &entry.path).unwrap();
    let result = NativeAdapters.apply_target(mutation(&actual), &fixture.snapshot, &fixture.home);
    assert!(
        result.is_err(),
        "recovery accepted a different link identity"
    );
    assert!(held.is_symlink());
    assert!(entry.path.is_symlink());
}
