use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::str::FromStr;

use packcaptain::domain::{
    AgentId, AppliedPresetState, ClosedMembersPolicy, Command, Effect, PresetDefinition, PresetId,
    Scope, SkillAvailability, SkillId, SkillRecord, Snapshot, TargetId, TargetState, plan,
};

fn skill(id: &str) -> (SkillId, SkillRecord) {
    let id = SkillId::from(id);
    let record = SkillRecord {
        id: id.clone(),
        name: id.as_str().to_owned(),
        source_path: format!("/fixtures/skills/{}", id.as_str()).into(),
        source_fingerprint: format!("fingerprint-{}", id.as_str()),
        availability: SkillAvailability::Available,
    };
    (id, record)
}

fn preset(id: &str, members: &[&SkillId], revision: u64) -> (PresetId, PresetDefinition) {
    let id = PresetId::from(id);
    let definition = PresetDefinition {
        id: id.clone(),
        name: id.as_str().to_owned(),
        revision,
        members: members.iter().map(|member| (*member).clone()).collect(),
    };
    (id, definition)
}

fn target(agent: AgentId, name: &str) -> TargetId {
    TargetId {
        agent,
        scope: Scope::Project(name.into()),
    }
}

#[test]
fn target_ids_have_stable_json_map_keys() {
    let target_id = target(AgentId::Pi, "project-1");
    assert_eq!(target_id.to_string(), "pi:project:project-1");
    assert_eq!(
        TargetId::from_str("pi:project:project-1").unwrap(),
        target_id
    );

    let states = BTreeMap::from([(target_id, TargetState::default())]);
    let encoded = serde_json::to_value(&states).unwrap();
    assert!(encoded.get("pi:project:project-1").is_some());
    let decoded: BTreeMap<TargetId, TargetState> = serde_json::from_value(encoded).unwrap();
    assert_eq!(decoded.len(), 1);

    let snapshot = Snapshot {
        targets: states,
        ..Snapshot::default()
    };
    let encoded = serde_json::to_value(&snapshot).unwrap();
    assert!(
        encoded
            .get("targets")
            .and_then(|targets| targets.get("pi:project:project-1"))
            .is_some()
    );
    assert_eq!(
        serde_json::from_value::<Snapshot>(encoded).unwrap(),
        snapshot
    );
}

#[test]
fn skill_ids_follow_canonical_source_path_not_display_name() {
    let project_source = SkillId::from_source_path(Path::new("/fixtures/project/skills/tool"));
    let user_source = SkillId::from_source_path(Path::new("/fixtures/user/skills/tool"));
    let project_source_again =
        SkillId::from_source_path(Path::new("/fixtures/project/skills/tool"));

    assert_ne!(project_source, user_source);
    assert_eq!(project_source, project_source_again);
}

#[test]
fn create_preset_keeps_distinct_skill_ids_for_duplicate_names() {
    let (first_id, mut first) = skill("source-project");
    let (second_id, mut second) = skill("source-user");
    first.name = "shared-name".to_owned();
    second.name = "shared-name".to_owned();
    let snapshot = Snapshot {
        skills: BTreeMap::from([(first_id.clone(), first), (second_id.clone(), second)]),
        ..Snapshot::default()
    };

    let preset_id = PresetId::from("authors");
    let members = BTreeSet::from([first_id.clone(), second_id.clone()]);
    let plan = plan(
        &Command::CreatePreset {
            preset_id: preset_id.clone(),
            name: "Authors".to_owned(),
            members: members.clone(),
        },
        &snapshot,
    )
    .expect("a preset can select same-name skills from separate sources");

    assert_eq!(
        plan.preset_updates[&preset_id],
        PresetDefinition {
            id: preset_id.clone(),
            name: "Authors".to_owned(),
            revision: 1,
            members,
        }
    );
    assert!(plan.effects.is_empty());
    assert!(plan.target_updates.is_empty());
    assert!(snapshot.presets.is_empty());
}

#[test]
fn manual_skill_disable_records_closure_per_target() {
    let (skill_id, skill_record) = skill("solo");
    let target_id = target(AgentId::Codex, "project-1");
    let snapshot = Snapshot {
        skills: BTreeMap::from([(skill_id.clone(), skill_record)]),
        targets: BTreeMap::from([(
            target_id.clone(),
            TargetState {
                enabled_skills: BTreeSet::from([skill_id.clone()]),
                ..TargetState::default()
            },
        )]),
        ..Snapshot::default()
    };

    let plan = plan(
        &Command::SetSkillEnabled {
            skill_id: skill_id.clone(),
            targets: vec![target_id.clone()],
            enabled: false,
        },
        &snapshot,
    )
    .expect("a discovered skill can be manually disabled");

    assert_eq!(
        plan.effects,
        vec![Effect::SetSkillEnabled {
            target: target_id.clone(),
            skill: skill_id.clone(),
            enabled: false,
        }]
    );
    assert!(
        !plan.target_updates[&target_id]
            .enabled_skills
            .contains(&skill_id)
    );
    assert!(
        plan.target_updates[&target_id]
            .manual_closed
            .contains(&skill_id)
    );
    assert!(
        snapshot.targets[&target_id]
            .enabled_skills
            .contains(&skill_id)
    );

    let mut after = snapshot.clone();
    after
        .targets
        .insert(target_id.clone(), plan.target_updates[&target_id].clone());
    let retry = packcaptain::domain::plan(
        &Command::SetSkillEnabled {
            skill_id: skill_id.clone(),
            targets: vec![target_id.clone()],
            enabled: false,
        },
        &after,
    )
    .expect("retrying an explicit disable is valid");
    assert!(retry.effects.is_empty());
    assert_eq!(retry.target_updates[&target_id], after.targets[&target_id]);
}

#[test]
fn write_command_requires_at_least_one_explicit_target() {
    let (skill_id, skill_record) = skill("solo");
    let snapshot = Snapshot {
        skills: BTreeMap::from([(skill_id.clone(), skill_record)]),
        ..Snapshot::default()
    };

    let conflict = plan(
        &Command::SetSkillEnabled {
            skill_id: skill_id.clone(),
            targets: Vec::new(),
            enabled: true,
        },
        &snapshot,
    )
    .expect_err("write commands must not default to all targets");

    assert_eq!(conflict.code, "targets-required");
    assert!(conflict.affected_targets.is_empty());
    assert_eq!(conflict.affected_skills, vec![skill_id]);
}

#[test]
fn explicit_preset_enable_reopens_previously_manually_closed_member() {
    let (a, skill_a) = skill("a");
    let (b, skill_b) = skill("b");
    let (preset_id, definition) = preset("A", &[&a, &b], 2);
    let target_id = target(AgentId::ClaudeCode, "project-1");
    let snapshot = Snapshot {
        skills: BTreeMap::from([(a.clone(), skill_a), (b.clone(), skill_b)]),
        presets: BTreeMap::from([(preset_id.clone(), definition)]),
        projects: BTreeMap::new(),
        entries: BTreeMap::new(),
        targets: BTreeMap::from([(
            target_id.clone(),
            TargetState {
                enabled_skills: BTreeSet::from([b.clone()]),
                manual_open: BTreeSet::new(),
                manual_closed: BTreeSet::from([a.clone()]),
                presets: BTreeMap::from([(
                    preset_id.clone(),
                    AppliedPresetState {
                        enabled: true,
                        applied_revision: 1,
                        applied_members: BTreeSet::from([a.clone(), b.clone()]),
                    },
                )]),
            },
        )]),
    };

    let plan = plan(
        &Command::SetPresetEnabled {
            preset_id: preset_id.clone(),
            targets: vec![target_id.clone()],
            enabled: true,
        },
        &snapshot,
    )
    .expect("explicitly enabling a preset is a valid plan");

    assert_eq!(
        plan.effects,
        vec![Effect::SetSkillEnabled {
            target: target_id.clone(),
            skill: a.clone(),
            enabled: true,
        }]
    );
    let next = &plan.target_updates[&target_id];
    assert_eq!(next.enabled_skills, BTreeSet::from([a.clone(), b.clone()]));
    assert!(!next.manual_closed.contains(&a));
    assert!(snapshot.targets[&target_id].manual_closed.contains(&a));
}

struct SyncFixture {
    snapshot: Snapshot,
    preset_a: PresetId,
    preset_b: PresetId,
    a: SkillId,
    s: SkillId,
    r: SkillId,
    b: SkillId,
    n: SkillId,
    t1: TargetId,
    t2: TargetId,
    t3: TargetId,
}

fn sync_fixture() -> SyncFixture {
    let (a, skill_a) = skill("a");
    let (s, skill_s) = skill("s");
    let (r, skill_r) = skill("r");
    let (b, skill_b) = skill("b");
    let (n, skill_n) = skill("n");
    let (preset_a, definition_a) = preset("A", &[&a, &s, &r], 1);
    let (preset_b, definition_b) = preset("B", &[&s, &b], 1);
    let t1 = target(AgentId::ClaudeCode, "project-1");
    let t2 = target(AgentId::Codex, "project-1");
    let t3 = target(AgentId::Pi, "project-1");
    let state = |manual_closed: BTreeSet<SkillId>| TargetState {
        enabled_skills: BTreeSet::from([a.clone(), s.clone(), r.clone(), b.clone()])
            .difference(&manual_closed)
            .cloned()
            .collect(),
        manual_open: BTreeSet::new(),
        manual_closed,
        presets: BTreeMap::from([
            (
                preset_a.clone(),
                AppliedPresetState {
                    enabled: true,
                    applied_revision: 1,
                    applied_members: BTreeSet::from([a.clone(), s.clone(), r.clone()]),
                },
            ),
            (
                preset_b.clone(),
                AppliedPresetState {
                    enabled: true,
                    applied_revision: 1,
                    applied_members: BTreeSet::from([s.clone(), b.clone()]),
                },
            ),
        ]),
    };
    SyncFixture {
        snapshot: Snapshot {
            skills: BTreeMap::from([
                (a.clone(), skill_a),
                (s.clone(), skill_s),
                (r.clone(), skill_r),
                (b.clone(), skill_b),
                (n.clone(), skill_n),
            ]),
            presets: BTreeMap::from([
                (preset_a.clone(), definition_a),
                (preset_b.clone(), definition_b),
            ]),
            projects: BTreeMap::new(),
            entries: BTreeMap::new(),
            targets: BTreeMap::from([
                (t1.clone(), state(BTreeSet::from([a.clone()]))),
                (t2.clone(), state(BTreeSet::new())),
                (
                    t3.clone(),
                    TargetState {
                        presets: BTreeMap::from([(
                            preset_a.clone(),
                            AppliedPresetState {
                                enabled: false,
                                applied_revision: 1,
                                applied_members: BTreeSet::from([a.clone(), s.clone(), r.clone()]),
                            },
                        )]),
                        ..TargetState::default()
                    },
                ),
            ]),
        },
        preset_a,
        preset_b,
        a,
        s,
        r,
        b,
        n,
        t1,
        t2,
        t3,
    }
}

#[test]
fn keep_sync_enables_new_member_and_changes_only_selected_target() {
    let mut fixture = sync_fixture();
    fixture
        .snapshot
        .targets
        .get_mut(&fixture.t1)
        .unwrap()
        .manual_closed
        .insert(fixture.n.clone());
    let old_t2 = fixture.snapshot.targets[&fixture.t2].clone();
    let old_t3 = fixture.snapshot.targets[&fixture.t3].clone();
    let new_members = BTreeSet::from([fixture.a.clone(), fixture.s.clone(), fixture.n.clone()]);

    let plan = plan(
        &Command::UpdatePreset {
            preset_id: fixture.preset_a.clone(),
            expected_revision: 1,
            members: new_members.clone(),
            sync_targets: vec![fixture.t1.clone()],
            closed_members: ClosedMembersPolicy::Keep,
        },
        &fixture.snapshot,
    )
    .expect("Keep resolves the selected target's manual closure");

    assert_eq!(
        plan.effects,
        vec![
            Effect::SetSkillEnabled {
                target: fixture.t1.clone(),
                skill: fixture.n.clone(),
                enabled: true,
            },
            Effect::SetSkillEnabled {
                target: fixture.t1.clone(),
                skill: fixture.r.clone(),
                enabled: false,
            },
        ]
    );
    assert_eq!(plan.preset_updates[&fixture.preset_a].revision, 2);
    assert_eq!(plan.preset_updates[&fixture.preset_a].members, new_members);
    let updated = &plan.target_updates[&fixture.t1];
    assert!(updated.presets[&fixture.preset_a].enabled);
    assert_eq!(updated.presets[&fixture.preset_a].applied_revision, 2);
    assert_eq!(
        updated.presets[&fixture.preset_a].applied_members,
        BTreeSet::from([fixture.a.clone(), fixture.s.clone(), fixture.n.clone()])
    );
    assert!(updated.manual_closed.contains(&fixture.a));
    assert!(!updated.manual_closed.contains(&fixture.n));
    assert_eq!(
        updated.enabled_skills,
        BTreeSet::from([fixture.s.clone(), fixture.b.clone(), fixture.n.clone()])
    );
    assert_eq!(plan.target_updates.len(), 1);
    assert_eq!(fixture.snapshot.targets[&fixture.t2], old_t2);
    assert_eq!(fixture.snapshot.targets[&fixture.t3], old_t3);
    assert!(
        fixture.snapshot.presets[&fixture.preset_a]
            .members
            .contains(&fixture.r)
    );
}

#[test]
fn restore_reopens_only_retained_old_closure_and_new_member() {
    let mut fixture = sync_fixture();
    fixture
        .snapshot
        .targets
        .get_mut(&fixture.t1)
        .unwrap()
        .manual_closed = BTreeSet::from([fixture.a.clone(), fixture.r.clone(), fixture.b.clone()]);
    fixture
        .snapshot
        .targets
        .get_mut(&fixture.t1)
        .unwrap()
        .enabled_skills = BTreeSet::from([fixture.s.clone()]);

    let plan = plan(
        &Command::UpdatePreset {
            preset_id: fixture.preset_a.clone(),
            expected_revision: 1,
            members: BTreeSet::from([fixture.a.clone(), fixture.s.clone(), fixture.n.clone()]),
            sync_targets: vec![fixture.t1.clone()],
            closed_members: ClosedMembersPolicy::Restore,
        },
        &fixture.snapshot,
    )
    .expect("Restore resolves the selected target's manual closure");

    let updated = &plan.target_updates[&fixture.t1];
    assert_eq!(
        updated.enabled_skills,
        BTreeSet::from([fixture.a.clone(), fixture.s.clone(), fixture.n.clone()])
    );
    assert_eq!(
        updated.manual_closed,
        BTreeSet::from([fixture.r.clone(), fixture.b.clone()])
    );
    assert!(updated.presets[&fixture.preset_b].enabled);
    assert!(!updated.enabled_skills.contains(&fixture.b));
    assert!(!updated.enabled_skills.contains(&fixture.r));
}

fn record_plan(snapshot: &mut Snapshot, plan: &packcaptain::domain::Plan) {
    for (preset_id, definition) in &plan.preset_updates {
        snapshot
            .presets
            .insert(preset_id.clone(), definition.clone());
    }
    for (target_id, state) in &plan.target_updates {
        snapshot.targets.insert(target_id.clone(), state.clone());
    }
}

#[test]
fn syncing_same_definition_to_another_target_uses_its_old_applied_members() {
    let fixture = sync_fixture();
    let members_a2 = BTreeSet::from([fixture.a.clone(), fixture.s.clone(), fixture.n.clone()]);
    let mut snapshot = fixture.snapshot.clone();

    let first = plan(
        &Command::UpdatePreset {
            preset_id: fixture.preset_a.clone(),
            expected_revision: 1,
            members: members_a2.clone(),
            sync_targets: vec![fixture.t1.clone()],
            closed_members: ClosedMembersPolicy::Keep,
        },
        &snapshot,
    )
    .expect("first target sync is valid");
    record_plan(&mut snapshot, &first);
    let t1_after_first = snapshot.targets[&fixture.t1].clone();
    assert_eq!(snapshot.presets[&fixture.preset_a].revision, 2);

    let second = plan(
        &Command::UpdatePreset {
            preset_id: fixture.preset_a.clone(),
            expected_revision: 2,
            members: members_a2.clone(),
            sync_targets: vec![fixture.t2.clone()],
            closed_members: ClosedMembersPolicy::Unspecified,
        },
        &snapshot,
    )
    .expect("same definition must sync the second target's own snapshot");

    assert_eq!(
        second.effects,
        vec![
            Effect::SetSkillEnabled {
                target: fixture.t2.clone(),
                skill: fixture.n.clone(),
                enabled: true,
            },
            Effect::SetSkillEnabled {
                target: fixture.t2.clone(),
                skill: fixture.r.clone(),
                enabled: false,
            },
        ]
    );
    assert_eq!(second.preset_updates[&fixture.preset_a].revision, 2);
    assert_eq!(
        second.target_updates[&fixture.t2].enabled_skills,
        BTreeSet::from([
            fixture.a.clone(),
            fixture.s.clone(),
            fixture.b.clone(),
            fixture.n.clone()
        ])
    );
    assert_eq!(
        second.target_updates[&fixture.t2].presets[&fixture.preset_a].applied_members,
        members_a2
    );
    assert_eq!(snapshot.targets[&fixture.t1], t1_after_first);

    record_plan(&mut snapshot, &second);
    assert_eq!(snapshot.targets[&fixture.t1], t1_after_first);
    let retry = plan(
        &Command::UpdatePreset {
            preset_id: fixture.preset_a.clone(),
            expected_revision: 2,
            members: BTreeSet::from([fixture.a.clone(), fixture.s.clone(), fixture.n.clone()]),
            sync_targets: vec![fixture.t2.clone()],
            closed_members: ClosedMembersPolicy::Unspecified,
        },
        &snapshot,
    )
    .expect("retrying the same target sync is valid");
    assert!(retry.effects.is_empty());
    assert_eq!(retry.preset_updates[&fixture.preset_a].revision, 2);
}

#[test]
fn missing_closed_members_policy_returns_impact_without_plan_mutation() {
    let fixture = sync_fixture();
    let original = fixture.snapshot.clone();
    let result = plan(
        &Command::UpdatePreset {
            preset_id: fixture.preset_a.clone(),
            expected_revision: 1,
            members: BTreeSet::from([fixture.a.clone(), fixture.s.clone(), fixture.n.clone()]),
            sync_targets: vec![fixture.t1.clone()],
            closed_members: ClosedMembersPolicy::Unspecified,
        },
        &fixture.snapshot,
    );

    let conflict = result.expect_err("a relevant closed member requires a policy");
    assert_eq!(conflict.code, "closed-members-policy-required");
    assert_eq!(conflict.affected_targets, vec![fixture.t1.clone()]);
    assert_eq!(conflict.affected_skills, vec![fixture.a.clone()]);
    assert_eq!(fixture.snapshot, original);
}

#[test]
fn unavailable_retained_member_is_not_a_closed_members_conflict() {
    let mut fixture = sync_fixture();
    fixture
        .snapshot
        .skills
        .get_mut(&fixture.s)
        .unwrap()
        .availability = SkillAvailability::Missing;
    fixture
        .snapshot
        .targets
        .get_mut(&fixture.t1)
        .unwrap()
        .manual_closed = BTreeSet::from([fixture.s.clone()]);
    fixture
        .snapshot
        .targets
        .get_mut(&fixture.t1)
        .unwrap()
        .enabled_skills
        .remove(&fixture.s);

    let plan = plan(
        &Command::UpdatePreset {
            preset_id: fixture.preset_a.clone(),
            expected_revision: 1,
            members: BTreeSet::from([fixture.a.clone(), fixture.s.clone(), fixture.n.clone()]),
            sync_targets: vec![fixture.t1.clone()],
            closed_members: ClosedMembersPolicy::Unspecified,
        },
        &fixture.snapshot,
    )
    .expect("missing skills do not require a manual-closure choice");

    let updated = &plan.target_updates[&fixture.t1];
    assert!(updated.manual_closed.contains(&fixture.s));
    assert!(!updated.enabled_skills.contains(&fixture.s));
    assert_eq!(
        updated.presets[&fixture.preset_a].applied_members,
        BTreeSet::from([fixture.a.clone(), fixture.s.clone(), fixture.n.clone()])
    );
}

#[test]
fn unavailable_closed_member_remains_retained_when_it_becomes_available_again() {
    let mut fixture = sync_fixture();
    fixture
        .snapshot
        .skills
        .get_mut(&fixture.s)
        .unwrap()
        .availability = SkillAvailability::Missing;
    fixture
        .snapshot
        .targets
        .get_mut(&fixture.t1)
        .unwrap()
        .manual_closed = BTreeSet::from([fixture.s.clone()]);
    fixture
        .snapshot
        .targets
        .get_mut(&fixture.t1)
        .unwrap()
        .enabled_skills = BTreeSet::from([fixture.a.clone(), fixture.r.clone(), fixture.b.clone()]);

    let members_a2 = BTreeSet::from([fixture.a.clone(), fixture.s.clone(), fixture.n.clone()]);
    let first = plan(
        &Command::UpdatePreset {
            preset_id: fixture.preset_a.clone(),
            expected_revision: 1,
            members: members_a2.clone(),
            sync_targets: vec![fixture.t1.clone()],
            closed_members: ClosedMembersPolicy::Unspecified,
        },
        &fixture.snapshot,
    )
    .expect("temporarily missing members are not closure conflicts");
    assert_eq!(
        first.target_updates[&fixture.t1].presets[&fixture.preset_a].applied_members,
        members_a2
    );

    let mut after_first = fixture.snapshot.clone();
    record_plan(&mut after_first, &first);
    after_first.skills.get_mut(&fixture.s).unwrap().availability = SkillAvailability::Available;
    let conflict = plan(
        &Command::UpdatePreset {
            preset_id: fixture.preset_a.clone(),
            expected_revision: 2,
            members: BTreeSet::from([fixture.a.clone(), fixture.s.clone(), fixture.n.clone()]),
            sync_targets: vec![fixture.t1.clone()],
            closed_members: ClosedMembersPolicy::Unspecified,
        },
        &after_first,
    )
    .expect_err("the old explicit closure remains relevant after the source returns");

    assert_eq!(conflict.code, "closed-members-policy-required");
    assert_eq!(conflict.affected_skills, vec![fixture.s.clone()]);
}

#[test]
fn preset_disable_clears_its_same_target_manual_open_override() {
    let fixture = sync_fixture();
    let manual_open = plan(
        &Command::SetSkillEnabled {
            skill_id: fixture.a.clone(),
            targets: vec![fixture.t1.clone()],
            enabled: true,
        },
        &fixture.snapshot,
    )
    .unwrap();
    let mut snapshot = fixture.snapshot.clone();
    record_plan(&mut snapshot, &manual_open);
    assert!(
        snapshot.targets[&fixture.t1]
            .manual_open
            .contains(&fixture.a)
    );

    let disabled = plan(
        &Command::SetPresetEnabled {
            preset_id: fixture.preset_a.clone(),
            targets: vec![fixture.t1.clone()],
            enabled: false,
        },
        &snapshot,
    )
    .unwrap();

    assert_eq!(
        disabled.effects,
        vec![
            Effect::SetSkillEnabled {
                target: fixture.t1.clone(),
                skill: fixture.a.clone(),
                enabled: false,
            },
            Effect::SetSkillEnabled {
                target: fixture.t1.clone(),
                skill: fixture.r.clone(),
                enabled: false,
            },
        ]
    );
    let next = &disabled.target_updates[&fixture.t1];
    assert_eq!(
        next.enabled_skills,
        BTreeSet::from([fixture.s.clone(), fixture.b.clone()])
    );
    assert!(!next.manual_open.contains(&fixture.a));
    assert!(next.presets[&fixture.preset_b].enabled);
}

#[test]
fn re_enabling_updated_preset_removes_members_from_its_old_applied_snapshot() {
    let (a, skill_a) = skill("a");
    let (n, skill_n) = skill("n");
    let (r, skill_r) = skill("r");
    let preset_id = PresetId::from("A");
    let target_id = target(AgentId::Codex, "project-2");
    let current_members = BTreeSet::from([a.clone(), n.clone()]);
    let snapshot = Snapshot {
        skills: BTreeMap::from([
            (a.clone(), skill_a),
            (n.clone(), skill_n),
            (r.clone(), skill_r),
        ]),
        presets: BTreeMap::from([(
            preset_id.clone(),
            PresetDefinition {
                id: preset_id.clone(),
                name: "A".to_owned(),
                revision: 2,
                members: current_members.clone(),
            },
        )]),
        targets: BTreeMap::from([(
            target_id.clone(),
            TargetState {
                enabled_skills: BTreeSet::from([a.clone(), r.clone()]),
                presets: BTreeMap::from([(
                    preset_id.clone(),
                    AppliedPresetState {
                        enabled: false,
                        applied_revision: 1,
                        applied_members: BTreeSet::from([a.clone(), r.clone()]),
                    },
                )]),
                ..TargetState::default()
            },
        )]),
        ..Snapshot::default()
    };

    let plan = plan(
        &Command::SetPresetEnabled {
            preset_id: preset_id.clone(),
            targets: vec![target_id.clone()],
            enabled: true,
        },
        &snapshot,
    )
    .expect("the updated preset can be reopened");

    assert_eq!(
        plan.effects,
        vec![
            Effect::SetSkillEnabled {
                target: target_id.clone(),
                skill: n.clone(),
                enabled: true,
            },
            Effect::SetSkillEnabled {
                target: target_id.clone(),
                skill: r.clone(),
                enabled: false,
            },
        ]
    );
    let next = &plan.target_updates[&target_id];
    assert_eq!(next.enabled_skills, current_members);
    assert_eq!(
        next.presets[&preset_id].applied_members,
        BTreeSet::from([a, n])
    );
    assert_eq!(next.presets[&preset_id].applied_revision, 2);
    assert!(snapshot.targets[&target_id].enabled_skills.contains(&r));
}

#[test]
fn disabling_one_preset_preserves_shared_skill_needed_by_another() {
    let (a, skill_a) = skill("a");
    let (s, skill_s) = skill("s");
    let (r, skill_r) = skill("r");
    let (b, skill_b) = skill("b");
    let (preset_a, definition_a) = preset("A", &[&a, &s, &r], 1);
    let (preset_b, definition_b) = preset("B", &[&s, &b], 1);
    let target = target(AgentId::ClaudeCode, "project-1");
    let snapshot = Snapshot {
        skills: BTreeMap::from([
            (a.clone(), skill_a),
            (s.clone(), skill_s),
            (r.clone(), skill_r),
            (b.clone(), skill_b),
        ]),
        presets: BTreeMap::from([
            (preset_a.clone(), definition_a),
            (preset_b.clone(), definition_b),
        ]),
        projects: BTreeMap::new(),
        entries: BTreeMap::new(),
        targets: BTreeMap::from([(
            target.clone(),
            TargetState {
                enabled_skills: BTreeSet::from([a.clone(), s.clone(), r.clone(), b.clone()]),
                manual_open: BTreeSet::new(),
                manual_closed: BTreeSet::new(),
                presets: BTreeMap::from([
                    (
                        preset_a.clone(),
                        AppliedPresetState {
                            enabled: true,
                            applied_revision: 1,
                            applied_members: BTreeSet::from([a.clone(), s.clone(), r.clone()]),
                        },
                    ),
                    (
                        preset_b.clone(),
                        AppliedPresetState {
                            enabled: true,
                            applied_revision: 1,
                            applied_members: BTreeSet::from([s.clone(), b.clone()]),
                        },
                    ),
                ]),
            },
        )]),
    };

    let plan = plan(
        &Command::SetPresetEnabled {
            preset_id: preset_a.clone(),
            targets: vec![target.clone()],
            enabled: false,
        },
        &snapshot,
    )
    .expect("disabling A is a valid plan");

    assert_eq!(
        plan.effects,
        vec![
            Effect::SetSkillEnabled {
                target: target.clone(),
                skill: a.clone(),
                enabled: false,
            },
            Effect::SetSkillEnabled {
                target: target.clone(),
                skill: r.clone(),
                enabled: false,
            },
        ],
        "disable only A-exclusive entries; never delete the shared source",
    );
    let next = &plan.target_updates[&target];
    assert_eq!(next.enabled_skills, BTreeSet::from([s.clone(), b.clone()]));
    assert!(!next.presets[&preset_a].enabled);
    assert!(next.presets[&preset_b].enabled);
    assert!(snapshot.targets[&target].presets[&preset_a].enabled);
    assert!(snapshot.targets[&target].enabled_skills.contains(&s));
}
