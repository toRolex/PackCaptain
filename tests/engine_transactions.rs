use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use packcaptain::domain::{
    AgentId, AppliedPresetState, Command, Effect, EntryOwnership, PresetDefinition, PresetId,
    Scope, SkillAvailability, SkillEntry, SkillId, SkillRecord, Snapshot, TargetId, TargetState,
};
use packcaptain::engine::{
    ApplyFailure, EffectExecutor, Engine, EngineError, TargetMutation, TargetOutcomeStatus,
};
use packcaptain::storage::StateStore;
use tempfile::tempdir;

#[derive(Default)]
struct FakeExecutor {
    enabled: BTreeMap<(TargetId, SkillId), bool>,
    fail_once_for: Option<SkillId>,
    entry_to_return: Option<(SkillId, SkillEntry)>,
}

impl EffectExecutor for FakeExecutor {
    fn prepare_target(
        &self,
        mutation: TargetMutation<'_>,
        _snapshot: &Snapshot,
        _agent_home: &Path,
    ) -> Result<BTreeMap<SkillId, Vec<SkillEntry>>, String> {
        Ok(mutation.entries_before.clone())
    }

    fn apply_target(
        &mut self,
        mutation: TargetMutation<'_>,
        _snapshot: &Snapshot,
        _agent_home: &Path,
    ) -> Result<BTreeMap<SkillId, Vec<SkillEntry>>, ApplyFailure> {
        for effect in mutation.effects {
            let Effect::SetSkillEnabled {
                target,
                skill,
                enabled,
            } = effect;
            let key = (target.clone(), skill.clone());
            let current = self.enabled.get(&key).copied().unwrap_or(false);
            if current == *enabled {
                continue;
            }
            let expected = mutation.before.enabled_skills.contains(skill);
            if current != expected {
                return Err("entry state changed outside this operation"
                    .to_owned()
                    .into());
            }
            if self.fail_once_for.as_ref() == Some(skill) {
                self.fail_once_for = None;
                let mut actual_entries = mutation.entries_before.clone();
                for (id, routes) in &mut actual_entries {
                    let active = self
                        .enabled
                        .get(&(mutation.target.clone(), id.clone()))
                        .copied()
                        .unwrap_or(false);
                    for route in routes {
                        route.active = active;
                        route.present = active;
                        if !active {
                            route.entry_fingerprint = "missing".into();
                        }
                    }
                }
                return Err(ApplyFailure {
                    message: "injected adapter failure".to_owned(),
                    actual_entries: Some(actual_entries),
                });
            }
            self.enabled.insert(key, *enabled);
        }
        let mut entries = mutation.entries_after.clone();
        if let Some((skill_id, entry)) = self.entry_to_return.take() {
            if mutation.effects.iter().any(|effect| {
                matches!(effect, Effect::SetSkillEnabled { skill, enabled: true, .. } if skill == &skill_id)
            }) {
                entries.insert(skill_id, vec![entry]);
            }
        }
        Ok(entries)
    }
}

fn global_target() -> TargetId {
    TargetId {
        agent: AgentId::Codex,
        scope: Scope::Global,
    }
}

fn enabled_preset_snapshot(members: &[&str]) -> Snapshot {
    let target = global_target();
    let preset_id = PresetId::from("bundle");
    let mut skills = BTreeMap::new();
    let mut skill_ids = BTreeSet::new();
    for member in members {
        let id = SkillId::from(*member);
        skill_ids.insert(id.clone());
        skills.insert(
            id.clone(),
            SkillRecord {
                id: id.clone(),
                name: id.as_str().to_owned(),
                source_path: format!("/fixture/source/{}", id.as_str()).into(),
                source_fingerprint: format!("fingerprint-{}", id.as_str()),
                availability: SkillAvailability::Available,
            },
        );
    }
    Snapshot {
        skills,
        presets: BTreeMap::from([(
            preset_id.clone(),
            PresetDefinition {
                id: preset_id.clone(),
                name: "Bundle".to_owned(),
                revision: 1,
                members: skill_ids.clone(),
            },
        )]),
        targets: BTreeMap::from([(
            target,
            TargetState {
                enabled_skills: skill_ids.clone(),
                presets: BTreeMap::from([(
                    preset_id,
                    AppliedPresetState {
                        enabled: true,
                        applied_revision: 1,
                        applied_members: skill_ids,
                    },
                )]),
                ..TargetState::default()
            },
        )]),
        ..Snapshot::default()
    }
}

#[test]
fn preview_does_not_create_state_dir_or_database() {
    let temporary = tempdir().unwrap();
    let state_dir = temporary.path().join("not-created");
    let engine = Engine::new(
        temporary.path().join("agent-home"),
        state_dir.clone(),
        FakeExecutor::default(),
    );
    let result = engine.preview(&Command::CreatePreset {
        preset_id: PresetId::from("preview-only"),
        name: "Preview".to_owned(),
        members: BTreeSet::new(),
    });

    assert!(
        result
            .unwrap()
            .preset_updates
            .contains_key(&PresetId::from("preview-only"))
    );
    assert!(!state_dir.exists());
}

#[test]
fn manual_disable_commits_database_and_native_effect() {
    let temporary = tempdir().unwrap();
    let state_dir = temporary.path().join("state");
    let mut store = StateStore::open(&state_dir).unwrap();
    let snapshot = enabled_preset_snapshot(&["solo"]);
    store.save_snapshot(&snapshot).unwrap();
    let skill = SkillId::from("solo");
    let target = global_target();
    let executor = FakeExecutor {
        enabled: BTreeMap::from([((target.clone(), skill.clone()), true)]),
        fail_once_for: None,
        entry_to_return: None,
    };
    let mut engine = Engine::new(
        temporary.path().join("agent-home"),
        state_dir.clone(),
        executor,
    );

    let report = engine
        .execute(&Command::SetSkillEnabled {
            skill_id: skill.clone(),
            targets: vec![target.clone()],
            enabled: false,
        })
        .unwrap();

    assert_eq!(report.targets.len(), 1);
    assert_eq!(report.targets[0].status, TargetOutcomeStatus::Applied);
    let stored = StateStore::open(&state_dir)
        .unwrap()
        .load_snapshot()
        .unwrap();
    assert!(!stored.targets[&target].enabled_skills.contains(&skill));
    assert!(stored.targets[&target].manual_closed.contains(&skill));
}

#[test]
fn adapter_failure_rolls_back_current_target_and_reports_failure() {
    let temporary = tempdir().unwrap();
    let state_dir = temporary.path().join("state");
    let mut store = StateStore::open(&state_dir).unwrap();
    let mut snapshot = enabled_preset_snapshot(&["a", "r"]);
    let target = global_target();
    let a = SkillId::from("a");
    let r = SkillId::from("r");
    let initial_entry = SkillEntry {
        path: "/fixture/target/a".into(),
        source_path: "/fixture/source/a".into(),
        entry_fingerprint: "before-fingerprint".to_owned(),
        present: true,
        active: true,
        ownership: EntryOwnership::PackCaptainLink,
        native_config_path: None,
        native_config_fingerprint: None,
    };
    let restored_entry = SkillEntry {
        entry_fingerprint: "restored-fingerprint".to_owned(),
        ..initial_entry.clone()
    };
    snapshot.entries.insert(
        target.clone(),
        BTreeMap::from([(a.clone(), vec![initial_entry]), (r.clone(), Vec::new())]),
    );
    store.save_snapshot(&snapshot).unwrap();
    let executor = FakeExecutor {
        enabled: BTreeMap::from([
            ((target.clone(), a.clone()), true),
            ((target.clone(), r.clone()), true),
        ]),
        fail_once_for: Some(r.clone()),
        entry_to_return: Some((a.clone(), restored_entry)),
    };
    let mut engine = Engine::new(
        temporary.path().join("agent-home"),
        state_dir.clone(),
        executor,
    );

    let report = engine
        .execute(&Command::SetPresetEnabled {
            preset_id: PresetId::from("bundle"),
            targets: vec![target.clone()],
            enabled: false,
        })
        .unwrap();

    assert_eq!(report.targets.len(), 1);
    assert_eq!(report.targets[0].status, TargetOutcomeStatus::Failed);
    let stored = StateStore::open(&state_dir)
        .unwrap()
        .load_snapshot()
        .unwrap();
    assert!(stored.targets[&target].presets[&PresetId::from("bundle")].enabled);
    assert_eq!(
        stored.targets[&target].enabled_skills,
        BTreeSet::from([a.clone(), r])
    );
    assert_eq!(
        stored.entries[&target][&a][0].entry_fingerprint,
        "restored-fingerprint"
    );
}

#[test]
fn target_entry_metadata_commits_with_enabled_state() {
    let temporary = tempdir().unwrap();
    let state_dir = temporary.path().join("state");
    let mut store = StateStore::open(&state_dir).unwrap();
    let skill = SkillId::from("source-id");
    let target = global_target();
    let source = std::path::PathBuf::from("/fixture/source");
    let entry = SkillEntry {
        path: "/fixture/target/skills/source-id".into(),
        source_path: source.clone(),
        entry_fingerprint: "link-fingerprint".to_owned(),
        present: true,
        active: true,
        ownership: EntryOwnership::PackCaptainLink,
        native_config_path: None,
        native_config_fingerprint: None,
    };
    let snapshot = Snapshot {
        skills: BTreeMap::from([(
            skill.clone(),
            SkillRecord {
                id: skill.clone(),
                name: "source".to_owned(),
                source_path: source,
                source_fingerprint: "source-fingerprint".to_owned(),
                availability: SkillAvailability::Available,
            },
        )]),
        targets: BTreeMap::from([(target.clone(), TargetState::default())]),
        ..Snapshot::default()
    };
    store.save_snapshot(&snapshot).unwrap();
    let executor = FakeExecutor {
        enabled: BTreeMap::new(),
        fail_once_for: None,
        entry_to_return: Some((skill.clone(), entry.clone())),
    };
    let mut engine = Engine::new(
        temporary.path().join("agent-home"),
        state_dir.clone(),
        executor,
    );

    let report = engine
        .execute(&Command::SetSkillEnabled {
            skill_id: skill.clone(),
            targets: vec![target.clone()],
            enabled: true,
        })
        .unwrap();

    assert_eq!(report.targets[0].status, TargetOutcomeStatus::Applied);
    let stored = StateStore::open(&state_dir)
        .unwrap()
        .load_snapshot()
        .unwrap();
    assert!(stored.targets[&target].enabled_skills.contains(&skill));
    assert_eq!(stored.entries[&target][&skill], vec![entry]);
}

#[test]
fn conflict_preflight_does_not_create_a_lock_or_write_state() {
    let temporary = tempdir().unwrap();
    let state_dir = temporary.path().join("state");
    let mut store = StateStore::open(&state_dir).unwrap();
    let mut snapshot = enabled_preset_snapshot(&["a"]);
    let target = global_target();
    let a = SkillId::from("a");
    snapshot
        .targets
        .get_mut(&target)
        .unwrap()
        .manual_closed
        .insert(a.clone());
    snapshot
        .targets
        .get_mut(&target)
        .unwrap()
        .enabled_skills
        .remove(&a);
    store.save_snapshot(&snapshot).unwrap();
    let mut engine = Engine::new(
        temporary.path().join("agent-home"),
        state_dir.clone(),
        FakeExecutor::default(),
    );

    let error = engine
        .execute(&Command::UpdatePreset {
            preset_id: PresetId::from("bundle"),
            expected_revision: 1,
            members: BTreeSet::from([a.clone()]),
            sync_targets: vec![target.clone()],
            closed_members: packcaptain::domain::ClosedMembersPolicy::Unspecified,
        })
        .unwrap_err();

    assert!(matches!(
        error,
        EngineError::Conflict(ref conflict) if conflict.code == "closed-members-policy-required"
    ));
    assert!(!state_dir.join("packcaptain.lock").exists());
    assert_eq!(
        StateStore::open(&state_dir)
            .unwrap()
            .load_snapshot()
            .unwrap(),
        snapshot
    );
}
