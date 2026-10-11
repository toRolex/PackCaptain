use std::collections::BTreeMap;
use std::fs;

use packcaptain::adapters::NativeAdapters;
use packcaptain::domain::{
    AgentId, Command, ProjectId, ProjectRecord, Scope, Snapshot, TargetId, TargetState,
};
use packcaptain::engine::{EffectExecutor, TargetMutation};
use tempfile::tempdir;

#[test]
fn direct_global_skill_uses_native_config_without_moving_source() {
    let temporary = tempdir().unwrap();
    let home_path = temporary.path().join("home");
    let direct_source = home_path.join(".claude/skills/direct-source");
    fs::create_dir_all(&direct_source).unwrap();
    fs::write(
        direct_source.join("SKILL.md"),
        "---\nname: direct-source\ndescription: fixture\n---\n",
    )
    .unwrap();
    let home = fs::canonicalize(home_path).unwrap();
    let source = fs::canonicalize(direct_source).unwrap();
    let source_contents = fs::read(source.join("SKILL.md")).unwrap();
    let adapter = NativeAdapters;
    let initial = adapter.discover(&home, &Snapshot::default());
    let mut snapshot = initial.merge_into(&Snapshot::default());
    let target = TargetId {
        agent: AgentId::ClaudeCode,
        scope: Scope::Global,
    };
    let skill_id = packcaptain::domain::SkillId::from_source_path(&source);
    assert!(snapshot.targets[&target].enabled_skills.contains(&skill_id));

    let mut adapter = NativeAdapters;
    let disable = packcaptain::domain::plan(
        &Command::SetSkillEnabled {
            skill_id: skill_id.clone(),
            targets: vec![target.clone()],
            enabled: false,
        },
        &snapshot,
    )
    .unwrap();
    let entries_before = snapshot.entries[&target].clone();
    let entries_after = adapter
        .prepare_target(
            TargetMutation {
                receipt_context: None,
                target: &target,
                before: &snapshot.targets[&target],
                after: &disable.target_updates[&target],
                effects: &disable.effects,
                entries_before: &entries_before,
                entries_after: &entries_before,
                planned_target_states: &BTreeMap::new(),
            },
            &snapshot,
            &home,
        )
        .unwrap();
    let disabled_entries = adapter
        .apply_target(
            TargetMutation {
                receipt_context: None,
                target: &target,
                before: &snapshot.targets[&target],
                after: &disable.target_updates[&target],
                effects: &disable.effects,
                entries_before: &entries_before,
                entries_after: &entries_after,
                planned_target_states: &BTreeMap::new(),
            },
            &snapshot,
            &home,
        )
        .unwrap();
    assert_eq!(fs::read(source.join("SKILL.md")).unwrap(), source_contents);
    assert_eq!(
        fs::symlink_metadata(&source).unwrap().file_type().is_dir(),
        true
    );

    snapshot
        .targets
        .insert(target.clone(), disable.target_updates[&target].clone());
    snapshot.entries.insert(target.clone(), disabled_entries);
    let fresh_disabled = adapter.discover(&home, &snapshot);
    assert!(!fresh_disabled.entries[&target][&skill_id][0].active);
    assert_eq!(fresh_disabled.skills[&skill_id].source_path, source);

    let enable = packcaptain::domain::plan(
        &Command::SetSkillEnabled {
            skill_id: skill_id.clone(),
            targets: vec![target.clone()],
            enabled: true,
        },
        &snapshot,
    )
    .unwrap();
    let entries_before = snapshot.entries[&target].clone();
    let entries_after = adapter
        .prepare_target(
            TargetMutation {
                receipt_context: None,
                target: &target,
                before: &snapshot.targets[&target],
                after: &enable.target_updates[&target],
                effects: &enable.effects,
                entries_before: &entries_before,
                entries_after: &entries_before,
                planned_target_states: &BTreeMap::new(),
            },
            &snapshot,
            &home,
        )
        .unwrap();
    let enabled_entries = adapter
        .apply_target(
            TargetMutation {
                receipt_context: None,
                target: &target,
                before: &snapshot.targets[&target],
                after: &enable.target_updates[&target],
                effects: &enable.effects,
                entries_before: &entries_before,
                entries_after: &entries_after,
                planned_target_states: &BTreeMap::new(),
            },
            &snapshot,
            &home,
        )
        .unwrap();
    snapshot
        .targets
        .insert(target.clone(), enable.target_updates[&target].clone());
    snapshot.entries.insert(target.clone(), enabled_entries);
    let fresh_enabled = adapter.discover(&home, &snapshot);
    assert!(fresh_enabled.entries[&target][&skill_id][0].active);
    assert_eq!(fs::read(source.join("SKILL.md")).unwrap(), source_contents);
}

#[test]
fn target_toggle_removes_only_owned_link_and_never_moves_source() {
    let temporary = tempdir().unwrap();
    let home_path = temporary.path().join("home");
    let source_dir = home_path.join(".agents/skills/shared-source");
    fs::create_dir_all(&source_dir).unwrap();
    fs::write(
        source_dir.join("SKILL.md"),
        "---\nname: shared-source\ndescription: fixture\n---\n",
    )
    .unwrap();
    let source_dir = fs::canonicalize(source_dir).unwrap();
    let home = fs::canonicalize(home_path).unwrap();
    let source_contents = fs::read(source_dir.join("SKILL.md")).unwrap();

    let project_id = ProjectId::from("project-1");
    let project_root = temporary.path().join("project");
    fs::create_dir_all(&project_root).unwrap();
    let project_root = fs::canonicalize(project_root).unwrap();
    let project = ProjectRecord {
        id: project_id.clone(),
        name: "project".to_owned(),
        root: project_root.clone(),
    };
    let mut adapter = NativeAdapters;
    let seed = Snapshot {
        projects: BTreeMap::from([(project_id.clone(), project)]),
        ..Snapshot::default()
    };
    let mut snapshot = adapter.discover(&home, &seed).merge_into(&seed);
    let skill_id = packcaptain::domain::SkillId::from_source_path(&source_dir);
    let target = TargetId {
        agent: AgentId::Codex,
        scope: Scope::Project(project_id),
    };
    snapshot
        .targets
        .insert(target.clone(), TargetState::default());
    let entries_before = BTreeMap::new();

    let enable = packcaptain::domain::plan(
        &Command::SetSkillEnabled {
            skill_id: skill_id.clone(),
            targets: vec![target.clone()],
            enabled: true,
        },
        &snapshot,
    )
    .unwrap();
    let entries_after = adapter
        .prepare_target(
            TargetMutation {
                receipt_context: None,
                target: &target,
                before: &snapshot.targets[&target],
                after: &enable.target_updates[&target],
                effects: &enable.effects,
                entries_before: &entries_before,
                entries_after: &entries_before,
                planned_target_states: &BTreeMap::new(),
            },
            &snapshot,
            &home,
        )
        .unwrap();
    let link_path = entries_after[&skill_id][0].path.clone();
    assert!(
        !link_path.exists(),
        "planning must not create a target entry"
    );

    let enabled_entries = adapter
        .apply_target(
            TargetMutation {
                receipt_context: None,
                target: &target,
                before: &snapshot.targets[&target],
                after: &enable.target_updates[&target],
                effects: &enable.effects,
                entries_before: &entries_before,
                entries_after: &entries_after,
                planned_target_states: &BTreeMap::new(),
            },
            &snapshot,
            &home,
        )
        .unwrap();
    assert!(
        fs::symlink_metadata(&link_path)
            .unwrap()
            .file_type()
            .is_symlink()
    );
    assert_eq!(fs::canonicalize(&link_path).unwrap(), source_dir);
    assert_eq!(
        fs::read(source_dir.join("SKILL.md")).unwrap(),
        source_contents
    );

    snapshot
        .targets
        .insert(target.clone(), enable.target_updates[&target].clone());
    snapshot
        .entries
        .insert(target.clone(), enabled_entries.clone());
    let disable = packcaptain::domain::plan(
        &Command::SetSkillEnabled {
            skill_id: skill_id.clone(),
            targets: vec![target.clone()],
            enabled: false,
        },
        &snapshot,
    )
    .unwrap();
    let entries_before = snapshot.entries[&target].clone();
    let entries_after = adapter
        .prepare_target(
            TargetMutation {
                receipt_context: None,
                target: &target,
                before: &snapshot.targets[&target],
                after: &disable.target_updates[&target],
                effects: &disable.effects,
                entries_before: &entries_before,
                entries_after: &entries_before,
                planned_target_states: &BTreeMap::new(),
            },
            &snapshot,
            &home,
        )
        .unwrap();
    let disabled_entries = adapter
        .apply_target(
            TargetMutation {
                receipt_context: None,
                target: &target,
                before: &snapshot.targets[&target],
                after: &disable.target_updates[&target],
                effects: &disable.effects,
                entries_before: &entries_before,
                entries_after: &entries_after,
                planned_target_states: &BTreeMap::new(),
            },
            &snapshot,
            &home,
        )
        .unwrap();

    assert!(!link_path.exists());
    assert!(source_dir.is_dir());
    assert_eq!(
        fs::read(source_dir.join("SKILL.md")).unwrap(),
        source_contents
    );
    assert!(!disabled_entries[&skill_id][0].active);
    assert_eq!(
        disabled_entries[&skill_id][0].ownership,
        packcaptain::domain::EntryOwnership::PackCaptainLink
    );

    snapshot
        .targets
        .insert(target.clone(), disable.target_updates[&target].clone());
    snapshot.entries.insert(target.clone(), disabled_entries);
    let enable_again = packcaptain::domain::plan(
        &Command::SetSkillEnabled {
            skill_id: skill_id.clone(),
            targets: vec![target.clone()],
            enabled: true,
        },
        &snapshot,
    )
    .unwrap();
    let entries_before = snapshot.entries[&target].clone();
    let entries_after = adapter
        .prepare_target(
            TargetMutation {
                receipt_context: None,
                target: &target,
                before: &snapshot.targets[&target],
                after: &enable_again.target_updates[&target],
                effects: &enable_again.effects,
                entries_before: &entries_before,
                entries_after: &entries_before,
                planned_target_states: &BTreeMap::new(),
            },
            &snapshot,
            &home,
        )
        .unwrap();
    let reopened = adapter
        .apply_target(
            TargetMutation {
                receipt_context: None,
                target: &target,
                before: &snapshot.targets[&target],
                after: &enable_again.target_updates[&target],
                effects: &enable_again.effects,
                entries_before: &entries_before,
                entries_after: &entries_after,
                planned_target_states: &BTreeMap::new(),
            },
            &snapshot,
            &home,
        )
        .unwrap();
    assert!(link_path.is_symlink());
    assert_eq!(fs::canonicalize(&link_path).unwrap(), source_dir);
    assert!(reopened[&skill_id][0].active);
}
