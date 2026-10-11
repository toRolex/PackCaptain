use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use packcaptain::adapters::NativeAdapters;
use packcaptain::domain::{
    AgentId, EntryOwnership, ProjectId, ProjectRecord, Scope, Snapshot, TargetId,
};
use tempfile::tempdir;

fn write_skill(directory: &Path, name: &str, body: &str) {
    fs::create_dir_all(directory).unwrap();
    fs::write(
        directory.join("SKILL.md"),
        format!("---\nname: {name}\ndescription: fixture\n---\n\n{body}\n"),
    )
    .unwrap();
}

#[cfg(unix)]
#[test]
fn discovery_keeps_canonical_source_identity_and_separate_target_entry_paths() {
    use std::os::unix::fs::symlink;

    let temporary = tempdir().unwrap();
    let home = temporary.path().join("home");
    let global_source = home.join(".agents/skills/shared-source");
    write_skill(&global_source, "shared-name", "shared content");
    let project_root = temporary.path().join("workspace");
    let project_alias = project_root.join(".agents/skills/project-alias");
    fs::create_dir_all(project_alias.parent().unwrap()).unwrap();
    symlink(&global_source, &project_alias).unwrap();
    let project_skill = project_root.join(".agents/skills/project-only");
    write_skill(&project_skill, "shared-name", "different content");

    let project_id = ProjectId::from("project-1");
    let snapshot = Snapshot {
        projects: BTreeMap::from([(
            project_id.clone(),
            ProjectRecord {
                id: project_id.clone(),
                name: "workspace".to_owned(),
                root: project_root.clone(),
            },
        )]),
        ..Snapshot::default()
    };
    let inventory = NativeAdapters.discover(&home, &snapshot);
    let global_target = TargetId {
        agent: AgentId::Codex,
        scope: Scope::Global,
    };
    let project_target = TargetId {
        agent: AgentId::Codex,
        scope: Scope::Project(project_id),
    };
    let shared_id =
        packcaptain::domain::SkillId::from_source_path(&fs::canonicalize(&global_source).unwrap());
    let project_id =
        packcaptain::domain::SkillId::from_source_path(&fs::canonicalize(&project_skill).unwrap());

    assert_eq!(inventory.skills[&shared_id].name, "shared-name");
    assert_eq!(inventory.skills[&project_id].name, "shared-name");
    assert_ne!(
        shared_id, project_id,
        "same names from different source dirs remain distinct"
    );
    assert_eq!(
        inventory.entries[&global_target][&shared_id][0].ownership,
        EntryOwnership::DirectDirectory
    );
    let project_entry = &inventory.entries[&project_target][&shared_id][0];
    assert_eq!(project_entry.path, project_alias);
    assert_eq!(
        project_entry.source_path,
        fs::canonicalize(global_source).unwrap()
    );
    assert_eq!(project_entry.ownership, EntryOwnership::UnmanagedSymlink);
    assert_eq!(inventory.skills[&shared_id].source_fingerprint.len(), 64);
}

#[test]
fn discovery_reads_user_native_disabled_state_without_losing_the_source() {
    let temporary = tempdir().unwrap();
    let home = temporary.path().join("home");
    let skill_root = home.join(".claude/skills/disabled-direct");
    write_skill(&skill_root, "disabled-fixture", "source still exists");
    let settings = home.join(".claude/settings.json");
    fs::create_dir_all(settings.parent().unwrap()).unwrap();
    fs::write(
        &settings,
        r#"{"skillOverrides":{"disabled-fixture":"off"}}"#,
    )
    .unwrap();

    let inventory = NativeAdapters.discover(&home, &Snapshot::default());
    let target = TargetId {
        agent: AgentId::ClaudeCode,
        scope: Scope::Global,
    };
    let skill_id =
        packcaptain::domain::SkillId::from_source_path(&fs::canonicalize(&skill_root).unwrap());

    assert_eq!(
        inventory.skills[&skill_id].availability,
        packcaptain::domain::SkillAvailability::Available
    );
    assert!(inventory.entries[&target][&skill_id][0].path.exists());
    assert!(!inventory.entries[&target][&skill_id][0].active);
    assert!(
        !inventory
            .observed_enabled
            .get(&target)
            .is_some_and(|skills| skills.contains(&skill_id))
    );
}

#[test]
fn source_fingerprint_changes_when_skill_contents_change() {
    let temporary = tempdir().unwrap();
    let home = temporary.path().join("home");
    let skill_root = home.join(".agents/skills/versioned");
    write_skill(&skill_root, "versioned", "before");
    let target = TargetId {
        agent: AgentId::Codex,
        scope: Scope::Global,
    };
    let adapter = NativeAdapters;
    let before = adapter.discover(&home, &Snapshot::default());
    let skill_id =
        packcaptain::domain::SkillId::from_source_path(&fs::canonicalize(&skill_root).unwrap());
    let before_fingerprint = before.skills[&skill_id].source_fingerprint.clone();

    fs::write(
        skill_root.join("SKILL.md"),
        "---\nname: versioned\ndescription: fixture\n---\n\nafter\n",
    )
    .unwrap();
    let after = adapter.discover(&home, &Snapshot::default());

    assert_ne!(
        before_fingerprint,
        after.skills[&skill_id].source_fingerprint
    );
    assert!(after.observed_enabled[&target].contains(&skill_id));
}
