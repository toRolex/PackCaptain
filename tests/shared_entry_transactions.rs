#![cfg(unix)]

use packcaptain::{
    adapters::NativeAdapters,
    domain::{
        AgentId, Command, EntryOwnership, ProjectId, ProjectRecord, Scope, SkillEntry, SkillId,
        Snapshot, TargetId, TargetState,
    },
    engine::{
        ApplyFailure, EffectExecutor, Engine, EngineError, TargetMutation, TargetOutcomeStatus,
    },
    storage::StateStore,
};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    os::unix::fs::MetadataExt,
    path::{Path, PathBuf},
};
use tempfile::TempDir;
type Entries = BTreeMap<SkillId, Vec<SkillEntry>>;

#[derive(Clone, Copy)]
enum Failure {
    None,
    Prepare,
    Apply,
}
struct Executor {
    failure: Failure,
    fail_target: TargetId,
}
impl EffectExecutor for Executor {
    fn prepare_target(
        &self,
        mutation: TargetMutation<'_>,
        snapshot: &Snapshot,
        home: &Path,
    ) -> Result<Entries, String> {
        if mutation.target == &self.fail_target && matches!(self.failure, Failure::Prepare) {
            return Err("injected second-target prepare failure".into());
        }
        NativeAdapters.prepare_target(mutation, snapshot, home)
    }
    fn apply_target(
        &mut self,
        mutation: TargetMutation<'_>,
        snapshot: &Snapshot,
        home: &Path,
    ) -> Result<Entries, ApplyFailure> {
        if mutation.target == &self.fail_target && matches!(self.failure, Failure::Apply) {
            return Err("injected second-target apply failure without actual proof"
                .to_owned()
                .into());
        }
        NativeAdapters.apply_target(mutation, snapshot, home)
    }
}
struct Fixture {
    _root: TempDir,
    home: PathBuf,
    state: PathBuf,
    project: PathBuf,
    source: PathBuf,
    link: PathBuf,
    skill: SkillId,
    first: TargetId,
    second: TargetId,
    snapshot: Snapshot,
}
impl Fixture {
    fn new() -> Self {
        let root = tempfile::tempdir().unwrap();
        let home = root.path().join("home");
        let project = root.path().join("project");
        let state = root.path().join("state");
        let source = home.join(".claude/skills/s");
        fs::create_dir_all(&source).unwrap();
        fs::create_dir_all(project.join(".agents/skills")).unwrap();
        fs::write(
            source.join("SKILL.md"),
            "---\nname: s\ndescription: fixture\n---\n",
        )
        .unwrap();
        let home = fs::canonicalize(home).unwrap();
        let project = fs::canonicalize(project).unwrap();
        let source = fs::canonicalize(source).unwrap();
        let link = project.join(".agents/skills/shared");
        std::os::unix::fs::symlink(&source, &link).unwrap();
        let id = ProjectId::from("shared-fixture");
        let seed = Snapshot {
            projects: BTreeMap::from([(
                id.clone(),
                ProjectRecord {
                    id: id.clone(),
                    name: "fixture".into(),
                    root: project.clone(),
                },
            )]),
            ..Default::default()
        };
        let mut snapshot = NativeAdapters.discover(&home, &seed).merge_into(&seed);
        let skill = SkillId::from_source_path(&source);
        let first = TargetId {
            agent: AgentId::Codex,
            scope: Scope::Project(id.clone()),
        };
        let second = TargetId {
            agent: AgentId::Pi,
            scope: Scope::Project(id),
        };
        for target in [&first, &second] {
            for entry in snapshot
                .entries
                .get_mut(target)
                .unwrap()
                .get_mut(&skill)
                .unwrap()
            {
                entry.ownership = EntryOwnership::PackCaptainLink;
            }
            snapshot.targets.insert(
                target.clone(),
                TargetState {
                    enabled_skills: BTreeSet::from([skill.clone()]),
                    manual_open: BTreeSet::from([skill.clone()]),
                    ..Default::default()
                },
            );
        }
        StateStore::open(&state)
            .unwrap()
            .save_snapshot(&snapshot)
            .unwrap();
        Self {
            _root: root,
            home,
            state,
            project,
            source,
            link,
            skill,
            first,
            second,
            snapshot,
        }
    }
    fn command(&self, targets: Vec<TargetId>) -> Command {
        Command::SetSkillEnabled {
            skill_id: self.skill.clone(),
            targets,
            enabled: false,
        }
    }
    fn identity(&self) -> (u64, u64, PathBuf) {
        let m = fs::symlink_metadata(&self.link).unwrap();
        assert!(m.file_type().is_symlink());
        (m.dev(), m.ino(), fs::read_link(&self.link).unwrap())
    }
    fn assert_source(&self) {
        assert_eq!(
            fs::read_to_string(self.source.join("SKILL.md")).unwrap(),
            "---\nname: s\ndescription: fixture\n---\n"
        );
    }
    fn assert_disabled(&self) {
        assert!(fs::symlink_metadata(&self.link).is_err());
        let after = StateStore::load_existing_snapshot(&self.state).unwrap();
        for target in [&self.first, &self.second] {
            assert!(!after.targets[target].enabled_skills.contains(&self.skill));
            assert!(!after.targets[target].manual_open.contains(&self.skill));
        }
        let observed = NativeAdapters
            .discover(&self.home, &after)
            .merge_into(&after);
        for target in [&self.first, &self.second] {
            assert!(!observed.entries[target][&self.skill][0].present);
            assert_eq!(
                observed.entries[target][&self.skill][0].entry_fingerprint,
                "missing"
            );
        }
        self.assert_source();
    }
}
fn retains_after_failure(failure: Failure) {
    let f = Fixture::new();
    let identity = f.identity();
    let mut engine = Engine::new(
        f.home.clone(),
        f.state.clone(),
        Executor {
            failure,
            fail_target: f.second.clone(),
        },
    );
    let result = engine.execute(&f.command(vec![f.first.clone(), f.second.clone()]));
    match failure {
        Failure::Prepare => {
            let report = result.unwrap();
            assert_eq!(report.targets[0].status, TargetOutcomeStatus::Applied);
            assert_eq!(report.targets[1].status, TargetOutcomeStatus::Failed);
        }
        Failure::Apply => assert!(matches!(result, Err(EngineError::NeedsRecovery { .. }))),
        Failure::None => unreachable!(),
    }
    assert_eq!(
        f.identity(),
        identity,
        "second target still needs committed entry"
    );
    let after = StateStore::load_existing_snapshot(&f.state).unwrap();
    assert!(!after.targets[&f.first].enabled_skills.contains(&f.skill));
    assert_eq!(after.targets[&f.second], f.snapshot.targets[&f.second]);
    assert_eq!(after.entries[&f.second], f.snapshot.entries[&f.second]);
    assert!(after.entries[&f.first][&f.skill][0].present);
    assert!(!after.entries[&f.first][&f.skill][0].active);
    f.assert_source();
    if matches!(failure, Failure::Prepare) {
        let observed = NativeAdapters.discover(&f.home, &after).merge_into(&after);
        assert!(!observed.targets[&f.first].enabled_skills.contains(&f.skill));
        Engine::new(f.home.clone(), f.state.clone(), NativeAdapters)
            .execute_with_native_discovery(&f.command(vec![f.second.clone()]))
            .unwrap();
        f.assert_disabled();
    } else {
        let connection = rusqlite::Connection::open(f.state.join("packcaptain.sqlite3")).unwrap();
        let count: i64 = connection
            .query_row("SELECT count(*) FROM operation_journal", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(count, 1);
    }
}
#[test]
fn second_prepare_failure_keeps_committed_entry_and_retry_releases_it() {
    retains_after_failure(Failure::Prepare);
}
#[test]
fn second_apply_failure_without_actual_proof_keeps_committed_entry_and_state() {
    retains_after_failure(Failure::Apply);
}
#[test]
fn unknown_second_target_keeps_committed_entry_and_state() {
    let f = Fixture::new();
    let identity = f.identity();
    fs::create_dir_all(f.project.join(".pi")).unwrap();
    fs::write(f.project.join(".pi/settings.json"), "{broken-json").unwrap();
    let report = Engine::new(f.home.clone(), f.state.clone(), NativeAdapters)
        .execute_with_native_discovery(&f.command(vec![f.first.clone(), f.second.clone()]))
        .unwrap();
    assert_eq!(
        report
            .targets
            .iter()
            .find(|item| item.target == f.first)
            .unwrap()
            .status,
        TargetOutcomeStatus::Applied
    );
    assert_eq!(
        report
            .targets
            .iter()
            .find(|item| item.target == f.second)
            .unwrap()
            .status,
        TargetOutcomeStatus::Failed
    );
    assert_eq!(f.identity(), identity);
    let after = StateStore::load_existing_snapshot(&f.state).unwrap();
    assert_eq!(after.targets[&f.second], f.snapshot.targets[&f.second]);
    assert_eq!(after.entries[&f.second], f.snapshot.entries[&f.second]);
    assert!(!after.targets[&f.first].enabled_skills.contains(&f.skill));
    f.assert_source();
}
#[test]
fn both_targets_disable_successfully_and_last_reference_releases_entry() {
    let f = Fixture::new();
    let report = Engine::new(
        f.home.clone(),
        f.state.clone(),
        Executor {
            failure: Failure::None,
            fail_target: f.second.clone(),
        },
    )
    .execute(&f.command(vec![f.first.clone(), f.second.clone()]))
    .unwrap();
    assert!(
        report
            .targets
            .iter()
            .all(|item| item.status == TargetOutcomeStatus::Applied)
    );
    f.assert_disabled();
}
#[test]
fn reverse_target_commit_order_releases_entry() {
    let f = Fixture::new();
    let mut engine = Engine::new(f.home.clone(), f.state.clone(), NativeAdapters);
    engine.execute(&f.command(vec![f.second.clone()])).unwrap();
    assert!(f.link.is_symlink());
    engine
        .execute_with_native_discovery(&f.command(vec![f.first.clone()]))
        .unwrap();
    f.assert_disabled();
}
#[test]
fn apply_rechecks_committed_peer_instead_of_trusting_prepared_unlink() {
    let f = Fixture::new();
    let mut no_peer_need = f.snapshot.clone();
    no_peer_need
        .targets
        .insert(f.second.clone(), TargetState::default());
    let command = f.command(vec![f.first.clone(), f.second.clone()]);
    let plan = packcaptain::domain::plan(&command, &no_peer_need).unwrap();
    let effects: Vec<_> = plan.effects.iter().filter(|effect| matches!(effect, packcaptain::domain::Effect::SetSkillEnabled { target, .. } if target == &f.first)).cloned().collect();
    let before = &f.snapshot.entries[&f.first];
    let prepared = NativeAdapters
        .prepare_target(
            TargetMutation {
                receipt_context: None,
                target: &f.first,
                before: &no_peer_need.targets[&f.first],
                after: &plan.target_updates[&f.first],
                effects: &effects,
                entries_before: before,
                entries_after: before,
                planned_target_states: &plan.target_updates,
            },
            &no_peer_need,
            &f.home,
        )
        .unwrap();
    assert!(!prepared[&f.skill][0].present);
    let identity = f.identity();
    let actual = NativeAdapters
        .apply_target(
            TargetMutation {
                receipt_context: None,
                target: &f.first,
                before: &f.snapshot.targets[&f.first],
                after: &plan.target_updates[&f.first],
                effects: &effects,
                entries_before: before,
                entries_after: &prepared,
                planned_target_states: &plan.target_updates,
            },
            &f.snapshot,
            &f.home,
        )
        .unwrap();
    assert!(actual[&f.skill][0].present);
    assert!(!actual[&f.skill][0].active);
    assert_eq!(identity, f.identity());
    f.assert_source();
}
#[test]
fn released_retained_route_can_be_enabled_again_without_claiming_replacement() {
    let f = Fixture::new();
    let mut engine = Engine::new(f.home.clone(), f.state.clone(), NativeAdapters);
    engine
        .execute(&f.command(vec![f.first.clone(), f.second.clone()]))
        .unwrap();
    f.assert_disabled();
    let enable = Command::SetSkillEnabled {
        skill_id: f.skill.clone(),
        targets: vec![f.first.clone()],
        enabled: true,
    };
    engine.execute_with_native_discovery(&enable).unwrap();
    assert!(f.link.is_symlink());
    f.assert_source();
}

#[test]
fn enabled_presets_with_explicit_manual_close_do_not_pin_last_entry() {
    let mut f = Fixture::new();
    let preset = packcaptain::domain::PresetId::from("shared-preset");
    for target in [&f.first, &f.second] {
        f.snapshot.targets.get_mut(target).unwrap().presets.insert(
            preset.clone(),
            packcaptain::domain::AppliedPresetState {
                enabled: true,
                applied_revision: 1,
                applied_members: BTreeSet::from([f.skill.clone()]),
            },
        );
    }
    StateStore::open(&f.state)
        .unwrap()
        .save_snapshot(&f.snapshot)
        .unwrap();
    let report = Engine::new(f.home.clone(), f.state.clone(), NativeAdapters)
        .execute(&f.command(vec![f.first.clone(), f.second.clone()]))
        .unwrap();
    assert!(
        report
            .targets
            .iter()
            .all(|item| item.status == TargetOutcomeStatus::Applied)
    );
    f.assert_disabled();
}
