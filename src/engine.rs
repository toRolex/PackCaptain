use std::collections::BTreeMap;
use std::error::Error;
use std::fmt;
use std::fs::{self, File, OpenOptions};
use std::path::{Path, PathBuf};

use fs2::FileExt;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::adapters::{DiscoveryError, DiscoveryReport, NativeAdapters, ProjectCandidate};
use crate::domain::{
    AgentId, Command, ConflictReport, Effect, Plan, ProjectRecord, Scope, SkillEntry, SkillId,
    Snapshot, TargetId, TargetState, plan,
};
use crate::storage::{JournalStatus, StateStore, StorageError, TargetJournal};

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum RecoveryDirection {
    Forward,
    Rollback,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ReceiptKey {
    pub operation_id: String,
    pub direction: RecoveryDirection,
    /// Unique across all skill routes in this journal direction.
    pub route_ordinal: u32,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct SymlinkIdentity {
    pub device: u64,
    pub inode: u64,
    pub readlink: PathBuf,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum ReceiptAction {
    Create,
    Delete { owned_fingerprint: String },
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
/// Immutable once prepared. The caller must verify the public path and anchor
/// share this no-follow symlink identity before using it as recovery proof.
pub struct ReceiptIdentity {
    pub target: TargetId,
    pub skill: SkillId,
    pub path: PathBuf,
    pub source: PathBuf,
    pub anchor: PathBuf,
    pub no_follow: SymlinkIdentity,
    pub action: ReceiptAction,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum ReceiptStage {
    Prepared,
    VerifiedApplied,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct MutationReceipt {
    pub key: ReceiptKey,
    pub identity: ReceiptIdentity,
    pub stage: ReceiptStage,
}

pub trait ReceiptWriter {
    /// Prepared must commit before the caller publishes or removes the public entry.
    /// Repeating a write is idempotent. Identity rebinding and stage regression fail.
    fn save_receipt(&self, receipt: &MutationReceipt) -> Result<(), StorageError>;
}

pub trait ReceiptReader {
    fn load_receipt(&self, key: &ReceiptKey) -> Result<Option<MutationReceipt>, StorageError>;
}

pub struct ForwardReceiptContext<'a> {
    pub operation_id: &'a str,
    pub direction: RecoveryDirection,
    pub route_ordinals: BTreeMap<(SkillId, PathBuf), u32>,
    pub writer: &'a dyn ReceiptWriter,
    pub reader: &'a dyn ReceiptReader,
}

impl ForwardReceiptContext<'_> {
    pub fn key(&self, skill: &SkillId, path: &Path) -> Result<ReceiptKey, String> {
        let route_ordinal = self
            .route_ordinals
            .get(&(skill.clone(), path.to_owned()))
            .copied()
            .ok_or_else(|| "route has no receipt ordinal".to_owned())?;
        Ok(ReceiptKey {
            operation_id: self.operation_id.to_owned(),
            direction: self.direction,
            route_ordinal,
        })
    }
}

pub struct TargetMutation<'a> {
    pub receipt_context: Option<&'a ForwardReceiptContext<'a>>,
    pub target: &'a TargetId,
    pub before: &'a TargetState,
    pub after: &'a TargetState,
    pub effects: &'a [Effect],
    pub entries_before: &'a BTreeMap<SkillId, Vec<SkillEntry>>,
    pub entries_after: &'a BTreeMap<SkillId, Vec<SkillEntry>>,
    pub planned_target_states: &'a BTreeMap<TargetId, TargetState>,
}

#[derive(Debug)]
pub struct ApplyFailure {
    pub message: String,
    pub actual_entries: Option<BTreeMap<SkillId, Vec<SkillEntry>>>,
}

impl From<String> for ApplyFailure {
    fn from(message: String) -> Self {
        Self {
            message,
            actual_entries: None,
        }
    }
}

pub trait EffectExecutor {
    fn prepare_target(
        &self,
        mutation: TargetMutation<'_>,
        snapshot: &Snapshot,
        agent_home: &Path,
    ) -> Result<BTreeMap<SkillId, Vec<SkillEntry>>, String>;

    // Failures may carry complete, proven actual metadata, never inferred ownership.
    fn apply_target(
        &mut self,
        mutation: TargetMutation<'_>,
        snapshot: &Snapshot,
        agent_home: &Path,
    ) -> Result<BTreeMap<SkillId, Vec<SkillEntry>>, ApplyFailure>;
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum TargetOutcomeStatus {
    Applied,
    Failed,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct TargetOutcome {
    pub target: TargetId,
    pub status: TargetOutcomeStatus,
    pub message: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ExecutionReport {
    pub plan: Plan,
    pub targets: Vec<TargetOutcome>,
}

#[derive(Debug)]
pub enum EngineError {
    Conflict(ConflictReport),
    Storage(StorageError),
    Lock(std::io::Error),
    Validation(String),
    Discovery {
        target: TargetId,
        error: DiscoveryError,
    },
    PreviewChanged(Box<Plan>),
    NeedsRecovery {
        target: TargetId,
        message: String,
        completed_targets: Vec<TargetId>,
    },
}

impl fmt::Display for EngineError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Conflict(conflict) => write!(formatter, "{}", conflict.message),
            Self::Storage(error) => write!(formatter, "{error}"),
            Self::Lock(error) => write!(formatter, "state lock failed: {error}"),
            Self::Validation(message) => formatter.write_str(message),
            Self::Discovery { target, error } => {
                write!(formatter, "target {target} state is unknown: {error}")
            }
            Self::PreviewChanged(_) => formatter.write_str("状态已变化，请重新确认"),
            Self::NeedsRecovery {
                target, message, ..
            } => write!(formatter, "target {target} needs recovery: {message}"),
        }
    }
}

impl Error for EngineError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Conflict(_) => None,
            Self::Storage(error) => Some(error),
            Self::Lock(error) => Some(error),
            Self::Discovery { error, .. } => Some(error),
            Self::Validation(_) | Self::PreviewChanged(_) => None,
            Self::NeedsRecovery { .. } => None,
        }
    }
}

impl From<StorageError> for EngineError {
    fn from(error: StorageError) -> Self {
        Self::Storage(error)
    }
}

fn reject_unknown_targets<'a>(
    discovery: &DiscoveryReport,
    targets: impl IntoIterator<Item = &'a TargetId>,
) -> Result<(), EngineError> {
    for target in targets {
        if let Some(error) = discovery.target_errors.get(target) {
            return Err(EngineError::Discovery {
                target: target.clone(),
                error: error.clone(),
            });
        }
    }
    Ok(())
}

fn healthy_plan(mut plan: Plan, discovery: &DiscoveryReport) -> Plan {
    plan.target_updates
        .retain(|target, _| !discovery.target_errors.contains_key(target));
    plan.effects
        .retain(|effect| !discovery.target_errors.contains_key(effect_target(effect)));
    plan
}

fn check_confirmation(
    fresh: &Plan,
    confirmed: &Plan,
    discovery: &DiscoveryReport,
) -> Result<(), EngineError> {
    let affected: Vec<_> = fresh
        .target_updates
        .keys()
        .filter(|target| discovery.target_errors.contains_key(*target))
        .collect();
    if !affected.is_empty()
        && (affected.len() == fresh.target_updates.len() || !fresh.preset_updates.is_empty())
    {
        reject_unknown_targets(discovery, affected)?;
    }
    let healthy = healthy_plan(fresh.clone(), discovery);
    if healthy != healthy_plan(confirmed.clone(), discovery) {
        return Err(EngineError::PreviewChanged(Box::new(healthy)));
    }
    Ok(())
}

pub struct Engine<E> {
    agent_home: PathBuf,
    state_dir: PathBuf,
    executor: E,
}

impl<E: EffectExecutor> Engine<E> {
    pub fn new(agent_home: PathBuf, state_dir: PathBuf, executor: E) -> Self {
        Self {
            agent_home,
            state_dir,
            executor,
        }
    }

    pub fn snapshot(&self) -> Result<Snapshot, EngineError> {
        StateStore::load_existing_snapshot(&self.state_dir).map_err(EngineError::from)
    }

    pub fn scan_candidates(root: &Path) -> Result<Vec<ProjectCandidate>, DiscoveryError> {
        NativeAdapters::scan_candidates(root)
    }

    pub fn claim_project(&mut self, mut project: ProjectRecord) -> Result<Snapshot, EngineError> {
        if !project.root.is_absolute() {
            return Err(EngineError::Validation(
                "project root must be absolute".to_owned(),
            ));
        }
        let root = fs::canonicalize(&project.root).map_err(|error| {
            EngineError::Validation(format!("cannot resolve project root: {error}"))
        })?;
        if !root.is_dir() {
            return Err(EngineError::Validation(
                "project root is not a directory".to_owned(),
            ));
        }
        if project.name.trim().is_empty() {
            return Err(EngineError::Validation(
                "project name cannot be empty".to_owned(),
            ));
        }
        project.root = root;
        let _lock = StateLock::acquire(&self.state_dir).map_err(EngineError::Lock)?;
        let snapshot = StateStore::load_existing_snapshot(&self.state_dir)?;
        let journals = StateStore::load_existing_journals(&self.state_dir)?;
        let discovery = NativeAdapters.discover(&self.agent_home, &snapshot);
        reject_unknown_targets(&discovery, journals.iter().map(|journal| &journal.target))?;
        let mut store = StateStore::open(&self.state_dir)?;
        self.recover_pending(&mut store, &[])?;
        let mut snapshot = store.load_snapshot()?;
        if let Some(existing) = snapshot
            .projects
            .values()
            .find(|existing| existing.root == project.root)
        {
            let _ = existing;
            return Ok(snapshot);
        }
        if let Some(existing) = snapshot.projects.get(&project.id) {
            return Err(EngineError::Validation(format!(
                "project ID {} is already registered at {}",
                project.id.as_str(),
                existing.root.display()
            )));
        }
        let project_id = project.id.clone();
        snapshot.projects.insert(project_id.clone(), project);
        for agent in [AgentId::ClaudeCode, AgentId::Codex, AgentId::Pi] {
            let target = TargetId {
                agent,
                scope: Scope::Project(project_id.clone()),
            };
            snapshot.targets.entry(target.clone()).or_default();
            snapshot.entries.entry(target).or_default();
        }
        store.save_snapshot(&snapshot)?;
        Ok(snapshot)
    }

    pub fn preview(&self, command: &Command) -> Result<Plan, EngineError> {
        self.preview_with_discovery(command, &DiscoveryReport::default())
    }

    pub fn preview_with_discovery(
        &self,
        command: &Command,
        discovery: &DiscoveryReport,
    ) -> Result<Plan, EngineError> {
        let snapshot = StateStore::load_existing_snapshot(&self.state_dir)?;
        let journals = StateStore::load_existing_journals(&self.state_dir)?;
        let prospective = snapshot_after_pending_journals(snapshot, &journals);
        let merged = discovery.merge_into(&prospective);
        let plan = plan(command, &merged).map_err(EngineError::Conflict)?;
        reject_unknown_targets(discovery, plan.target_updates.keys())?;
        Ok(plan)
    }

    pub fn execute(&mut self, command: &Command) -> Result<ExecutionReport, EngineError> {
        self.execute_with_discovery(command, &DiscoveryReport::default())
    }

    pub fn execute_with_discovery(
        &mut self,
        command: &Command,
        discovery: &DiscoveryReport,
    ) -> Result<ExecutionReport, EngineError> {
        self.execute_with_scan(command, None, |_, _| discovery.clone())
    }

    pub fn execute_with_native_discovery(
        &mut self,
        command: &Command,
    ) -> Result<ExecutionReport, EngineError> {
        self.execute_with_scan(command, None, |home, snapshot| {
            NativeAdapters.discover(home, snapshot)
        })
    }

    pub fn preview_with_native_discovery(&self, command: &Command) -> Result<Plan, EngineError> {
        let snapshot = StateStore::load_existing_snapshot(&self.state_dir)?;
        let journals = StateStore::load_existing_journals(&self.state_dir)?;
        let prospective = snapshot_after_pending_journals(snapshot, &journals);
        let discovery = NativeAdapters.discover(&self.agent_home, &prospective);
        let plan =
            plan(command, &discovery.merge_into(&prospective)).map_err(EngineError::Conflict)?;
        reject_unknown_targets(&discovery, plan.target_updates.keys())?;
        Ok(plan)
    }

    pub fn execute_confirmed_with_native_discovery(
        &mut self,
        command: &Command,
        confirmed: &Plan,
    ) -> Result<ExecutionReport, EngineError> {
        self.execute_with_scan(command, Some(confirmed), |home, snapshot| {
            NativeAdapters.discover(home, snapshot)
        })
    }

    fn execute_with_scan(
        &mut self,
        command: &Command,
        confirmed: Option<&Plan>,
        mut scan: impl FnMut(&Path, &Snapshot) -> DiscoveryReport,
    ) -> Result<ExecutionReport, EngineError> {
        let read_snapshot = StateStore::load_existing_snapshot(&self.state_dir)?;
        let read_journals = StateStore::load_existing_journals(&self.state_dir)?;
        let discovery = scan(&self.agent_home, &read_snapshot);
        let prospective = snapshot_after_pending_journals(read_snapshot, &read_journals);
        let prospective = discovery.merge_into(&prospective);
        let initial_plan = plan(command, &prospective).map_err(EngineError::Conflict)?;
        if confirmed.is_some() && !read_journals.is_empty() {
            return Err(EngineError::Validation(
                "存在未完成操作，需先完成恢复后重新预览".to_owned(),
            ));
        }
        if let Some(confirmed) = confirmed {
            check_confirmation(&initial_plan, confirmed, &discovery)?;
        }
        if !initial_plan.preset_updates.is_empty() {
            reject_unknown_targets(&discovery, initial_plan.target_updates.keys())?;
        }
        reject_unknown_targets(
            &discovery,
            read_journals.iter().map(|journal| &journal.target),
        )?;
        if !initial_plan.target_updates.is_empty()
            && initial_plan
                .target_updates
                .keys()
                .all(|target| discovery.target_errors.contains_key(target))
        {
            return Ok(ExecutionReport {
                targets: initial_plan
                    .target_updates
                    .keys()
                    .map(|target| TargetOutcome {
                        target: target.clone(),
                        status: TargetOutcomeStatus::Failed,
                        message: discovery
                            .target_errors
                            .get(target)
                            .map(|error| format!("state is unknown: {error}")),
                    })
                    .collect(),
                plan: healthy_plan(initial_plan, &discovery),
            });
        }

        let _lock = StateLock::acquire(&self.state_dir).map_err(EngineError::Lock)?;
        if let Some(confirmed) = confirmed {
            let snapshot = StateStore::load_existing_snapshot(&self.state_dir)?;
            let journals = StateStore::load_existing_journals(&self.state_dir)?;
            if !journals.is_empty() {
                return Err(EngineError::Validation(
                    "存在未完成操作，需先完成恢复后重新预览".to_owned(),
                ));
            }
            let discovery = scan(&self.agent_home, &snapshot);
            let fresh =
                plan(command, &discovery.merge_into(&snapshot)).map_err(EngineError::Conflict)?;
            check_confirmation(&fresh, confirmed, &discovery)?;
        }
        let current_snapshot = StateStore::load_existing_snapshot(&self.state_dir)?;
        let current_journals = StateStore::load_existing_journals(&self.state_dir)?;
        let discovery = scan(&self.agent_home, &current_snapshot);
        reject_unknown_targets(
            &discovery,
            current_journals.iter().map(|journal| &journal.target),
        )?;
        let prospective = snapshot_after_pending_journals(current_snapshot, &current_journals);
        let prospective = discovery.merge_into(&prospective);
        plan(command, &prospective).map_err(EngineError::Conflict)?;

        let mut store = StateStore::open(&self.state_dir)?;
        let mut completed_targets = self.recover_pending(&mut store, &[])?;
        let stored_snapshot = store.load_snapshot()?;
        let mut current_discovery = scan(&self.agent_home, &stored_snapshot);
        for journal in &current_journals {
            current_discovery.entries.remove(&journal.target);
            current_discovery.observed_enabled.remove(&journal.target);
        }
        let snapshot = current_discovery.merge_into(&stored_snapshot);
        let operation_plan = plan(command, &snapshot).map_err(EngineError::Conflict)?;
        if let Some(confirmed) = confirmed {
            check_confirmation(&operation_plan, confirmed, &current_discovery)?;
        }
        if !operation_plan.preset_updates.is_empty() {
            reject_unknown_targets(&current_discovery, operation_plan.target_updates.keys())?;
        }
        let mut target_outcomes: Vec<_> = operation_plan
            .target_updates
            .keys()
            .filter_map(|target| {
                current_discovery
                    .target_errors
                    .get(target)
                    .map(|error| TargetOutcome {
                        target: target.clone(),
                        status: TargetOutcomeStatus::Failed,
                        message: Some(format!("state is unknown: {error}")),
                    })
            })
            .collect();
        let operation_plan = healthy_plan(operation_plan, &current_discovery);
        let mut committed_snapshot = snapshot;
        for (preset_id, definition) in &operation_plan.preset_updates {
            committed_snapshot
                .presets
                .insert(preset_id.clone(), definition.clone());
        }
        if committed_snapshot != stored_snapshot {
            store.save_snapshot(&committed_snapshot)?;
        }

        for (target, next_state) in &operation_plan.target_updates {
            let before = committed_snapshot
                .targets
                .get(target)
                .cloned()
                .unwrap_or_default();
            let effects: Vec<_> = operation_plan
                .effects
                .iter()
                .filter(|effect| effect_target(effect) == target)
                .cloned()
                .collect();
            if effects.is_empty() {
                committed_snapshot
                    .targets
                    .insert(target.clone(), next_state.clone());
                store.save_snapshot(&committed_snapshot)?;
                target_outcomes.push(TargetOutcome {
                    target: target.clone(),
                    status: TargetOutcomeStatus::Applied,
                    message: None,
                });
                completed_targets.push(target.clone());
                continue;
            }

            let entries_before = committed_snapshot
                .entries
                .get(target)
                .cloned()
                .unwrap_or_default();
            let entries_after = match self.executor.prepare_target(
                TargetMutation {
                    receipt_context: None,
                    target,
                    before: &before,
                    after: next_state,
                    effects: &effects,
                    entries_before: &entries_before,
                    entries_after: &entries_before,
                    planned_target_states: &operation_plan.target_updates,
                },
                &committed_snapshot,
                &self.agent_home,
            ) {
                Ok(entries) => entries,
                Err(message) => {
                    target_outcomes.push(TargetOutcome {
                        target: target.clone(),
                        status: TargetOutcomeStatus::Failed,
                        message: Some(message),
                    });
                    continue;
                }
            };
            let mut journal = TargetJournal {
                id: Uuid::new_v4().to_string(),
                target: target.clone(),
                status: JournalStatus::Applying,
                before,
                after: next_state.clone(),
                entries_before,
                entries_after,
                planned_target_states: operation_plan.target_updates.clone(),
                effects,
                failure: None,
            };
            store.insert_journal(&journal)?;
            match self.finish_forward(&store, &mut journal, &committed_snapshot) {
                Ok(()) => {
                    commit_and_cleanup(&mut store, &journal, next_state)?;
                    committed_snapshot
                        .targets
                        .insert(target.clone(), next_state.clone());
                    committed_snapshot
                        .entries
                        .insert(target.clone(), journal.entries_after.clone());
                    target_outcomes.push(TargetOutcome {
                        target: target.clone(),
                        status: TargetOutcomeStatus::Applied,
                        message: None,
                    });
                    completed_targets.push(target.clone());
                }
                Err(failure) => {
                    let message = failure.message;
                    if failure.actual_entries.is_none() {
                        journal.status = JournalStatus::NeedsRecovery;
                        journal.failure = Some(message.clone());
                        store.update_journal(&journal)?;
                        return Err(EngineError::NeedsRecovery {
                            target: target.clone(),
                            message,
                            completed_targets,
                        });
                    }
                    journal.status = JournalStatus::RollingBack;
                    journal.failure = Some(message.clone());
                    store.update_journal(&journal)?;
                    if let Err(rollback_error) =
                        self.rollback_journal(&mut store, &mut journal, &committed_snapshot)
                    {
                        journal.status = JournalStatus::NeedsRecovery;
                        journal.failure = Some(rollback_error.clone());
                        store.update_journal(&journal)?;
                        return Err(EngineError::NeedsRecovery {
                            target: target.clone(),
                            message: rollback_error,
                            completed_targets,
                        });
                    }
                    target_outcomes.push(TargetOutcome {
                        target: target.clone(),
                        status: TargetOutcomeStatus::Failed,
                        message: Some(message),
                    });
                }
            }
        }

        Ok(ExecutionReport {
            plan: operation_plan,
            targets: target_outcomes,
        })
    }

    fn recover_pending(
        &mut self,
        store: &mut StateStore,
        completed: &[TargetId],
    ) -> Result<Vec<TargetId>, EngineError> {
        let mut completed_targets = completed.to_vec();
        let mut journals = store.pending_journals()?;
        if !journals.is_empty() {
            let snapshot = store.load_snapshot()?;
            let discovery = NativeAdapters.discover(&self.agent_home, &snapshot);
            reject_unknown_targets(&discovery, journals.iter().map(|journal| &journal.target))?;
        }
        journals.sort_by(|left, right| left.id.cmp(&right.id));
        for mut journal in journals {
            let snapshot = store.load_snapshot()?;
            match journal.status {
                JournalStatus::Applying => {
                    if let Err(failure) = self.finish_forward(store, &mut journal, &snapshot) {
                        let message = failure.message;
                        if failure.actual_entries.is_none() {
                            journal.status = JournalStatus::NeedsRecovery;
                            journal.failure = Some(message.clone());
                            store.update_journal(&journal)?;
                            return Err(EngineError::NeedsRecovery {
                                target: journal.target,
                                message,
                                completed_targets,
                            });
                        }
                        journal.status = JournalStatus::RollingBack;
                        journal.failure = Some(message.clone());
                        store.update_journal(&journal)?;
                        if let Err(rollback_error) =
                            self.rollback_journal(store, &mut journal, &snapshot)
                        {
                            journal.status = JournalStatus::NeedsRecovery;
                            journal.failure = Some(rollback_error.clone());
                            store.update_journal(&journal)?;
                            return Err(EngineError::NeedsRecovery {
                                target: journal.target,
                                message: rollback_error,
                                completed_targets,
                            });
                        }
                    } else {
                        commit_and_cleanup(store, &journal, &journal.after)?;
                        completed_targets.push(journal.target);
                    }
                }
                JournalStatus::RollingBack => {
                    if let Err(message) = self.rollback_journal(store, &mut journal, &snapshot) {
                        journal.status = JournalStatus::NeedsRecovery;
                        journal.failure = Some(message.clone());
                        store.update_journal(&journal)?;
                        return Err(EngineError::NeedsRecovery {
                            target: journal.target,
                            message,
                            completed_targets,
                        });
                    }
                }
                JournalStatus::NeedsRecovery => {
                    return Err(EngineError::NeedsRecovery {
                        target: journal.target,
                        message: journal
                            .failure
                            .unwrap_or_else(|| "previous operation needs recovery".to_owned()),
                        completed_targets,
                    });
                }
            }
        }
        Ok(completed_targets)
    }

    fn finish_forward(
        &mut self,
        store: &StateStore,
        journal: &mut TargetJournal,
        snapshot: &Snapshot,
    ) -> Result<(), ApplyFailure> {
        let context = receipt_context(store, journal, RecoveryDirection::Forward)
            .map_err(|error| ApplyFailure::from(error.to_string()))?;
        let mutation = TargetMutation {
            receipt_context: Some(&context),
            target: &journal.target,
            before: &journal.before,
            after: &journal.after,
            effects: &journal.effects,
            entries_before: &journal.entries_before,
            entries_after: &journal.entries_after,
            planned_target_states: &journal.planned_target_states,
        };
        match self
            .executor
            .apply_target(mutation, snapshot, &self.agent_home)
        {
            Ok(entries) => journal.entries_after = entries,
            Err(failure) => {
                if let Some(entries) = &failure.actual_entries {
                    journal.entries_after = entries.clone();
                    store
                        .update_journal(journal)
                        .map_err(|error| ApplyFailure::from(error.to_string()))?;
                }
                return Err(failure);
            }
        }
        store
            .update_journal(journal)
            .map_err(|error| ApplyFailure::from(error.to_string()))
    }

    fn rollback_journal(
        &mut self,
        store: &mut StateStore,
        journal: &mut TargetJournal,
        snapshot: &Snapshot,
    ) -> Result<(), String> {
        let inverse_effects = inverse_effects(&journal.effects, &journal.before);
        let context = receipt_context(store, journal, RecoveryDirection::Rollback)
            .map_err(|error| error.to_string())?;
        let mutation = TargetMutation {
            receipt_context: Some(&context),
            target: &journal.target,
            before: &journal.after,
            after: &journal.before,
            effects: &inverse_effects,
            entries_before: &journal.entries_after,
            entries_after: &journal.entries_before,
            planned_target_states: &journal.planned_target_states,
        };
        match self
            .executor
            .apply_target(mutation, snapshot, &self.agent_home)
        {
            Ok(entries) => journal.entries_after = entries,
            Err(failure) => {
                if let Some(entries) = failure.actual_entries {
                    journal.entries_after = entries;
                    store
                        .update_journal(journal)
                        .map_err(|error| error.to_string())?;
                }
                return Err(failure.message);
            }
        }
        commit_and_cleanup(store, journal, &journal.before).map_err(|error| error.to_string())
    }
}

fn receipt_context<'a>(
    store: &'a StateStore,
    journal: &'a TargetJournal,
    direction: RecoveryDirection,
) -> Result<ForwardReceiptContext<'a>, StorageError> {
    let routes: std::collections::BTreeSet<_> = journal
        .entries_before
        .iter()
        .chain(&journal.entries_after)
        .flat_map(|(skill, entries)| {
            entries
                .iter()
                .map(move |entry| (skill.clone(), entry.path.clone()))
        })
        .collect();
    let route_ordinals = routes
        .into_iter()
        .enumerate()
        .map(|(ordinal, route)| {
            u32::try_from(ordinal)
                .map(|ordinal| (route, ordinal))
                .map_err(|_| StorageError::Receipt("too many routes".to_owned()))
        })
        .collect::<Result<_, _>>()?;
    Ok(ForwardReceiptContext {
        operation_id: &journal.id,
        direction,
        route_ordinals,
        writer: store,
        reader: store,
    })
}

fn commit_and_cleanup(
    store: &mut StateStore,
    journal: &TargetJournal,
    state: &TargetState,
) -> Result<(), StorageError> {
    let mut receipts = Vec::new();
    for direction in [RecoveryDirection::Forward, RecoveryDirection::Rollback] {
        let context = receipt_context(store, journal, direction)?;
        for (skill, path) in context.route_ordinals.keys() {
            if let Some(receipt) =
                store.load_receipt(&context.key(skill, path).map_err(StorageError::Receipt)?)?
                && receipt.identity.target == journal.target
                && receipt.identity.skill == *skill
                && receipt.identity.path == *path
            {
                receipts.push(receipt);
            }
        }
    }
    store.commit_target_and_finish_journal(
        &journal.target,
        state,
        &journal.entries_after,
        &journal.id,
    )?;
    for receipt in receipts {
        crate::adapters::cleanup_receipt_anchor(&receipt);
    }
    Ok(())
}

fn snapshot_after_pending_journals(mut snapshot: Snapshot, journals: &[TargetJournal]) -> Snapshot {
    for journal in journals {
        let (state, entries) = match journal.status {
            JournalStatus::Applying => (&journal.after, &journal.entries_after),
            JournalStatus::RollingBack | JournalStatus::NeedsRecovery => {
                (&journal.before, &journal.entries_before)
            }
        };
        snapshot
            .targets
            .insert(journal.target.clone(), state.clone());
        snapshot
            .entries
            .insert(journal.target.clone(), entries.clone());
    }
    snapshot
}

fn inverse_effects(effects: &[Effect], before: &TargetState) -> Vec<Effect> {
    effects
        .iter()
        .map(|effect| Effect::SetSkillEnabled {
            target: effect_target(effect).clone(),
            skill: effect_skill(effect).clone(),
            enabled: before.enabled_skills.contains(effect_skill(effect)),
        })
        .collect()
}

fn effect_target(effect: &Effect) -> &TargetId {
    match effect {
        Effect::SetSkillEnabled { target, .. } => target,
    }
}

fn effect_skill(effect: &Effect) -> &SkillId {
    match effect {
        Effect::SetSkillEnabled { skill, .. } => skill,
    }
}

#[cfg(test)]
mod tests {
    use std::collections::{BTreeMap, BTreeSet};

    use tempfile::tempdir;

    use super::*;
    use crate::domain::{
        AgentId, AppliedPresetState, ClosedMembersPolicy, PresetDefinition, PresetId, Scope,
        SkillAvailability, SkillRecord,
    };

    #[derive(Default)]
    struct CountingExecutor {
        calls: usize,
    }

    impl EffectExecutor for CountingExecutor {
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
            self.calls += 1;
            Ok(mutation.entries_after.clone())
        }
    }

    #[test]
    fn invalid_conflict_does_not_recover_pending_journal_or_create_lock() {
        let temporary = tempdir().unwrap();
        let state_dir = temporary.path().join("state");
        let target = TargetId {
            agent: AgentId::ClaudeCode,
            scope: Scope::Project("project-1".into()),
        };
        let preset_id = PresetId::from("A");
        let skill_a = SkillId::from("a");
        let skill_b = SkillId::from("b");
        let preset = PresetDefinition {
            id: preset_id.clone(),
            name: "A".to_owned(),
            revision: 1,
            members: BTreeSet::from([skill_a.clone(), skill_b.clone()]),
        };
        let before = TargetState {
            enabled_skills: BTreeSet::from([skill_b.clone()]),
            manual_open: BTreeSet::new(),
            manual_closed: BTreeSet::from([skill_a.clone()]),
            presets: BTreeMap::from([(
                preset_id.clone(),
                AppliedPresetState {
                    enabled: true,
                    applied_revision: 1,
                    applied_members: BTreeSet::from([skill_a.clone(), skill_b.clone()]),
                },
            )]),
        };
        let snapshot = Snapshot {
            skills: BTreeMap::from([
                (
                    skill_a.clone(),
                    SkillRecord {
                        id: skill_a.clone(),
                        name: "a".to_owned(),
                        source_path: "/fixture/a".into(),
                        source_fingerprint: "a-hash".to_owned(),
                        availability: SkillAvailability::Available,
                    },
                ),
                (
                    skill_b.clone(),
                    SkillRecord {
                        id: skill_b.clone(),
                        name: "b".to_owned(),
                        source_path: "/fixture/b".into(),
                        source_fingerprint: "b-hash".to_owned(),
                        availability: SkillAvailability::Available,
                    },
                ),
            ]),
            presets: BTreeMap::from([(preset_id.clone(), preset)]),
            targets: BTreeMap::from([(target.clone(), before.clone())]),
            ..Snapshot::default()
        };
        let after = TargetState {
            enabled_skills: BTreeSet::new(),
            manual_open: BTreeSet::new(),
            manual_closed: BTreeSet::from([skill_a.clone(), skill_b.clone()]),
            presets: before.presets.clone(),
        };
        let mut store = StateStore::open(&state_dir).unwrap();
        store.save_snapshot(&snapshot).unwrap();
        store
            .insert_journal(&TargetJournal {
                id: "interrupted".to_owned(),
                target: target.clone(),
                status: JournalStatus::Applying,
                before: before.clone(),
                after,
                entries_before: BTreeMap::new(),
                entries_after: BTreeMap::new(),
                planned_target_states: BTreeMap::from([(target.clone(), before.clone())]),
                effects: vec![Effect::SetSkillEnabled {
                    target: target.clone(),
                    skill: skill_b.clone(),
                    enabled: false,
                }],
                failure: None,
            })
            .unwrap();
        drop(store);

        let mut engine = Engine::new(
            temporary.path().join("agent-home"),
            state_dir.clone(),
            CountingExecutor::default(),
        );
        let error = engine
            .execute(&Command::UpdatePreset {
                preset_id,
                expected_revision: 1,
                members: BTreeSet::from([skill_a.clone(), skill_b.clone()]),
                sync_targets: vec![target.clone()],
                closed_members: ClosedMembersPolicy::Unspecified,
            })
            .unwrap_err();

        assert!(matches!(
            error,
            EngineError::Conflict(ref conflict)
                if conflict.code == "closed-members-policy-required"
        ));
        assert_eq!(engine.executor.calls, 0);
        assert!(!state_dir.join("packcaptain.lock").exists());
        let store = StateStore::open(&state_dir).unwrap();
        assert_eq!(store.load_snapshot().unwrap(), snapshot);
        assert_eq!(store.pending_journals().unwrap().len(), 1);
        drop(store);

        let no_target = engine
            .execute(&Command::SetSkillEnabled {
                skill_id: skill_a,
                targets: Vec::new(),
                enabled: true,
            })
            .unwrap_err();
        assert!(matches!(
            no_target,
            EngineError::Conflict(ref conflict) if conflict.code == "targets-required"
        ));
        assert_eq!(engine.executor.calls, 0);
        assert!(!state_dir.join("packcaptain.lock").exists());
    }
}

struct StateLock {
    _file: File,
}

impl StateLock {
    fn acquire(state_dir: &Path) -> Result<Self, std::io::Error> {
        fs::create_dir_all(state_dir)?;
        let lock_path = state_dir.join("packcaptain.lock");
        let file = OpenOptions::new()
            .create(true)
            .read(true)
            .write(true)
            .open(lock_path)?;
        file.lock_exclusive()?;
        Ok(Self { _file: file })
    }
}
