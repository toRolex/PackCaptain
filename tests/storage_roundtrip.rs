use std::collections::{BTreeMap, BTreeSet};

use packcaptain::domain::{
    AgentId, AppliedPresetState, PresetDefinition, PresetId, Scope, SkillAvailability, SkillId,
    SkillRecord, Snapshot, TargetId, TargetState,
};
use packcaptain::storage::StateStore;
use tempfile::tempdir;

fn fixture_snapshot() -> Snapshot {
    let skill_id = SkillId::from("source-id");
    let preset_id = PresetId::from("preset-id");
    let target_id = TargetId {
        agent: AgentId::ClaudeCode,
        scope: Scope::Project("project-id".into()),
    };
    Snapshot {
        skills: BTreeMap::from([(
            skill_id.clone(),
            SkillRecord {
                id: skill_id.clone(),
                name: "same-name".to_owned(),
                source_path: "/fixture/source".into(),
                source_fingerprint: "source-hash".to_owned(),
                availability: SkillAvailability::Available,
            },
        )]),
        presets: BTreeMap::from([(
            preset_id.clone(),
            PresetDefinition {
                id: preset_id.clone(),
                name: "Authors".to_owned(),
                revision: 3,
                members: BTreeSet::from([skill_id.clone()]),
            },
        )]),
        projects: BTreeMap::new(),
        entries: BTreeMap::new(),
        targets: BTreeMap::from([(
            target_id,
            TargetState {
                enabled_skills: BTreeSet::from([skill_id.clone()]),
                manual_open: BTreeSet::new(),
                manual_closed: BTreeSet::new(),
                presets: BTreeMap::from([(
                    preset_id,
                    AppliedPresetState {
                        enabled: true,
                        applied_revision: 3,
                        applied_members: BTreeSet::from([skill_id]),
                    },
                )]),
            },
        )]),
    }
}

#[test]
fn state_dir_isolated_snapshot_survives_store_reopen() {
    let temporary = tempdir().unwrap();
    let state_a = temporary.path().join("state-a");
    let state_b = temporary.path().join("state-b");
    let snapshot = fixture_snapshot();

    let mut store_a = StateStore::open(&state_a).unwrap();
    store_a.save_snapshot(&snapshot).unwrap();
    assert_eq!(store_a.load_snapshot().unwrap(), snapshot);
    assert_ne!(store_a.database_path(), state_b.join("packcaptain.sqlite3"));

    let store_b = StateStore::open(&state_b).unwrap();
    assert_eq!(store_b.load_snapshot().unwrap(), Snapshot::default());
    drop(store_a);

    let reopened = StateStore::open(&state_a).unwrap();
    assert_eq!(reopened.load_snapshot().unwrap(), snapshot);
}
