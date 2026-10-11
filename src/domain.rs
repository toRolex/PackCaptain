use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::path::{Path, PathBuf};
use std::str::FromStr;

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use uuid::Uuid;

#[derive(Clone, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(transparent)]
pub struct SkillId(String);

impl SkillId {
    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn from_source_path(path: &Path) -> Self {
        Self(Uuid::new_v5(&Uuid::NAMESPACE_URL, path.as_os_str().as_encoded_bytes()).to_string())
    }
}

impl From<&str> for SkillId {
    fn from(value: &str) -> Self {
        Self(value.to_owned())
    }
}

impl From<String> for SkillId {
    fn from(value: String) -> Self {
        Self(value)
    }
}

#[derive(Clone, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(transparent)]
pub struct PresetId(String);

impl PresetId {
    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn new() -> Self {
        Self(Uuid::new_v4().to_string())
    }
}

impl Default for PresetId {
    fn default() -> Self {
        Self::new()
    }
}

impl From<&str> for PresetId {
    fn from(value: &str) -> Self {
        Self(value.to_owned())
    }
}

#[derive(Clone, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(transparent)]
pub struct ProjectId(String);

impl ProjectId {
    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn new() -> Self {
        Self(Uuid::new_v4().to_string())
    }
}

impl Default for ProjectId {
    fn default() -> Self {
        Self::new()
    }
}

impl From<&str> for ProjectId {
    fn from(value: &str) -> Self {
        Self(value.to_owned())
    }
}

impl From<String> for ProjectId {
    fn from(value: String) -> Self {
        Self(value)
    }
}

#[derive(Clone, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum AgentId {
    ClaudeCode,
    Codex,
    Pi,
}

impl AgentId {
    fn as_key(&self) -> &'static str {
        match self {
            Self::ClaudeCode => "claude-code",
            Self::Codex => "codex",
            Self::Pi => "pi",
        }
    }

    fn parse_key(value: &str) -> Result<Self, TargetIdParseError> {
        match value {
            "claude-code" => Ok(Self::ClaudeCode),
            "codex" => Ok(Self::Codex),
            "pi" => Ok(Self::Pi),
            _ => Err(TargetIdParseError(format!("unknown agent '{value}'"))),
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(tag = "kind", content = "project", rename_all = "kebab-case")]
pub enum Scope {
    Global,
    Project(ProjectId),
}

#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct TargetId {
    pub agent: AgentId,
    pub scope: Scope,
}

impl fmt::Display for TargetId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.scope {
            Scope::Global => write!(formatter, "{}:global", self.agent.as_key()),
            Scope::Project(project_id) => {
                write!(
                    formatter,
                    "{}:project:{}",
                    self.agent.as_key(),
                    project_id.as_str()
                )
            }
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TargetIdParseError(String);

impl fmt::Display for TargetIdParseError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl std::error::Error for TargetIdParseError {}

impl FromStr for TargetId {
    type Err = TargetIdParseError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        let mut parts = value.split(':');
        let agent = AgentId::parse_key(parts.next().unwrap_or_default())?;
        let scope = match parts.next() {
            Some("global") if parts.next().is_none() => Scope::Global,
            Some("project") => {
                let project_id = parts
                    .next()
                    .filter(|part| !part.is_empty())
                    .ok_or_else(|| {
                        TargetIdParseError("project target ID is missing its project ID".to_owned())
                    })?;
                if parts.next().is_some() {
                    return Err(TargetIdParseError(
                        "project target ID contains an extra separator".to_owned(),
                    ));
                }
                Scope::Project(ProjectId::from(project_id))
            }
            Some(scope) => {
                return Err(TargetIdParseError(format!(
                    "unknown target scope '{scope}'"
                )));
            }
            None => {
                return Err(TargetIdParseError(
                    "target ID is missing its scope".to_owned(),
                ));
            }
        };
        Ok(Self { agent, scope })
    }
}

impl Serialize for TargetId {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&self.to_string())
    }
}

impl<'de> Deserialize<'de> for TargetId {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Self::from_str(&value).map_err(serde::de::Error::custom)
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum SkillAvailability {
    Available,
    Missing,
    Unsupported,
    Unreadable,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct SkillRecord {
    pub id: SkillId,
    pub name: String,
    pub source_path: PathBuf,
    pub source_fingerprint: String,
    pub availability: SkillAvailability,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum EntryOwnership {
    DirectDirectory,
    UnmanagedSymlink,
    PackCaptainLink,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct SkillEntry {
    pub path: PathBuf,
    pub source_path: PathBuf,
    pub entry_fingerprint: String,
    pub present: bool,
    pub active: bool,
    pub ownership: EntryOwnership,
    #[serde(default)]
    pub native_config_path: Option<PathBuf>,
    #[serde(default)]
    pub native_config_fingerprint: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct PresetDefinition {
    pub id: PresetId,
    pub name: String,
    pub revision: u64,
    pub members: BTreeSet<SkillId>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct AppliedPresetState {
    pub enabled: bool,
    pub applied_revision: u64,
    pub applied_members: BTreeSet<SkillId>,
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(default)]
pub struct TargetState {
    pub enabled_skills: BTreeSet<SkillId>,
    pub manual_open: BTreeSet<SkillId>,
    pub manual_closed: BTreeSet<SkillId>,
    pub presets: BTreeMap<PresetId, AppliedPresetState>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ProjectRecord {
    pub id: ProjectId,
    pub name: String,
    pub root: PathBuf,
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct Snapshot {
    pub skills: BTreeMap<SkillId, SkillRecord>,
    pub presets: BTreeMap<PresetId, PresetDefinition>,
    #[serde(default)]
    pub projects: BTreeMap<ProjectId, ProjectRecord>,
    #[serde(default)]
    pub entries: BTreeMap<TargetId, BTreeMap<SkillId, Vec<SkillEntry>>>,
    pub targets: BTreeMap<TargetId, TargetState>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum ClosedMembersPolicy {
    Keep,
    Restore,
    Unspecified,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "command", content = "args", rename_all = "kebab-case")]
pub enum Command {
    CreatePreset {
        preset_id: PresetId,
        name: String,
        members: BTreeSet<SkillId>,
    },
    SetPresetEnabled {
        preset_id: PresetId,
        targets: Vec<TargetId>,
        enabled: bool,
    },
    SetSkillEnabled {
        skill_id: SkillId,
        targets: Vec<TargetId>,
        enabled: bool,
    },
    UpdatePreset {
        preset_id: PresetId,
        expected_revision: u64,
        members: BTreeSet<SkillId>,
        sync_targets: Vec<TargetId>,
        closed_members: ClosedMembersPolicy,
    },
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "effect", content = "args", rename_all = "kebab-case")]
pub enum Effect {
    SetSkillEnabled {
        target: TargetId,
        skill: SkillId,
        enabled: bool,
    },
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct Plan {
    pub effects: Vec<Effect>,
    pub preset_updates: BTreeMap<PresetId, PresetDefinition>,
    pub target_updates: BTreeMap<TargetId, TargetState>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ConflictReport {
    pub code: String,
    pub message: String,
    pub affected_targets: Vec<TargetId>,
    pub affected_skills: Vec<SkillId>,
}

pub fn plan(command: &Command, snapshot: &Snapshot) -> Result<Plan, ConflictReport> {
    match command {
        Command::CreatePreset {
            preset_id,
            name,
            members,
        } => plan_create_preset(preset_id, name, members, snapshot),
        Command::SetPresetEnabled {
            preset_id,
            targets,
            enabled,
        } => plan_set_preset_enabled(preset_id, targets, *enabled, snapshot),
        Command::SetSkillEnabled {
            skill_id,
            targets,
            enabled,
        } => plan_set_skill_enabled(skill_id, targets, *enabled, snapshot),
        Command::UpdatePreset {
            preset_id,
            expected_revision,
            members,
            sync_targets,
            closed_members,
        } => plan_update_preset(
            preset_id,
            *expected_revision,
            members,
            sync_targets,
            *closed_members,
            snapshot,
        ),
    }
}

fn plan_create_preset(
    preset_id: &PresetId,
    name: &str,
    members: &BTreeSet<SkillId>,
    snapshot: &Snapshot,
) -> Result<Plan, ConflictReport> {
    if name.trim().is_empty() {
        return Err(ConflictReport {
            code: "preset-name-required".to_owned(),
            message: "preset name cannot be empty".to_owned(),
            affected_targets: Vec::new(),
            affected_skills: Vec::new(),
        });
    }
    if snapshot.presets.contains_key(preset_id) {
        return Err(ConflictReport {
            code: "preset-id-exists".to_owned(),
            message: format!("preset '{}' already exists", preset_id.as_str()),
            affected_targets: Vec::new(),
            affected_skills: Vec::new(),
        });
    }
    for skill_id in members {
        if !snapshot.skills.contains_key(skill_id) {
            return Err(ConflictReport {
                code: "skill-not-found".to_owned(),
                message: format!("skill '{}' does not exist", skill_id.as_str()),
                affected_targets: Vec::new(),
                affected_skills: vec![skill_id.clone()],
            });
        }
    }

    let mut plan = Plan::default();
    plan.preset_updates.insert(
        preset_id.clone(),
        PresetDefinition {
            id: preset_id.clone(),
            name: name.trim().to_owned(),
            revision: 1,
            members: members.clone(),
        },
    );
    Ok(plan)
}

fn require_targets(
    targets: &[TargetId],
    affected_skills: Vec<SkillId>,
) -> Result<(), ConflictReport> {
    if targets.is_empty() {
        return Err(ConflictReport {
            code: "targets-required".to_owned(),
            message: "write commands require at least one explicit target".to_owned(),
            affected_targets: Vec::new(),
            affected_skills,
        });
    }
    Ok(())
}

fn plan_update_preset(
    preset_id: &PresetId,
    expected_revision: u64,
    members: &BTreeSet<SkillId>,
    sync_targets: &[TargetId],
    closed_members: ClosedMembersPolicy,
    snapshot: &Snapshot,
) -> Result<Plan, ConflictReport> {
    require_targets(sync_targets, Vec::new())?;
    let current = snapshot
        .presets
        .get(preset_id)
        .ok_or_else(|| ConflictReport {
            code: "preset-not-found".to_owned(),
            message: format!("preset '{}' does not exist", preset_id.as_str()),
            affected_targets: sync_targets.to_vec(),
            affected_skills: Vec::new(),
        })?;
    if current.revision != expected_revision {
        return Err(ConflictReport {
            code: "stale-preset-revision".to_owned(),
            message: format!(
                "preset '{}' changed from expected revision {} to {}",
                preset_id.as_str(),
                expected_revision,
                current.revision
            ),
            affected_targets: sync_targets.to_vec(),
            affected_skills: Vec::new(),
        });
    }
    for skill_id in members {
        if !snapshot.skills.contains_key(skill_id) {
            return Err(ConflictReport {
                code: "skill-not-found".to_owned(),
                message: format!("skill '{}' does not exist", skill_id.as_str()),
                affected_targets: sync_targets.to_vec(),
                affected_skills: vec![skill_id.clone()],
            });
        }
    }

    let selected_targets: BTreeSet<_> = sync_targets.iter().cloned().collect();
    let mut closed_conflicts = BTreeMap::<TargetId, BTreeSet<SkillId>>::new();
    for target_id in &selected_targets {
        let target_state = snapshot
            .targets
            .get(target_id)
            .ok_or_else(|| ConflictReport {
                code: "target-not-found".to_owned(),
                message: "target does not exist".to_owned(),
                affected_targets: vec![target_id.clone()],
                affected_skills: Vec::new(),
            })?;
        let Some(applied_state) = target_state.presets.get(preset_id) else {
            continue;
        };
        if !applied_state.enabled {
            continue;
        }
        let relevant_closed: BTreeSet<_> = applied_state
            .applied_members
            .intersection(members)
            .filter(|skill_id| {
                target_state.manual_closed.contains(*skill_id)
                    && snapshot
                        .skills
                        .get(*skill_id)
                        .is_some_and(|skill| skill.availability == SkillAvailability::Available)
            })
            .cloned()
            .collect();
        if !relevant_closed.is_empty() {
            closed_conflicts.insert(target_id.clone(), relevant_closed);
        }
    }

    if !closed_conflicts.is_empty() && closed_members == ClosedMembersPolicy::Unspecified {
        let affected_targets = closed_conflicts.keys().cloned().collect();
        let affected_skills = closed_conflicts.values().flatten().cloned().collect();
        return Err(ConflictReport {
            code: "closed-members-policy-required".to_owned(),
            message: "choose whether to keep or restore relevant manually closed members"
                .to_owned(),
            affected_targets,
            affected_skills,
        });
    }

    let mut next_definition = current.clone();
    if next_definition.members != *members {
        next_definition.revision =
            next_definition
                .revision
                .checked_add(1)
                .ok_or_else(|| ConflictReport {
                    code: "preset-revision-overflow".to_owned(),
                    message: "preset revision cannot be incremented".to_owned(),
                    affected_targets: sync_targets.to_vec(),
                    affected_skills: Vec::new(),
                })?;
        next_definition.members = members.clone();
    }

    let mut plan = Plan::default();
    plan.preset_updates
        .insert(preset_id.clone(), next_definition.clone());

    for target_id in selected_targets {
        let mut target_state = snapshot.targets[&target_id].clone();
        let Some(applied_state) = target_state.presets.get(preset_id).cloned() else {
            continue;
        };
        if !applied_state.enabled {
            continue;
        }

        let old_members = applied_state.applied_members;
        let added_members: BTreeSet<_> = members.difference(&old_members).cloned().collect();
        let removed_members: BTreeSet<_> = old_members.difference(members).cloned().collect();
        let active_elsewhere: BTreeSet<_> = target_state
            .presets
            .iter()
            .filter(|(other_id, state)| *other_id != preset_id && state.enabled)
            .flat_map(|(_, state)| state.applied_members.iter().cloned())
            .collect();
        let affected_members: BTreeSet<_> = old_members.union(members).cloned().collect();
        for skill_id in &added_members {
            target_state.manual_closed.remove(skill_id);
        }

        for skill_id in affected_members {
            if !snapshot
                .skills
                .get(&skill_id)
                .is_some_and(|skill| skill.availability == SkillAvailability::Available)
            {
                continue;
            }
            let is_retained = old_members.contains(&skill_id) && members.contains(&skill_id);
            let should_enable = if added_members.contains(&skill_id) {
                true
            } else if is_retained {
                if target_state.manual_closed.contains(&skill_id) {
                    match closed_members {
                        ClosedMembersPolicy::Keep => false,
                        ClosedMembersPolicy::Restore => {
                            target_state.manual_closed.remove(&skill_id);
                            true
                        }
                        ClosedMembersPolicy::Unspecified => true,
                    }
                } else {
                    true
                }
            } else if removed_members.contains(&skill_id) {
                (active_elsewhere.contains(&skill_id)
                    || target_state.manual_open.contains(&skill_id))
                    && !target_state.manual_closed.contains(&skill_id)
            } else {
                false
            };

            let was_enabled = target_state.enabled_skills.contains(&skill_id);
            if should_enable {
                target_state.enabled_skills.insert(skill_id.clone());
            } else {
                target_state.enabled_skills.remove(&skill_id);
            }
            if was_enabled != should_enable {
                plan.effects.push(Effect::SetSkillEnabled {
                    target: target_id.clone(),
                    skill: skill_id,
                    enabled: should_enable,
                });
            }
        }

        target_state.presets.insert(
            preset_id.clone(),
            AppliedPresetState {
                enabled: true,
                applied_revision: next_definition.revision,
                applied_members: members_with_deferred_removals(members, &old_members, snapshot),
            },
        );
        plan.target_updates.insert(target_id, target_state);
    }

    Ok(plan)
}

fn members_with_deferred_removals(
    members: &BTreeSet<SkillId>,
    old_members: &BTreeSet<SkillId>,
    snapshot: &Snapshot,
) -> BTreeSet<SkillId> {
    members
        .iter()
        .chain(old_members.iter().filter(|skill_id| {
            !snapshot
                .skills
                .get(*skill_id)
                .is_some_and(|skill| skill.availability == SkillAvailability::Available)
        }))
        .cloned()
        .collect()
}

fn plan_set_skill_enabled(
    skill_id: &SkillId,
    targets: &[TargetId],
    enabled: bool,
    snapshot: &Snapshot,
) -> Result<Plan, ConflictReport> {
    require_targets(targets, vec![skill_id.clone()])?;
    let skill = snapshot
        .skills
        .get(skill_id)
        .ok_or_else(|| ConflictReport {
            code: "skill-not-found".to_owned(),
            message: format!("skill '{}' does not exist", skill_id.as_str()),
            affected_targets: targets.to_vec(),
            affected_skills: vec![skill_id.clone()],
        })?;
    if skill.availability != SkillAvailability::Available {
        return Err(ConflictReport {
            code: "skill-unavailable".to_owned(),
            message: format!("skill '{}' is not available", skill.name),
            affected_targets: targets.to_vec(),
            affected_skills: vec![skill_id.clone()],
        });
    }

    let mut plan = Plan::default();
    let unique_targets: BTreeSet<_> = targets.iter().cloned().collect();
    for target_id in unique_targets {
        let mut target_state =
            snapshot
                .targets
                .get(&target_id)
                .cloned()
                .ok_or_else(|| ConflictReport {
                    code: "target-not-found".to_owned(),
                    message: "target does not exist".to_owned(),
                    affected_targets: vec![target_id.clone()],
                    affected_skills: vec![skill_id.clone()],
                })?;

        let was_enabled = target_state.enabled_skills.contains(skill_id);
        if enabled {
            target_state.manual_closed.remove(skill_id);
            target_state.manual_open.insert(skill_id.clone());
            target_state.enabled_skills.insert(skill_id.clone());
        } else {
            target_state.manual_open.remove(skill_id);
            target_state.manual_closed.insert(skill_id.clone());
            target_state.enabled_skills.remove(skill_id);
        }
        if was_enabled != enabled {
            plan.effects.push(Effect::SetSkillEnabled {
                target: target_id.clone(),
                skill: skill_id.clone(),
                enabled,
            });
        }
        plan.target_updates.insert(target_id, target_state);
    }
    Ok(plan)
}

fn plan_set_preset_enabled(
    preset_id: &PresetId,
    targets: &[TargetId],
    enabled: bool,
    snapshot: &Snapshot,
) -> Result<Plan, ConflictReport> {
    require_targets(targets, Vec::new())?;
    let definition = snapshot
        .presets
        .get(preset_id)
        .ok_or_else(|| ConflictReport {
            code: "preset-not-found".to_owned(),
            message: format!("preset '{}' does not exist", preset_id.as_str()),
            affected_targets: targets.to_vec(),
            affected_skills: Vec::new(),
        })?;
    let mut plan = Plan::default();
    let unique_targets: BTreeSet<_> = targets.iter().cloned().collect();

    for target_id in unique_targets {
        let mut target_state =
            snapshot
                .targets
                .get(&target_id)
                .cloned()
                .ok_or_else(|| ConflictReport {
                    code: "target-not-found".to_owned(),
                    message: "target does not exist".to_owned(),
                    affected_targets: vec![target_id.clone()],
                    affected_skills: Vec::new(),
                })?;

        let existing_preset_state = target_state.presets.get(preset_id).cloned();
        let applied_members = existing_preset_state
            .as_ref()
            .map(|state| state.applied_members.clone())
            .unwrap_or_else(|| definition.members.clone());

        let mut active_members = BTreeSet::new();
        for (other_preset_id, state) in &target_state.presets {
            if *other_preset_id != *preset_id && state.enabled {
                active_members.extend(state.applied_members.iter().cloned());
            }
        }

        if enabled {
            for skill_id in &definition.members {
                target_state.manual_closed.remove(skill_id);
            }
        } else {
            for skill_id in &applied_members {
                target_state.manual_open.remove(skill_id);
            }
        }

        let touched_members: BTreeSet<_> = if enabled {
            definition
                .members
                .union(&applied_members)
                .cloned()
                .collect()
        } else {
            applied_members.clone()
        };

        for skill_id in touched_members {
            if !snapshot
                .skills
                .get(&skill_id)
                .is_some_and(|skill| skill.availability == SkillAvailability::Available)
            {
                continue;
            }
            let should_enable = if enabled {
                if definition.members.contains(&skill_id) {
                    true
                } else {
                    (active_members.contains(&skill_id)
                        || target_state.manual_open.contains(&skill_id))
                        && !target_state.manual_closed.contains(&skill_id)
                }
            } else {
                active_members.contains(&skill_id)
                    && !target_state.manual_closed.contains(&skill_id)
            };
            let was_enabled = target_state.enabled_skills.contains(&skill_id);
            if should_enable {
                target_state.enabled_skills.insert(skill_id.clone());
                if enabled {
                    target_state.manual_closed.remove(&skill_id);
                }
            } else {
                target_state.enabled_skills.remove(&skill_id);
            }
            if was_enabled != should_enable {
                plan.effects.push(Effect::SetSkillEnabled {
                    target: target_id.clone(),
                    skill: skill_id,
                    enabled: should_enable,
                });
            }
        }

        let next_applied_members = if enabled {
            members_with_deferred_removals(&definition.members, &applied_members, snapshot)
        } else {
            existing_preset_state
                .as_ref()
                .map(|state| state.applied_members.clone())
                .unwrap_or_else(|| definition.members.clone())
        };
        target_state.presets.insert(
            preset_id.clone(),
            AppliedPresetState {
                enabled,
                applied_revision: if enabled {
                    definition.revision
                } else {
                    existing_preset_state
                        .as_ref()
                        .map(|state| state.applied_revision)
                        .unwrap_or(definition.revision)
                },
                applied_members: next_applied_members,
            },
        );
        plan.target_updates.insert(target_id, target_state);
    }

    Ok(plan)
}
