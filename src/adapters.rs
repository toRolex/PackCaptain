use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, File, OpenOptions};
use std::io::{self, BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command as ProcessCommand, Stdio};

use serde_json::Value as JsonValue;
use sha2::{Digest, Sha256};
use toml::Value as TomlValue;
use uuid::Uuid;
use walkdir::WalkDir;

use crate::domain::{
    AgentId, Effect, EntryOwnership, ProjectRecord, Scope, SkillAvailability, SkillEntry, SkillId,
    SkillRecord, Snapshot, TargetId,
};
use crate::engine::{
    ApplyFailure, EffectExecutor, ForwardReceiptContext, MutationReceipt, ReceiptAction,
    ReceiptIdentity, ReceiptKey, ReceiptStage, RecoveryDirection, SymlinkIdentity, TargetMutation,
};

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct DiscoveryReport {
    pub skills: BTreeMap<SkillId, SkillRecord>,
    pub entries: BTreeMap<TargetId, BTreeMap<SkillId, Vec<SkillEntry>>>,
    pub observed_enabled: BTreeMap<TargetId, BTreeSet<SkillId>>,
    pub diagnostics: Vec<String>,
    pub target_errors: BTreeMap<TargetId, DiscoveryError>,
}

#[derive(Clone, Debug, Default)]
pub struct NativeAdapters;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProjectCandidate {
    pub name: String,
    pub path: PathBuf,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DiscoveryError(pub String);

impl std::fmt::Display for DiscoveryError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl std::error::Error for DiscoveryError {}

impl NativeAdapters {
    pub fn scan_candidates(root: &Path) -> Result<Vec<ProjectCandidate>, DiscoveryError> {
        if !root.is_absolute() {
            return Err(DiscoveryError(format!(
                "candidate scan root must be absolute: {}",
                root.display()
            )));
        }
        let root = fs::canonicalize(root).map_err(|error| {
            DiscoveryError(format!(
                "cannot open candidate root {}: {error}",
                root.display()
            ))
        })?;
        if !root.is_dir() {
            return Err(DiscoveryError(format!(
                "candidate scan root is not a directory: {}",
                root.display()
            )));
        }
        let mut candidates = BTreeMap::<PathBuf, ProjectCandidate>::new();
        for item in WalkDir::new(&root)
            .follow_links(false)
            .max_depth(4)
            .sort_by_file_name()
        {
            let item = item.map_err(|error| {
                DiscoveryError(format!(
                    "scan failed at {}: {error}",
                    error.path().unwrap_or(&root).display()
                ))
            })?;
            if !item.file_type().is_dir() {
                continue;
            }
            let path = item.path();
            if path != root
                && path
                    .file_name()
                    .is_some_and(|name| name.to_string_lossy().starts_with('.'))
            {
                continue;
            }
            if !looks_like_project(path) {
                continue;
            }
            let path = fs::canonicalize(path).map_err(|error| {
                DiscoveryError(format!("cannot resolve candidate path: {error}"))
            })?;
            let name = path
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("Project")
                .to_owned();
            candidates.insert(path.clone(), ProjectCandidate { name, path });
        }
        Ok(candidates.into_values().collect())
    }
}

fn looks_like_project(path: &Path) -> bool {
    [
        ".git",
        ".claude/skills",
        ".agents/skills",
        ".codex/skills",
        ".pi/skills",
        "Cargo.toml",
        "package.json",
        "pyproject.toml",
        "go.mod",
    ]
    .iter()
    .any(|marker| fs::symlink_metadata(path.join(marker)).is_ok())
}

impl DiscoveryReport {
    pub fn merge_into(&self, previous: &Snapshot) -> Snapshot {
        let mut merged = previous.clone();
        for (skill_id, skill) in &self.skills {
            merged.skills.insert(skill_id.clone(), skill.clone());
        }
        for (target, entries) in &self.entries {
            if self.target_errors.contains_key(target) {
                continue;
            }
            merged.entries.insert(target.clone(), entries.clone());
            let state = merged.targets.entry(target.clone()).or_default();
            state.enabled_skills = self
                .observed_enabled
                .get(target)
                .cloned()
                .unwrap_or_default();
        }
        merged
    }
}

impl NativeAdapters {
    pub fn discover(&self, agent_home: &Path, snapshot: &Snapshot) -> DiscoveryReport {
        let mut report = DiscoveryReport::default();
        let agents = [AgentId::ClaudeCode, AgentId::Codex, AgentId::Pi];
        for agent in agents {
            let global_target = TargetId {
                agent: agent.clone(),
                scope: Scope::Global,
            };
            self.discover_target(agent_home, &global_target, None, snapshot, &mut report);
            for project in snapshot.projects.values() {
                let target = TargetId {
                    agent: agent.clone(),
                    scope: Scope::Project(project.id.clone()),
                };
                self.discover_target(agent_home, &target, Some(project), snapshot, &mut report);
            }
        }
        for (skill_id, skill) in &snapshot.skills {
            report
                .skills
                .entry(skill_id.clone())
                .or_insert(skill.clone());
        }
        report
    }

    fn discover_target(
        &self,
        agent_home: &Path,
        target: &TargetId,
        project: Option<&ProjectRecord>,
        snapshot: &Snapshot,
        report: &mut DiscoveryReport,
    ) {
        let mut local = DiscoveryReport::default();
        let result = self
            .scan_target(agent_home, target, project, snapshot, &mut local)
            .and_then(|()| preserve_disabled_owned_entries(snapshot, target, &mut local));
        report.diagnostics.extend(local.diagnostics);
        match result {
            Ok(()) => {
                report.skills.extend(local.skills);
                report.entries.extend(local.entries);
                report.observed_enabled.extend(local.observed_enabled);
            }
            Err(error) => {
                report.diagnostics.push(format!("{target}: {error}"));
                report.target_errors.insert(target.clone(), error);
            }
        }
    }

    fn scan_target(
        &self,
        agent_home: &Path,
        target: &TargetId,
        project: Option<&ProjectRecord>,
        snapshot: &Snapshot,
        report: &mut DiscoveryReport,
    ) -> Result<(), DiscoveryError> {
        let config_path = native_config_path(&target.agent, agent_home, project);
        let config =
            NativeConfig::read(&target.agent, config_path.as_deref()).map_err(DiscoveryError)?;
        report.entries.entry(target.clone()).or_default();
        report.observed_enabled.entry(target.clone()).or_default();
        let roots = skill_roots(&target.agent, agent_home, project);
        for root in roots {
            let Some(metadata) = scan_optional(fs::metadata(&root), &root)? else {
                continue;
            };
            if !metadata.is_dir() {
                return Err(DiscoveryError(format!(
                    "skill root is not a directory: {}",
                    root.display()
                )));
            }
            let walker = WalkDir::new(&root).follow_links(false).sort_by_file_name();
            for item in walker {
                let item = item.map_err(|error| {
                    DiscoveryError(format!(
                        "cannot scan {}: {error}",
                        error.path().unwrap_or(&root).display()
                    ))
                })?;
                if item.depth() == 0 {
                    continue;
                }
                let path = item.path();
                let is_symlink = item.file_type().is_symlink();
                let is_skill_dir = if is_symlink || item.file_type().is_dir() {
                    scan_definition_exists(&path.join("SKILL.md"))?
                } else {
                    item.file_type().is_file()
                        && path.file_name().is_some_and(|name| name == "SKILL.md")
                };
                if !is_skill_dir {
                    continue;
                }
                let entry_path = if path.file_name().is_some_and(|name| name == "SKILL.md") {
                    match path.parent() {
                        Some(parent) => parent,
                        None => continue,
                    }
                } else {
                    path
                };
                if !scan_definition_exists(&entry_path.join("SKILL.md"))? {
                    continue;
                }
                let owned = snapshot
                    .entries
                    .get(target)
                    .into_iter()
                    .flat_map(|by_skill| by_skill.values())
                    .flatten()
                    .find(|entry| {
                        entry.path == entry_path
                            && entry.ownership == EntryOwnership::PackCaptainLink
                    });
                if let Some(owned) = owned {
                    let Some(fingerprint) = scan_entry_fingerprint(entry_path)? else {
                        continue;
                    };
                    let Some(source) = scan_optional(fs::canonicalize(entry_path), entry_path)?
                    else {
                        continue;
                    };
                    let unchanged =
                        fingerprint == owned.entry_fingerprint && source == owned.source_path;
                    if !unchanged {
                        report.diagnostics.push(format!(
                            "managed skill entry changed externally: {}",
                            entry_path.display()
                        ));
                        continue;
                    }
                }
                let Some(source_path) = scan_optional(fs::canonicalize(entry_path), entry_path)?
                else {
                    continue;
                };
                let skill_id = SkillId::from_source_path(&source_path);
                let skill_file = entry_path.join("SKILL.md");
                let Some(contents) = scan_optional(fs::read_to_string(&skill_file), &skill_file)?
                else {
                    continue;
                };
                let name = parse_skill_name(&contents).unwrap_or_else(|| {
                    entry_path
                        .file_name()
                        .and_then(|name| name.to_str())
                        .unwrap_or("unnamed-skill")
                        .to_owned()
                });
                let source_fingerprint = match fingerprint_directory(&source_path) {
                    Ok(fingerprint) => fingerprint,
                    Err(error) => {
                        if error.kind() == io::ErrorKind::NotFound
                            && !scan_definition_exists(&source_path.join("SKILL.md"))?
                        {
                            continue;
                        }
                        return Err(scan_error(&source_path, error));
                    }
                };
                let config_fingerprint = config.fingerprint.clone();
                let prior = snapshot
                    .entries
                    .get(target)
                    .and_then(|entries| entries.get(&skill_id))
                    .and_then(|entries| entries.iter().find(|entry| entry.path == entry_path));
                let ownership = prior.map_or_else(
                    || {
                        if is_symlink {
                            EntryOwnership::UnmanagedSymlink
                        } else {
                            EntryOwnership::DirectDirectory
                        }
                    },
                    |entry| entry.ownership,
                );
                let Some(entry_fingerprint) = scan_entry_fingerprint(entry_path)? else {
                    continue;
                };
                let skill = SkillRecord {
                    id: skill_id.clone(),
                    name,
                    source_path: source_path.clone(),
                    source_fingerprint,
                    availability: SkillAvailability::Available,
                };
                report.skills.insert(skill_id.clone(), skill.clone());
                let active = !config
                    .disables(&skill, entry_path, &source_path)
                    .map_err(DiscoveryError)?
                    && !prior.is_some_and(|entry| {
                        entry.ownership == EntryOwnership::PackCaptainLink
                            && entry.present
                            && !entry.active
                            && snapshot.entries.iter().any(|(other, by_skill)| {
                                other != target
                                    && by_skill.get(&skill_id).is_some_and(|entries| {
                                        entries.iter().any(|peer| {
                                            peer.path == entry.path
                                                && peer.source_path == entry.source_path
                                        })
                                    })
                            })
                    });
                let entry = SkillEntry {
                    path: entry_path.to_path_buf(),
                    source_path,
                    entry_fingerprint,
                    present: true,
                    active,
                    ownership,
                    native_config_path: config_path.clone(),
                    native_config_fingerprint: config_fingerprint,
                };
                let entries = report
                    .entries
                    .entry(target.clone())
                    .or_default()
                    .entry(skill_id.clone())
                    .or_default();
                if !entries.iter().any(|current| current.path == entry.path) {
                    entries.push(entry);
                }
                if active {
                    report
                        .observed_enabled
                        .entry(target.clone())
                        .or_default()
                        .insert(skill_id);
                }
            }
        }
        Ok(())
    }
}

fn scan_error(path: &Path, error: io::Error) -> DiscoveryError {
    DiscoveryError(format!(
        "cannot inspect skill path {}: {error}",
        path.display()
    ))
}

fn scan_optional<T>(result: io::Result<T>, path: &Path) -> Result<Option<T>, DiscoveryError> {
    match result {
        Ok(value) => Ok(Some(value)),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(scan_error(path, error)),
    }
}

fn scan_definition_exists(path: &Path) -> Result<bool, DiscoveryError> {
    Ok(scan_optional(fs::metadata(path), path)?.is_some_and(|metadata| metadata.is_file()))
}

fn scan_entry_fingerprint(path: &Path) -> Result<Option<String>, DiscoveryError> {
    match fingerprint_entry(path) {
        Ok(fingerprint) => Ok(Some(fingerprint)),
        Err(error) => {
            if error.kind() == io::ErrorKind::NotFound
                && (scan_optional(fs::symlink_metadata(path), path)?.is_none()
                    || !scan_definition_exists(&path.join("SKILL.md"))?)
            {
                return Ok(None);
            }
            Err(scan_error(path, error))
        }
    }
}

impl EffectExecutor for NativeAdapters {
    fn prepare_target(
        &self,
        mutation: TargetMutation<'_>,
        snapshot: &Snapshot,
        agent_home: &Path,
    ) -> Result<BTreeMap<SkillId, Vec<SkillEntry>>, String> {
        let mut planned = mutation.entries_before.clone();
        for effect in mutation.effects {
            let skill_id = effect_skill(effect);
            let skill = snapshot
                .skills
                .get(skill_id)
                .ok_or_else(|| format!("skill '{}' is not in the inventory", skill_id.as_str()))?;
            if skill.availability != SkillAvailability::Available {
                return Err(format!("skill '{}' is not available", skill.name));
            }
            verify_source(skill)?;
            let desired = effect_enabled(effect);
            let Some(routes) = planned.get_mut(skill_id) else {
                if !desired {
                    return Err(format!(
                        "no target-local entry exists for skill '{}'",
                        skill.name
                    ));
                }
                let root = primary_skill_root(mutation.target, snapshot, agent_home)?;
                ensure_no_symlink_components(&root)?;
                let path = root.join(format!("pc-{}", skill.id.as_str()));
                if fs::symlink_metadata(&path).is_ok() {
                    return Err(format!(
                        "managed skill entry path is occupied by an unowned file: {}",
                        path.display()
                    ));
                }
                planned.insert(
                    skill_id.clone(),
                    vec![SkillEntry {
                        path,
                        source_path: skill.source_path.clone(),
                        entry_fingerprint: "pending".to_owned(),
                        present: true,
                        active: true,
                        ownership: EntryOwnership::PackCaptainLink,
                        native_config_path: None,
                        native_config_fingerprint: None,
                    }],
                );
                continue;
            };

            reject_mixed_routes(routes)?;
            for route in routes.iter_mut() {
                verify_route(route, skill)?;
                if route.ownership != EntryOwnership::PackCaptainLink {
                    let project = match &mutation.target.scope {
                        Scope::Global => None,
                        Scope::Project(project_id) => snapshot.projects.get(project_id),
                    };
                    ensure_native_scope_safe(mutation.target, skill, snapshot)?;
                    let config_path = route
                        .native_config_path
                        .clone()
                        .or_else(|| native_config_path(&mutation.target.agent, agent_home, project))
                        .ok_or_else(|| "native config path is unavailable".to_owned())?;
                    let actual_config_fingerprint = fingerprint_file_if_present(&config_path)
                        .map_err(|error| format!("cannot inspect native config: {error}"))?;
                    if route.native_config_fingerprint != actual_config_fingerprint {
                        return Err(format!(
                            "native config changed after discovery: {}",
                            config_path.display()
                        ));
                    }
                    route.native_config_path = Some(config_path);
                }
                if route.ownership == EntryOwnership::PackCaptainLink {
                    if desired && !route.present {
                        route.entry_fingerprint = "pending".to_owned();
                    }
                    route.present = desired
                        || shared_entry_needed_elsewhere(
                            mutation.target,
                            skill_id,
                            &route.path,
                            snapshot,
                        );
                }
                route.active = desired;
            }
        }
        Ok(planned)
    }

    fn apply_target(
        &mut self,
        mutation: TargetMutation<'_>,
        snapshot: &Snapshot,
        agent_home: &Path,
    ) -> Result<BTreeMap<SkillId, Vec<SkillEntry>>, ApplyFailure> {
        let mut updated = mutation.entries_after.clone();
        for effect in mutation.effects {
            if let Some(routes) = updated.get_mut(effect_skill(effect)) {
                for route in routes {
                    if route.ownership == EntryOwnership::PackCaptainLink {
                        route.present = effect_enabled(effect)
                            || shared_entry_needed_elsewhere(
                                mutation.target,
                                effect_skill(effect),
                                &route.path,
                                snapshot,
                            );
                    }
                }
            }
        }
        let mut link_changes = Vec::new();
        let mut native_changes = Vec::new();

        for effect in mutation.effects {
            let skill_id = effect_skill(effect);
            let skill = snapshot
                .skills
                .get(skill_id)
                .ok_or_else(|| format!("skill '{}' is not in the inventory", skill_id.as_str()))?;
            verify_source(skill)?;
            let mut routes_before = mutation
                .entries_before
                .get(skill_id)
                .cloned()
                .unwrap_or_default();
            let routes_after = updated.get(skill_id).cloned().unwrap_or_default();
            reject_mixed_routes(&routes_before)?;
            reject_mixed_routes(&routes_after)?;
            if let Some(context) = mutation.receipt_context {
                for route in &routes_after {
                    if route.ownership == EntryOwnership::PackCaptainLink
                        && route.active
                        && route.source_path != skill.source_path
                    {
                        return Err("receipt route source does not match skill"
                            .to_owned()
                            .into());
                    }
                    if route.ownership == EntryOwnership::PackCaptainLink
                        && route.active
                        && let Some(receipt) =
                            load_create_receipt(context, mutation.target, skill, &route.path)?
                    {
                        verify_anchor_pin(&receipt)?;
                        match fs::symlink_metadata(&route.path) {
                            Ok(_) => verify_public_pin(&receipt)?,
                            Err(error)
                                if error.kind() == io::ErrorKind::NotFound
                                    && receipt.stage == ReceiptStage::Prepared => {}
                            Err(error) => return Err(error.to_string().into()),
                        }
                    }
                }
            }
            let verified_after = !routes_after.is_empty()
                && routes_after.iter().all(|route| {
                    route.ownership == EntryOwnership::PackCaptainLink
                        && route.active == effect_enabled(effect)
                        && route.present
                            == (effect_enabled(effect)
                                || shared_entry_needed_elsewhere(
                                    mutation.target,
                                    skill_id,
                                    &route.path,
                                    snapshot,
                                ))
                        && route.entry_fingerprint != "pending"
                        && verify_route(route, skill).is_ok()
                });
            if verified_after {
                routes_before = routes_after.clone();
            } else {
                for route in &routes_before {
                    if let Err(error) = verify_route(route, skill) {
                        let recovered = if let Some(context) = mutation.receipt_context {
                            if route.ownership == EntryOwnership::PackCaptainLink
                                && route.source_path == skill.source_path
                                && !route.present
                                && effect_enabled(effect)
                            {
                                load_create_receipt(context, mutation.target, skill, &route.path)?
                                    .is_some_and(|receipt| verify_public_pin(&receipt).is_ok())
                            } else {
                                false
                            }
                        } else {
                            false
                        };
                        if !recovered {
                            return Err(error.into());
                        }
                    }
                }
            }
            if routes_before
                .iter()
                .chain(&routes_after)
                .all(|route| route.ownership == EntryOwnership::PackCaptainLink)
            {
                link_changes.push((skill.clone(), routes_before, routes_after));
            } else {
                native_changes.push((skill.clone(), routes_before, routes_after));
            }
        }

        apply_link_changes(&link_changes, &mutation)?;
        let config_fingerprints = if native_changes.is_empty() {
            BTreeMap::new()
        } else {
            apply_native_changes(&mutation, &native_changes, snapshot, agent_home)?
        };

        for effect in mutation.effects {
            let skill_id = effect_skill(effect);
            let desired = effect_enabled(effect);
            if let Some(routes) = updated.get_mut(skill_id) {
                for route in routes {
                    route.active = desired;
                    if route.ownership == EntryOwnership::PackCaptainLink {
                        if mutation.receipt_context.is_some()
                            && route.present
                            && route.native_config_path.is_none()
                        {
                            let project = match &mutation.target.scope {
                                Scope::Global => None,
                                Scope::Project(id) => snapshot.projects.get(id),
                            };
                            route.native_config_path =
                                native_config_path(&mutation.target.agent, agent_home, project);
                            if let Some(path) = &route.native_config_path {
                                route.native_config_fingerprint = fingerprint_file_if_present(path)
                                    .map_err(|error| error.to_string())?;
                            }
                        }
                        route.entry_fingerprint = if route.present {
                            fingerprint_entry(&route.path).map_err(|error| error.to_string())?
                        } else {
                            "missing".to_owned()
                        };
                    }
                    if let Some(config_path) = &route.native_config_path
                        && let Some(fingerprint) = config_fingerprints.get(config_path)
                    {
                        route.native_config_fingerprint = fingerprint.clone();
                    }
                }
            }
        }
        Ok(updated)
    }
}

fn effect_skill(effect: &Effect) -> &SkillId {
    match effect {
        Effect::SetSkillEnabled { skill, .. } => skill,
    }
}

fn effect_enabled(effect: &Effect) -> bool {
    match effect {
        Effect::SetSkillEnabled { enabled, .. } => *enabled,
    }
}

fn primary_skill_root(
    target: &TargetId,
    snapshot: &Snapshot,
    agent_home: &Path,
) -> Result<PathBuf, String> {
    let project = match &target.scope {
        Scope::Global => None,
        Scope::Project(project_id) => Some(
            snapshot
                .projects
                .get(project_id)
                .ok_or_else(|| format!("project '{}' is not registered", project_id.as_str()))?,
        ),
    };
    skill_roots(&target.agent, agent_home, project)
        .into_iter()
        .next()
        .ok_or_else(|| "agent has no skill root".to_owned())
}

fn verify_source(skill: &SkillRecord) -> Result<(), String> {
    let canonical = fs::canonicalize(&skill.source_path).map_err(|error| {
        format!(
            "skill source is no longer available at {}: {error}",
            skill.source_path.display()
        )
    })?;
    if canonical != skill.source_path {
        return Err(format!(
            "skill source path changed: {}",
            skill.source_path.display()
        ));
    }
    let current = fingerprint_directory(&canonical).map_err(|error| {
        format!(
            "cannot verify skill source {}: {error}",
            canonical.display()
        )
    })?;
    if current != skill.source_fingerprint {
        return Err(format!(
            "skill source changed after discovery: {}",
            canonical.display()
        ));
    }
    Ok(())
}

fn reject_mixed_routes(routes: &[SkillEntry]) -> Result<(), String> {
    let owned = routes
        .iter()
        .any(|route| route.ownership == EntryOwnership::PackCaptainLink);
    let native = routes
        .iter()
        .any(|route| route.ownership != EntryOwnership::PackCaptainLink);
    if owned && native {
        return Err(
            "mixed native and managed routes cannot be toggled safely; no files were changed"
                .to_owned(),
        );
    }
    Ok(())
}

fn verify_route(route: &SkillEntry, skill: &SkillRecord) -> Result<(), String> {
    if route.source_path != skill.source_path {
        return Err(format!(
            "skill entry source changed: {}",
            route.path.display()
        ));
    }
    match route.ownership {
        EntryOwnership::PackCaptainLink if !route.present => {
            match fs::symlink_metadata(&route.path) {
                Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
                Err(error) => Err(error.to_string()),
                Ok(_) => Err(format!(
                    "disabled managed entry reappeared externally: {}",
                    route.path.display()
                )),
            }
        }
        EntryOwnership::PackCaptainLink | EntryOwnership::UnmanagedSymlink => {
            let metadata = fs::symlink_metadata(&route.path).map_err(|error| {
                format!("skill entry changed: {}: {error}", route.path.display())
            })?;
            if !metadata.file_type().is_symlink() {
                return Err(format!(
                    "skill entry was replaced: {}",
                    route.path.display()
                ));
            }
            let canonical = fs::canonicalize(&route.path).map_err(|error| {
                format!(
                    "skill link is no longer resolvable: {}: {error}",
                    route.path.display()
                )
            })?;
            if canonical != skill.source_path {
                return Err(format!(
                    "skill link target changed: {}",
                    route.path.display()
                ));
            }
            let fingerprint = fingerprint_entry(&route.path).map_err(|error| error.to_string())?;
            if fingerprint != route.entry_fingerprint {
                return Err(format!(
                    "skill link changed after discovery: {}",
                    route.path.display()
                ));
            }
            Ok(())
        }
        EntryOwnership::DirectDirectory => {
            let canonical = fs::canonicalize(&route.path).map_err(|error| {
                format!("skill directory changed: {}: {error}", route.path.display())
            })?;
            if canonical != skill.source_path {
                return Err(format!(
                    "skill directory source changed: {}",
                    route.path.display()
                ));
            }
            let fingerprint = fingerprint_entry(&route.path).map_err(|error| error.to_string())?;
            if fingerprint != route.entry_fingerprint {
                return Err(format!(
                    "skill directory changed after discovery: {}",
                    route.path.display()
                ));
            }
            Ok(())
        }
    }
}

fn shared_entry_needed_elsewhere(
    target: &TargetId,
    skill_id: &SkillId,
    path: &Path,
    snapshot: &Snapshot,
) -> bool {
    snapshot.entries.iter().any(|(other_target, by_skill)| {
        if other_target == target {
            return false;
        }
        let shares_path = by_skill
            .get(skill_id)
            .is_some_and(|entries| entries.iter().any(|entry| entry.path == path));
        if !shares_path {
            return false;
        }
        let Some(state) = snapshot.targets.get(other_target) else {
            return true;
        };
        state.enabled_skills.contains(skill_id)
            || state.manual_open.contains(skill_id)
            || (!state.manual_closed.contains(skill_id)
                && state
                    .presets
                    .values()
                    .any(|preset| preset.enabled && preset.applied_members.contains(skill_id)))
    })
}

fn ensure_native_scope_safe(
    target: &TargetId,
    skill: &SkillRecord,
    snapshot: &Snapshot,
) -> Result<(), String> {
    match (&target.agent, &target.scope) {
        (AgentId::ClaudeCode, Scope::Global) => {
            for (other_skill_id, other_skill) in &snapshot.skills {
                if other_skill_id != &skill.id && other_skill.name == skill.name {
                    return Err(format!(
                        "Claude's native name-based switch for '{}' would affect another skill source",
                        skill.name
                    ));
                }
            }
            if has_other_target_source(target, skill, snapshot) {
                return Err(format!(
                    "Claude's user-level override for '{}' would affect another target",
                    skill.name
                ));
            }
            Ok(())
        }
        (AgentId::ClaudeCode, Scope::Project(_)) => Err(
            "target-local disable for an existing Claude project directory is not verified; no files were changed".to_owned(),
        ),
        (AgentId::Codex, Scope::Global) => {
            if has_other_target_source(target, skill, snapshot) {
                return Err(format!(
                    "Codex's user-level canonical-path switch for '{}' would affect another target",
                    skill.name
                ));
            }
            Ok(())
        }
        (AgentId::Codex, Scope::Project(_)) => Err(
            "target-local disable for an existing Codex project directory is not verified; no files were changed".to_owned(),
        ),
        (AgentId::Pi, Scope::Global) => Ok(()),
        (AgentId::Pi, Scope::Project(_)) => {
            if has_global_source(target, skill, snapshot) {
                return Err(format!(
                    "Pi project filtering this source could mask its global source: {}",
                    skill.source_path.display()
                ));
            }
            Ok(())
        }
    }
}

fn apply_native_changes(
    mutation: &TargetMutation<'_>,
    changes: &[(SkillRecord, Vec<SkillEntry>, Vec<SkillEntry>)],
    snapshot: &Snapshot,
    agent_home: &Path,
) -> Result<BTreeMap<PathBuf, Option<String>>, String> {
    let project = match &mutation.target.scope {
        Scope::Global => None,
        Scope::Project(project_id) => snapshot.projects.get(project_id),
    };
    let fallback_config = native_config_path(&mutation.target.agent, agent_home, project)
        .ok_or_else(|| "native settings path is unavailable".to_owned())?;
    let mut grouped = BTreeMap::<PathBuf, Vec<NativeConfigChange>>::new();
    let mut expected_by_path = BTreeMap::<PathBuf, Option<String>>::new();

    for (skill, before_routes, after_routes) in changes {
        let desired = after_routes.iter().any(|route| route.active);
        let native_routes: Vec<_> = after_routes
            .iter()
            .filter(|route| route.ownership != EntryOwnership::PackCaptainLink)
            .cloned()
            .collect();
        if native_routes.is_empty() {
            continue;
        }
        let config_path = native_routes
            .iter()
            .find_map(|route| route.native_config_path.clone())
            .unwrap_or_else(|| fallback_config.clone());
        let expected = before_routes
            .iter()
            .filter(|route| route.native_config_path.as_ref() == Some(&config_path))
            .map(|route| route.native_config_fingerprint.clone())
            .next()
            .unwrap_or(None);
        if let Some(previous) = expected_by_path.get(&config_path) {
            if previous != &expected {
                return Err(format!(
                    "inconsistent native config observations for {}",
                    config_path.display()
                ));
            }
        } else {
            expected_by_path.insert(config_path.clone(), expected);
        }
        grouped
            .entry(config_path)
            .or_default()
            .push(NativeConfigChange {
                skill: skill.clone(),
                routes: native_routes,
                prior_enabled: mutation.before.enabled_skills.contains(&skill.id),
                desired,
            });
    }

    let mut fingerprints = BTreeMap::new();
    for (config_path, changes) in grouped {
        let expected = expected_by_path.get(&config_path).cloned().flatten();
        let actual = fingerprint_file_if_present(&config_path).map_err(|error| {
            format!(
                "cannot inspect native config {}: {error}",
                config_path.display()
            )
        })?;
        let mut all_already_desired = true;
        for change in &changes {
            for route in &change.routes {
                if native_enabled(&mutation.target.agent, &change.skill, route, &config_path)?
                    != change.desired
                {
                    all_already_desired = false;
                }
            }
        }
        if actual != expected && all_already_desired {
            fingerprints.insert(config_path, actual);
            continue;
        }
        if actual != expected {
            return Err(format!(
                "native config changed after discovery: {}",
                config_path.display()
            ));
        }
        if all_already_desired {
            fingerprints.insert(config_path, actual);
            continue;
        }

        let updated_fingerprint = match &mutation.target.agent {
            AgentId::ClaudeCode => {
                write_claude_settings(&config_path, expected.as_deref(), &changes)?
            }
            AgentId::Codex => {
                let writes = codex_config_writes(&changes);
                if let Err(error) = run_codex_config_writes(agent_home, &writes) {
                    let rollback = codex_prior_writes(&changes);
                    if let Err(rollback_error) = run_codex_config_writes(agent_home, &rollback) {
                        return Err(format!(
                            "{error}; Codex config rollback failed: {rollback_error}"
                        ));
                    }
                    return Err(error);
                }
                fingerprint_file_if_present(&config_path)
                    .map_err(|error| format!("cannot verify Codex config write: {error}"))?
                    .ok_or_else(|| "Codex config write did not create a config file".to_owned())?
            }
            AgentId::Pi => write_pi_settings(&config_path, expected.as_deref(), &changes)?,
        };
        fingerprints.insert(config_path, Some(updated_fingerprint));
    }
    Ok(fingerprints)
}

#[derive(Clone)]
struct NativeConfigChange {
    skill: SkillRecord,
    routes: Vec<SkillEntry>,
    prior_enabled: bool,
    desired: bool,
}

fn native_enabled(
    agent: &AgentId,
    skill: &SkillRecord,
    route: &SkillEntry,
    config_path: &Path,
) -> Result<bool, String> {
    match agent {
        AgentId::Codex => codex_skill_enabled(config_path, &skill.source_path.join("SKILL.md")),
        AgentId::ClaudeCode | AgentId::Pi => native_config_disables(
            agent,
            skill,
            &route.path,
            &route.source_path,
            Some(config_path),
        )
        .map(|disabled| !disabled),
    }
}

fn write_claude_settings(
    path: &Path,
    expected: Option<&str>,
    changes: &[NativeConfigChange],
) -> Result<String, String> {
    let mut value = read_json_config(path, expected)?;
    let object = value
        .as_object_mut()
        .ok_or_else(|| "Claude settings root is not a JSON object".to_owned())?;
    let overrides = object
        .entry("skillOverrides")
        .or_insert_with(|| JsonValue::Object(serde_json::Map::new()));
    let overrides = overrides
        .as_object_mut()
        .ok_or_else(|| "Claude skillOverrides is not a JSON object".to_owned())?;
    for change in changes {
        overrides.insert(
            change.skill.name.clone(),
            JsonValue::String(if change.desired { "on" } else { "off" }.to_owned()),
        );
    }
    write_json_config(path, expected, &value)
}

fn write_pi_settings(
    path: &Path,
    expected: Option<&str>,
    changes: &[NativeConfigChange],
) -> Result<String, String> {
    let mut value = read_json_config(path, expected)?;
    let object = value
        .as_object_mut()
        .ok_or_else(|| "Pi settings root is not a JSON object".to_owned())?;
    let rules = object
        .entry("skills")
        .or_insert_with(|| JsonValue::Array(Vec::new()));
    let rules = rules
        .as_array_mut()
        .ok_or_else(|| "Pi skills setting is not an array".to_owned())?;
    for change in changes {
        for route in &change.routes {
            let path = route.path.to_string_lossy();
            let wanted = format!("{}{}", if change.desired { "+" } else { "-" }, path);
            let last_matching_rule = rules.iter().rev().find_map(|rule| {
                rule.as_str().and_then(|rule| {
                    let (prefix, path) = rule.split_at_checked(1)?;
                    (path == route.path.to_string_lossy()).then_some(prefix)
                })
            });
            let wanted_prefix = if change.desired { "+" } else { "-" };
            if last_matching_rule != Some(wanted_prefix) {
                rules.push(JsonValue::String(wanted));
            }
        }
    }
    write_json_config(path, expected, &value)
}

fn read_json_config(path: &Path, expected: Option<&str>) -> Result<JsonValue, String> {
    let fingerprint = fingerprint_file_if_present(path).map_err(|error| error.to_string())?;
    if fingerprint.as_deref() != expected {
        return Err(format!(
            "native config changed after preview: {}",
            path.display()
        ));
    }
    match fs::read(path) {
        Ok(contents) => serde_json::from_slice(&contents)
            .map_err(|error| format!("invalid settings JSON at {}: {error}", path.display())),
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            Ok(JsonValue::Object(serde_json::Map::new()))
        }
        Err(error) => Err(format!("cannot read settings {}: {error}", path.display())),
    }
}

fn write_json_config(
    path: &Path,
    expected: Option<&str>,
    value: &JsonValue,
) -> Result<String, String> {
    let bytes = serde_json::to_vec_pretty(value).map_err(|error| error.to_string())?;
    atomic_write_config(path, expected, &bytes)
}

fn atomic_write_config(
    path: &Path,
    expected: Option<&str>,
    bytes: &[u8],
) -> Result<String, String> {
    let current = fingerprint_file_if_present(path).map_err(|error| error.to_string())?;
    if current.as_deref() != expected {
        return Err(format!(
            "native config changed before write: {}",
            path.display()
        ));
    }
    let parent = path
        .parent()
        .ok_or_else(|| "native config path has no parent".to_owned())?;
    ensure_no_symlink_components(parent)?;
    create_dir_no_symlink(parent)?;
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| "native config filename is not valid UTF-8".to_owned())?;
    let temporary = parent.join(format!(".{file_name}.packcaptain-{}.tmp", Uuid::new_v4()));
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temporary)
        .map_err(|error| error.to_string())?;
    if let Ok(metadata) = fs::symlink_metadata(path) {
        if metadata.file_type().is_symlink() {
            let _ = fs::remove_file(&temporary);
            return Err(format!(
                "refusing to replace symlinked config {}",
                path.display()
            ));
        }
        fs::set_permissions(&temporary, metadata.permissions())
            .map_err(|error| error.to_string())?;
    } else {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&temporary, fs::Permissions::from_mode(0o600))
                .map_err(|error| error.to_string())?;
        }
    }
    if let Err(error) = file.write_all(bytes).and_then(|()| file.sync_all()) {
        let _ = fs::remove_file(&temporary);
        return Err(error.to_string());
    }
    drop(file);
    let current = fingerprint_file_if_present(path).map_err(|error| error.to_string())?;
    if current.as_deref() != expected {
        let _ = fs::remove_file(&temporary);
        return Err(format!(
            "native config changed during write: {}",
            path.display()
        ));
    }
    fs::rename(&temporary, path).map_err(|error| {
        let _ = fs::remove_file(&temporary);
        error.to_string()
    })?;
    Ok(digest_hex(bytes))
}

fn codex_config_writes(changes: &[NativeConfigChange]) -> Vec<(PathBuf, bool)> {
    let mut writes = BTreeMap::new();
    for change in changes {
        writes.insert(change.skill.source_path.join("SKILL.md"), change.desired);
    }
    writes.into_iter().collect()
}

fn codex_prior_writes(changes: &[NativeConfigChange]) -> Vec<(PathBuf, bool)> {
    let mut writes = BTreeMap::new();
    for change in changes {
        writes.insert(
            change.skill.source_path.join("SKILL.md"),
            change.prior_enabled,
        );
    }
    writes.into_iter().collect()
}

fn codex_skill_enabled(config_path: &Path, skill_md: &Path) -> Result<bool, String> {
    let contents = match fs::read_to_string(config_path) {
        Ok(contents) => contents,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(true),
        Err(error) => return Err(error.to_string()),
    };
    let value: TomlValue = contents
        .parse()
        .map_err(|error| format!("invalid Codex config {}: {error}", config_path.display()))?;
    let Some(entries) = value.get("skills").and_then(|skills| skills.get("config")) else {
        return Ok(true);
    };
    let entries = entries
        .as_array()
        .ok_or_else(|| "Codex skills.config is not an array".to_owned())?;
    let mut enabled = true;
    for entry in entries {
        let Some(path) = entry.get("path").and_then(TomlValue::as_str) else {
            continue;
        };
        if Path::new(path) == skill_md {
            enabled = entry
                .get("enabled")
                .and_then(TomlValue::as_bool)
                .unwrap_or(true);
        }
    }
    Ok(enabled)
}

fn run_codex_config_writes(agent_home: &Path, writes: &[(PathBuf, bool)]) -> Result<(), String> {
    if writes.is_empty() {
        return Ok(());
    }
    let mut command = ProcessCommand::new("codex");
    command
        .arg("app-server")
        .current_dir(agent_home)
        .env_clear()
        .env("HOME", agent_home)
        .env("CODEX_HOME", agent_home.join(".codex"))
        .env("TERM", "dumb");
    if let Some(path) = std::env::var_os("PATH") {
        command.env("PATH", path);
    }
    let mut child = command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| format!("cannot start installed Codex app-server: {error}"))?;
    let stderr_pipe = child
        .stderr
        .take()
        .ok_or_else(|| "Codex app-server stderr is unavailable".to_owned())?;
    let stderr_reader = std::thread::spawn(move || {
        let mut stderr_text = String::new();
        let mut reader = BufReader::new(stderr_pipe);
        let _ = reader.read_to_string(&mut stderr_text);
        stderr_text
    });
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| "Codex app-server stdout is unavailable".to_owned())?;
    let mut stdout_reader = BufReader::new(stdout);
    let mut input = child
        .stdin
        .take()
        .ok_or_else(|| "Codex app-server stdin is unavailable".to_owned())?;

    let rpc_result = (|| {
        write_rpc_request(
            &mut input,
            &serde_json::json!({
                "id": 1,
                "method": "initialize",
                "params": {"clientInfo": {"name": "packcaptain", "version": env!("CARGO_PKG_VERSION")}}
            }),
        )?;
        let initialized = read_rpc_response(&mut stdout_reader, 1)?;
        ensure_rpc_success(&initialized, "initialize")?;
        writeln!(input, "{}", serde_json::json!({"method": "initialized"}))
            .map_err(|error| error.to_string())?;
        input.flush().map_err(|error| error.to_string())?;

        for (index, (path, enabled)) in writes.iter().enumerate() {
            let id = index as u64 + 2;
            write_rpc_request(
                &mut input,
                &serde_json::json!({
                    "id": id,
                    "method": "skills/config/write",
                    "params": {"path": path, "enabled": enabled}
                }),
            )?;
            let response = read_rpc_response(&mut stdout_reader, id)?;
            ensure_rpc_success(&response, "skills/config/write")?;
            let effective = response
                .get("result")
                .and_then(|result| result.get("effectiveEnabled"))
                .and_then(JsonValue::as_bool)
                .ok_or_else(|| "Codex config/write response omitted effectiveEnabled".to_owned())?;
            if effective != *enabled {
                return Err(format!(
                    "Codex config write returned enabled={effective}, expected {enabled}"
                ));
            }
        }
        Ok(())
    })();

    drop(input);
    let mut remaining_stdout = String::new();
    let drain_result = stdout_reader
        .read_to_string(&mut remaining_stdout)
        .map_err(|error| error.to_string());
    drop(stdout_reader);
    let status = child
        .wait()
        .map_err(|error| format!("failed waiting for Codex app-server: {error}"))?;
    let stderr = stderr_reader
        .join()
        .unwrap_or_else(|_| "failed reading Codex app-server stderr".to_owned());
    rpc_result?;
    drain_result?;
    if !status.success() {
        return Err(format!("Codex app-server exited with {status}: {stderr}"));
    }
    Ok(())
}

fn write_rpc_request(input: &mut impl Write, value: &JsonValue) -> Result<(), String> {
    writeln!(input, "{value}").map_err(|error| error.to_string())?;
    input.flush().map_err(|error| error.to_string())
}

fn read_rpc_response(reader: &mut impl BufRead, id: u64) -> Result<JsonValue, String> {
    loop {
        let mut line = String::new();
        let read = reader
            .read_line(&mut line)
            .map_err(|error| format!("failed reading Codex RPC response: {error}"))?;
        if read == 0 {
            return Err(format!("Codex app-server closed before response id {id}"));
        }
        let response: JsonValue = serde_json::from_str(line.trim_end())
            .map_err(|error| format!("invalid Codex app-server response: {error}"))?;
        if response.get("id").and_then(JsonValue::as_u64) == Some(id) {
            return Ok(response);
        }
    }
}

fn ensure_rpc_success(response: &JsonValue, method: &str) -> Result<(), String> {
    if let Some(error) = response.get("error") {
        return Err(format!("Codex {method} RPC failed: {error}"));
    }
    if response.get("result").is_none() {
        return Err(format!("Codex {method} RPC omitted its result"));
    }
    Ok(())
}

fn has_other_target_source(target: &TargetId, skill: &SkillRecord, snapshot: &Snapshot) -> bool {
    snapshot.entries.iter().any(|(other_target, entries)| {
        other_target != target
            && entries
                .values()
                .flatten()
                .any(|entry| entry.active && entry.source_path == skill.source_path)
    })
}

fn has_global_source(target: &TargetId, skill: &SkillRecord, snapshot: &Snapshot) -> bool {
    let global = TargetId {
        agent: target.agent.clone(),
        scope: Scope::Global,
    };
    snapshot
        .entries
        .get(&global)
        .into_iter()
        .flat_map(|entries| entries.values())
        .flatten()
        .any(|entry| entry.source_path == skill.source_path && entry.active)
}

fn receipt_directory_name(key: &ReceiptKey) -> String {
    match key.direction {
        RecoveryDirection::Forward => {
            format!(".packcaptain-{}-{}", key.operation_id, key.route_ordinal)
        }
        RecoveryDirection::Rollback => format!(
            ".packcaptain-{}-rollback-{}",
            key.operation_id, key.route_ordinal
        ),
    }
}

fn receipt_anchor_path(
    context: &ForwardReceiptContext<'_>,
    skill: &SkillId,
    path: &Path,
) -> Result<PathBuf, String> {
    if Uuid::parse_str(context.operation_id).is_err() {
        return Err("invalid receipt context".to_owned());
    }
    let key = context.key(skill, path)?;
    let parent = path
        .parent()
        .ok_or_else(|| "managed link has no parent".to_owned())?;
    Ok(parent
        .parent()
        .ok_or_else(|| "managed link root has no parent".to_owned())?
        .join(receipt_directory_name(&key))
        .join("anchor"))
}

fn load_create_receipt(
    context: &ForwardReceiptContext<'_>,
    target: &TargetId,
    skill: &SkillRecord,
    path: &Path,
) -> Result<Option<MutationReceipt>, String> {
    let key = context.key(&skill.id, path)?;
    let anchor = receipt_anchor_path(context, &skill.id, path)?;
    let receipt = context
        .reader
        .load_receipt(&key)
        .map_err(|error| error.to_string())?;
    if let Some(receipt) = &receipt {
        let identity = &receipt.identity;
        if receipt.key != key
            || identity.target != *target
            || identity.skill != skill.id
            || identity.path != path
            || identity.source != skill.source_path
            || identity.anchor != anchor
            || identity.action != ReceiptAction::Create
            || identity.no_follow.readlink != skill.source_path
        {
            return Err("receipt does not match route identity".to_owned());
        }
    }
    Ok(receipt)
}

#[cfg(unix)]
fn symlink_identity(path: &Path) -> Result<SymlinkIdentity, String> {
    use std::os::unix::fs::MetadataExt;
    let metadata = fs::symlink_metadata(path).map_err(|error| error.to_string())?;
    if !metadata.file_type().is_symlink() {
        return Err("inode pin is not a symlink".to_owned());
    }
    Ok(SymlinkIdentity {
        device: metadata.dev(),
        inode: metadata.ino(),
        readlink: fs::read_link(path).map_err(|error| error.to_string())?,
    })
}

#[cfg(not(unix))]
fn symlink_identity(_path: &Path) -> Result<SymlinkIdentity, String> {
    Err("staged symlink inode pins are unsupported on this platform".to_owned())
}

fn verify_anchor_pin(receipt: &MutationReceipt) -> Result<(), String> {
    ensure_no_symlink_components(
        receipt
            .identity
            .anchor
            .parent()
            .ok_or_else(|| "anchor has no parent".to_owned())?,
    )?;
    if symlink_identity(&receipt.identity.anchor)? != receipt.identity.no_follow {
        return Err("staged anchor identity changed".to_owned());
    }
    Ok(())
}

fn verify_public_pin(receipt: &MutationReceipt) -> Result<(), String> {
    verify_anchor_pin(receipt)?;
    ensure_no_symlink_components(
        receipt
            .identity
            .path
            .parent()
            .ok_or_else(|| "managed link has no parent".to_owned())?,
    )?;
    if symlink_identity(&receipt.identity.path)? != receipt.identity.no_follow {
        return Err("public link differs from staged inode pin".to_owned());
    }
    Ok(())
}

fn create_pinned_link(
    context: &ForwardReceiptContext<'_>,
    target: &TargetId,
    skill: &SkillRecord,
    path: &Path,
    expected: &str,
) -> Result<String, String> {
    let mut receipt = if let Some(receipt) = load_create_receipt(context, target, skill, path)? {
        verify_anchor_pin(&receipt)?;
        receipt
    } else {
        if expected != "pending" && expected != "missing" {
            return set_managed_link(path, &skill.source_path, true, expected)?
                .ok_or_else(|| "managed link is missing".to_owned());
        }
        match fs::symlink_metadata(path) {
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.to_string()),
            Ok(_) => {
                return Err(format!(
                    "managed entry appeared externally before creation: {}",
                    path.display()
                ));
            }
        }
        let parent = path
            .parent()
            .ok_or_else(|| "managed link has no parent".to_owned())?;
        create_dir_no_symlink(parent)?;
        let anchor = receipt_anchor_path(context, &skill.id, path)?;
        let directory = anchor.parent().unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::DirBuilderExt;
            fs::DirBuilder::new()
                .mode(0o700)
                .create(directory)
                .map_err(|error| error.to_string())?;
        }
        #[cfg(not(unix))]
        {
            return Err("staged symlink inode pins are unsupported on this platform".to_owned());
        }
        create_directory_symlink(&skill.source_path, &anchor).map_err(|error| error.to_string())?;
        let identity = symlink_identity(&anchor)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            if fs::symlink_metadata(parent)
                .map_err(|error| error.to_string())?
                .dev()
                != identity.device
            {
                return Err(
                    "staging anchor and public parent are on different filesystems".to_owned(),
                );
            }
        }
        let probe = directory.join("probe");
        fs::hard_link(&anchor, &probe)
            .map_err(|error| format!("symlink hard-link pin unsupported: {error}"))?;
        if symlink_identity(&probe)? != identity {
            return Err("hard-link did not preserve symlink inode".to_owned());
        }
        fs::remove_file(&probe).map_err(|error| error.to_string())?;
        File::open(directory)
            .and_then(|file| file.sync_all())
            .map_err(|error| error.to_string())?;
        File::open(directory.parent().unwrap())
            .and_then(|file| file.sync_all())
            .map_err(|error| error.to_string())?;
        let receipt = MutationReceipt {
            key: context.key(&skill.id, path)?,
            identity: ReceiptIdentity {
                target: target.clone(),
                skill: skill.id.clone(),
                path: path.to_owned(),
                source: skill.source_path.clone(),
                anchor,
                no_follow: identity,
                action: ReceiptAction::Create,
            },
            stage: ReceiptStage::Prepared,
        };
        context
            .writer
            .save_receipt(&receipt)
            .map_err(|error| error.to_string())?;
        receipt
    };
    match fs::symlink_metadata(path) {
        Ok(_) => verify_public_pin(&receipt)?,
        Err(error)
            if error.kind() == io::ErrorKind::NotFound
                && receipt.stage == ReceiptStage::Prepared =>
        {
            verify_anchor_pin(&receipt)?;
            fs::hard_link(&receipt.identity.anchor, path).map_err(|error| error.to_string())?;
            verify_public_pin(&receipt)?;
            File::open(path.parent().unwrap())
                .and_then(|file| file.sync_all())
                .map_err(|error| error.to_string())?;
        }
        Err(error) => return Err(error.to_string()),
    }
    receipt.stage = ReceiptStage::VerifiedApplied;
    context
        .writer
        .save_receipt(&receipt)
        .map_err(|error| error.to_string())?;
    fingerprint_entry(path).map_err(|error| error.to_string())
}

pub(crate) fn cleanup_receipt_anchor(receipt: &MutationReceipt) {
    if receipt.identity.action != ReceiptAction::Create
        || Uuid::parse_str(&receipt.key.operation_id).is_err()
    {
        return;
    }
    let Some(parent) = receipt.identity.path.parent() else {
        return;
    };
    let Some(staging_parent) = parent.parent() else {
        return;
    };
    let directory = staging_parent.join(receipt_directory_name(&receipt.key));
    if receipt.identity.anchor != directory.join("anchor")
        || receipt.identity.no_follow.readlink != receipt.identity.source
        || verify_anchor_pin(receipt).is_err()
    {
        return;
    }
    if fs::remove_file(&receipt.identity.anchor).is_ok() {
        let _ = fs::remove_dir(&directory);
    }
}

fn apply_link_changes(
    changes: &[(SkillRecord, Vec<SkillEntry>, Vec<SkillEntry>)],
    mutation: &TargetMutation<'_>,
) -> Result<(), String> {
    let mut completed = Vec::<(PathBuf, PathBuf, bool, String)>::new();
    for (skill, before, after) in changes {
        let mut routes: BTreeMap<PathBuf, (Option<&SkillEntry>, Option<&SkillEntry>)> =
            BTreeMap::new();
        for route in before {
            routes.entry(route.path.clone()).or_default().0 = Some(route);
        }
        for route in after {
            routes.entry(route.path.clone()).or_default().1 = Some(route);
        }
        for (path, (old, new)) in routes {
            let was_active = old.is_some_and(|entry| entry.present);
            let is_active = new.is_some_and(|entry| entry.present);
            if was_active == is_active {
                continue;
            }
            let expected = old
                .map(|entry| entry.entry_fingerprint.as_str())
                .or_else(|| new.map(|entry| entry.entry_fingerprint.as_str()))
                .unwrap_or("pending");
            let result = if is_active && let Some(context) = mutation.receipt_context {
                create_pinned_link(context, mutation.target, skill, &path, expected).map(Some)
            } else {
                set_managed_link(&path, &skill.source_path, is_active, expected)
            };
            match result {
                Ok(current_fingerprint) => completed.push((
                    path,
                    skill.source_path.clone(),
                    was_active,
                    current_fingerprint.unwrap_or_else(|| "missing".to_owned()),
                )),
                Err(error) => {
                    if mutation.receipt_context.is_some() {
                        return Err(error);
                    }
                    let mut rollback_errors = Vec::new();
                    for (path, source, was_enabled, current_fingerprint) in completed.iter().rev() {
                        if let Err(rollback_error) =
                            set_managed_link(path, source, *was_enabled, current_fingerprint)
                        {
                            rollback_errors.push(rollback_error);
                        }
                    }
                    if rollback_errors.is_empty() {
                        return Err(error);
                    }
                    return Err(format!(
                        "{error}; rollback failed: {}",
                        rollback_errors.join("; ")
                    ));
                }
            }
        }
    }
    Ok(())
}

fn set_managed_link(
    path: &Path,
    source: &Path,
    enabled: bool,
    expected_fingerprint: &str,
) -> Result<Option<String>, String> {
    if enabled {
        if fs::symlink_metadata(path).is_ok() {
            if expected_fingerprint == "pending" || expected_fingerprint == "missing" {
                return Err(format!(
                    "managed entry appeared externally before creation: {}",
                    path.display()
                ));
            }
            let metadata = fs::symlink_metadata(path).map_err(|error| error.to_string())?;
            if !metadata.file_type().is_symlink() {
                return Err(format!(
                    "refusing to replace unowned entry {}",
                    path.display()
                ));
            }
            let target = fs::canonicalize(path).map_err(|error| error.to_string())?;
            if target != source {
                return Err(format!(
                    "refusing to replace external symlink {}",
                    path.display()
                ));
            }
            let actual = fingerprint_entry(path).map_err(|error| error.to_string())?;
            if actual != expected_fingerprint {
                return Err(format!("symlink changed after preview: {}", path.display()));
            }
            return Ok(Some(actual));
        }
        if expected_fingerprint != "pending" && expected_fingerprint != "missing" {
            return Err(format!(
                "managed link disappeared after preview: {}",
                path.display()
            ));
        }
        let parent = path
            .parent()
            .ok_or_else(|| "managed link has no parent".to_owned())?;
        ensure_no_symlink_components(parent)?;
        create_dir_no_symlink(parent)?;
        create_directory_symlink(source, path).map_err(|error| error.to_string())?;
        Ok(Some(
            fingerprint_entry(path).map_err(|error| error.to_string())?,
        ))
    } else {
        match fs::symlink_metadata(path) {
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(error.to_string()),
            Ok(metadata) => {
                if !metadata.file_type().is_symlink() {
                    return Err(format!(
                        "refusing to remove non-link entry {}",
                        path.display()
                    ));
                }
                let target = fs::canonicalize(path).map_err(|error| error.to_string())?;
                if target != source {
                    return Err(format!(
                        "refusing to remove external symlink {}",
                        path.display()
                    ));
                }
                let actual = fingerprint_entry(path).map_err(|error| error.to_string())?;
                if actual != expected_fingerprint {
                    return Err(format!("symlink changed after preview: {}", path.display()));
                }
                remove_directory_symlink(path).map_err(|error| error.to_string())?;
                Ok(None)
            }
        }
    }
}

fn ensure_no_symlink_components(path: &Path) -> Result<(), String> {
    let mut current = PathBuf::new();
    for component in path.components() {
        current.push(component);
        match fs::symlink_metadata(&current) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                return Err(format!(
                    "refusing to write through symlinked directory {}",
                    current.display()
                ));
            }
            Ok(metadata) if !metadata.is_dir() => {
                return Err(format!("expected a directory at {}", current.display()));
            }
            Ok(_) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.to_string()),
        }
    }
    Ok(())
}

fn create_dir_no_symlink(path: &Path) -> Result<(), String> {
    ensure_no_symlink_components(path)?;
    let mut current = PathBuf::new();
    for component in path.components() {
        current.push(component);
        if fs::symlink_metadata(&current).is_ok() {
            continue;
        }
        match fs::create_dir(&current) {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
                let metadata = fs::symlink_metadata(&current).map_err(|error| error.to_string())?;
                if metadata.file_type().is_symlink() || !metadata.is_dir() {
                    return Err(format!(
                        "refusing to use unsafe directory {}",
                        current.display()
                    ));
                }
            }
            Err(error) => return Err(error.to_string()),
        }
    }
    Ok(())
}

#[cfg(unix)]
fn create_directory_symlink(source: &Path, destination: &Path) -> io::Result<()> {
    std::os::unix::fs::symlink(source, destination)
}

#[cfg(windows)]
fn create_directory_symlink(source: &Path, destination: &Path) -> io::Result<()> {
    std::os::windows::fs::symlink_dir(source, destination)
}

#[cfg(unix)]
fn remove_directory_symlink(path: &Path) -> io::Result<()> {
    fs::remove_file(path)
}

#[cfg(windows)]
fn remove_directory_symlink(path: &Path) -> io::Result<()> {
    fs::remove_dir(path)
}

fn preserve_disabled_owned_entries(
    snapshot: &Snapshot,
    target: &TargetId,
    report: &mut DiscoveryReport,
) -> Result<(), DiscoveryError> {
    if let Some(existing_by_skill) = snapshot.entries.get(target) {
        let entries_by_skill = report.entries.entry(target.clone()).or_default();
        for (skill_id, existing_entries) in existing_by_skill {
            let current_entries = entries_by_skill.entry(skill_id.clone()).or_default();
            for existing in existing_entries {
                if current_entries
                    .iter()
                    .any(|entry| entry.path == existing.path)
                {
                    continue;
                }
                let mut preserved = existing.clone();
                if preserved.ownership == EntryOwnership::PackCaptainLink
                    && preserved.present
                    && !preserved.active
                    && fs::symlink_metadata(&preserved.path)
                        .is_err_and(|error| error.kind() == io::ErrorKind::NotFound)
                    && snapshot.entries.iter().any(|(other, by_skill)| {
                        other != target
                            && by_skill.get(skill_id).is_some_and(|entries| {
                                entries.iter().any(|peer| {
                                    peer.path == preserved.path
                                        && peer.source_path == preserved.source_path
                                        && peer.ownership == EntryOwnership::PackCaptainLink
                                        && !peer.present
                                        && !peer.active
                                        && peer.entry_fingerprint == "missing"
                                })
                            })
                    })
                {
                    preserved.present = false;
                    preserved.entry_fingerprint = "missing".to_owned();
                }
                let source_definition_exists =
                    scan_definition_exists(&preserved.source_path.join("SKILL.md"))?;
                let entry_exists =
                    scan_optional(fs::symlink_metadata(&preserved.path), &preserved.path)?
                        .is_some();
                if !source_definition_exists {
                    if let Some(skill) = snapshot.skills.get(skill_id) {
                        let mut skill = skill.clone();
                        skill.availability = SkillAvailability::Missing;
                        report.skills.insert(skill_id.clone(), skill);
                    }
                    preserved.active = false;
                    report.diagnostics.push(format!(
                        "skill source definition is missing: {}",
                        preserved.source_path.display()
                    ));
                } else if !entry_exists && preserved.active {
                    report.diagnostics.push(format!(
                        "previous skill entry is missing: {}",
                        preserved.path.display()
                    ));
                }
                if preserved.ownership == EntryOwnership::PackCaptainLink && preserved.active {
                    report
                        .observed_enabled
                        .entry(target.clone())
                        .or_default()
                        .insert(skill_id.clone());
                }
                current_entries.push(preserved);
            }
        }
    }
    Ok(())
}

fn skill_roots(
    agent: &AgentId,
    agent_home: &Path,
    project: Option<&ProjectRecord>,
) -> Vec<PathBuf> {
    let base = project
        .map(|project| project.root.as_path())
        .unwrap_or(agent_home);
    match (agent, project.is_some()) {
        (AgentId::ClaudeCode, false) => vec![base.join(".claude/skills")],
        (AgentId::ClaudeCode, true) => vec![base.join(".claude/skills")],
        (AgentId::Codex, false) => vec![base.join(".codex/skills"), base.join(".agents/skills")],
        (AgentId::Codex, true) => vec![base.join(".codex/skills"), base.join(".agents/skills")],
        (AgentId::Pi, false) => vec![base.join(".pi/agent/skills"), base.join(".agents/skills")],
        (AgentId::Pi, true) => vec![base.join(".pi/skills"), base.join(".agents/skills")],
    }
}

fn native_config_path(
    agent: &AgentId,
    agent_home: &Path,
    project: Option<&ProjectRecord>,
) -> Option<PathBuf> {
    let base = project
        .map(|project| project.root.as_path())
        .unwrap_or(agent_home);
    match (agent, project.is_some()) {
        (AgentId::ClaudeCode, false) => Some(base.join(".claude/settings.json")),
        (AgentId::ClaudeCode, true) => Some(base.join(".claude/settings.local.json")),
        (AgentId::Codex, false) => Some(base.join(".codex/config.toml")),
        (AgentId::Codex, true) => Some(base.join(".codex/config.toml")),
        (AgentId::Pi, false) => Some(base.join(".pi/agent/settings.json")),
        (AgentId::Pi, true) => Some(base.join(".pi/settings.json")),
    }
}

fn native_config_disables(
    agent: &AgentId,
    skill: &SkillRecord,
    entry_path: &Path,
    source_path: &Path,
    config_path: Option<&Path>,
) -> Result<bool, String> {
    NativeConfig::read(agent, config_path)?.disables(skill, entry_path, source_path)
}

enum NativeConfigValue {
    Missing,
    Claude(JsonValue),
    Codex(TomlValue),
    Pi(JsonValue),
}

struct NativeConfig {
    value: NativeConfigValue,
    fingerprint: Option<String>,
}

impl NativeConfig {
    fn read(agent: &AgentId, path: Option<&Path>) -> Result<Self, String> {
        let missing = || Self {
            value: NativeConfigValue::Missing,
            fingerprint: None,
        };
        let Some(path) = path else {
            return Ok(missing());
        };
        match fs::symlink_metadata(path) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                return Err(format!(
                    "native config path is a symbolic link: {}",
                    path.display()
                ));
            }
            Ok(_) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(missing()),
            Err(error) => {
                return Err(format!(
                    "cannot inspect native config {}: {error}",
                    path.display()
                ));
            }
        }
        let bytes = fs::read(path)
            .map_err(|error| format!("cannot read native config {}: {error}", path.display()))?;
        let contents = std::str::from_utf8(&bytes)
            .map_err(|error| format!("invalid native config {}: {error}", path.display()))?;
        let invalid = |error: String| format!("invalid native config {}: {error}", path.display());
        let value = match agent {
            AgentId::ClaudeCode | AgentId::Pi => {
                let value: JsonValue =
                    serde_json::from_str(contents).map_err(|error| invalid(error.to_string()))?;
                let object = value
                    .as_object()
                    .ok_or_else(|| invalid("settings root is not a JSON object".to_owned()))?;
                if *agent == AgentId::ClaudeCode {
                    if let Some(overrides) = object.get("skillOverrides") {
                        let overrides = overrides.as_object().ok_or_else(|| {
                            invalid("Claude skillOverrides is not an object".to_owned())
                        })?;
                        if overrides
                            .values()
                            .any(|value| !matches!(value.as_str(), Some("on" | "off")))
                        {
                            return Err(invalid("unknown Claude skill override".to_owned()));
                        }
                    }
                    NativeConfigValue::Claude(value)
                } else {
                    if let Some(rules) = object.get("skills") {
                        let rules = rules.as_array().ok_or_else(|| {
                            invalid("Pi skills setting is not an array".to_owned())
                        })?;
                        if rules.iter().any(|rule| rule.as_str().is_none()) {
                            return Err(invalid("Pi skill filter is not a string".to_owned()));
                        }
                    }
                    NativeConfigValue::Pi(value)
                }
            }
            AgentId::Codex => {
                let value: TomlValue = contents
                    .parse()
                    .map_err(|error: toml::de::Error| invalid(error.to_string()))?;
                if let Some(skills) = value.get("skills") {
                    let skills = skills
                        .as_table()
                        .ok_or_else(|| invalid("Codex skills is not a table".to_owned()))?;
                    if let Some(config) = skills.get("config") {
                        let entries = config.as_array().ok_or_else(|| {
                            invalid("Codex skills.config is not an array".to_owned())
                        })?;
                        for entry in entries {
                            if entry.as_table().is_none()
                                || entry
                                    .get("path")
                                    .is_some_and(|path| path.as_str().is_none())
                                || entry
                                    .get("enabled")
                                    .is_some_and(|enabled| enabled.as_bool().is_none())
                            {
                                return Err(invalid("invalid Codex skill config entry".to_owned()));
                            }
                        }
                    }
                }
                NativeConfigValue::Codex(value)
            }
        };
        Ok(Self {
            value,
            fingerprint: Some(digest_hex(&bytes)),
        })
    }

    fn disables(
        &self,
        skill: &SkillRecord,
        entry_path: &Path,
        source_path: &Path,
    ) -> Result<bool, String> {
        match &self.value {
            NativeConfigValue::Missing => Ok(false),
            NativeConfigValue::Claude(value) => {
                let Some(overrides) = value.get("skillOverrides") else {
                    return Ok(false);
                };
                let Some(override_value) = overrides.get(&skill.name) else {
                    return Ok(false);
                };
                match override_value.as_str() {
                    Some("off") => Ok(true),
                    Some("on") => Ok(false),
                    Some(value) => Err(format!("unknown skill override value '{value}'")),
                    None => Err("skill override is not a string".to_owned()),
                }
            }
            NativeConfigValue::Codex(value) => {
                let Some(config) = value.get("skills").and_then(|skills| skills.get("config"))
                else {
                    return Ok(false);
                };
                let config = config
                    .as_array()
                    .ok_or_else(|| "Codex skills.config is not an array".to_owned())?;
                Ok(config.iter().any(|entry| {
                    entry.get("enabled").and_then(TomlValue::as_bool) == Some(false)
                        && entry
                            .get("path")
                            .and_then(TomlValue::as_str)
                            .is_some_and(|path| {
                                Path::new(path) == entry_path.join("SKILL.md")
                                    || Path::new(path) == source_path.join("SKILL.md")
                            })
                }))
            }
            NativeConfigValue::Pi(value) => {
                let Some(rules) = value.get("skills") else {
                    return Ok(false);
                };
                let rules = rules
                    .as_array()
                    .ok_or_else(|| "Pi skills setting is not an array".to_owned())?;
                let mut enabled = true;
                for rule in rules {
                    let rule = rule
                        .as_str()
                        .ok_or_else(|| "Pi skill filter is not a string".to_owned())?;
                    let Some((prefix, pattern)) = rule.split_at_checked(1) else {
                        continue;
                    };
                    if pattern.contains('*') || pattern.contains('?') {
                        return Err(format!("unsupported Pi skill filter pattern '{rule}'"));
                    }
                    let matches =
                        Path::new(pattern) == entry_path || Path::new(pattern) == source_path;
                    if matches {
                        match prefix {
                            "-" => enabled = false,
                            "+" => enabled = true,
                            _ => return Err(format!("unsupported Pi skill filter '{rule}'")),
                        }
                    }
                }
                Ok(!enabled)
            }
        }
    }
}

fn parse_skill_name(contents: &str) -> Option<String> {
    let mut lines = contents.lines();
    if lines.next()?.trim() != "---" {
        return None;
    }
    for line in lines {
        if line.trim() == "---" {
            break;
        }
        if let Some(value) = line.trim().strip_prefix("name:") {
            let name = value.trim().trim_matches(['\'', '"']);
            if !name.is_empty() {
                return Some(name.to_owned());
            }
        }
    }
    None
}

fn fingerprint_file_if_present(path: &Path) -> io::Result<Option<String>> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "native config path is a symbolic link",
        )),
        Ok(_) => fs::read(path).map(|bytes| Some(digest_hex(&bytes))),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error),
    }
}

fn fingerprint_entry(path: &Path) -> io::Result<String> {
    let metadata = fs::symlink_metadata(path)?;
    let mut digest = Sha256::new();
    digest.update(path.as_os_str().as_encoded_bytes());
    if metadata.file_type().is_symlink() {
        digest.update(b"symlink");
        digest.update(fs::read_link(path)?.as_os_str().as_encoded_bytes());
    } else if metadata.is_dir() {
        digest.update(b"directory");
        digest.update(fingerprint_directory(path)?.as_bytes());
    } else {
        digest.update(b"file");
        digest.update(fingerprint_file(path)?.as_bytes());
    }
    digest.update(metadata.len().to_be_bytes());
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        digest.update(metadata.dev().to_be_bytes());
        digest.update(metadata.ino().to_be_bytes());
    }
    Ok(format!("{:x}", digest.finalize()))
}

fn fingerprint_file(path: &Path) -> io::Result<String> {
    let mut file = File::open(path)?;
    let mut digest = Sha256::new();
    let mut buffer = [0; 16 * 1024];
    loop {
        let count = file.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        digest.update(&buffer[..count]);
    }
    Ok(format!("{:x}", digest.finalize()))
}

fn fingerprint_directory(path: &Path) -> io::Result<String> {
    let mut digest = Sha256::new();
    for item in WalkDir::new(path)
        .follow_links(true)
        .sort_by_file_name()
        .into_iter()
    {
        let item = item.map_err(|error| {
            let kind = error
                .io_error()
                .map_or(io::ErrorKind::Other, io::Error::kind);
            io::Error::new(kind, error.to_string())
        })?;
        let relative = item.path().strip_prefix(path).unwrap_or(item.path());
        digest.update(relative.as_os_str().as_encoded_bytes());
        let metadata = fs::symlink_metadata(item.path())?;
        if metadata.is_dir() {
            digest.update(b"directory");
        } else if metadata.is_file() {
            digest.update(b"file");
            digest.update(fingerprint_file(item.path())?.as_bytes());
        } else if metadata.file_type().is_symlink() {
            digest.update(b"symlink");
            digest.update(fs::read_link(item.path())?.as_os_str().as_encoded_bytes());
        }
    }
    Ok(format!("{:x}", digest.finalize()))
}

fn digest_hex(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
