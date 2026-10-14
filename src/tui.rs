#[path = "tui_theme.rs"]
mod tui_theme;
use tui_theme::{MOCHA, Theme};

use std::collections::BTreeSet;
use std::io;
use std::path::PathBuf;

use crossterm::{
    event::{self, Event, KeyCode, KeyEventKind},
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use packcaptain::{
    adapters::{DiscoveryReport, NativeAdapters, ProjectCandidate},
    domain::{
        AgentId, ClosedMembersPolicy, Command, Plan, PresetId, ProjectId, ProjectRecord, Scope,
        SkillId, Snapshot, TargetId, TargetState,
    },
    engine::{Engine, TargetOutcomeStatus},
};
use ratatui::{
    Terminal,
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, ListState, Paragraph, Wrap},
};

use packcaptain::engine::EngineError;

#[derive(Clone, Copy, PartialEq, Debug)]
enum Screen {
    ScopeRoot,
    PresetList,
    PresetActions,
    SkillPick,
    SkillActions,
    CreateName,
    CreateMembers,
    EditMembers,
    Targets,
    Confirm,
    ProjectRoot,
    ProjectCandidates,
    EditPolicy,
}

/// Typed draft of an in-progress operation. The final `Command` is built
/// once targets are selected, never stored half-filled.
#[derive(Clone, Debug)]
enum Draft {
    TogglePreset {
        preset_id: PresetId,
        enabled: bool,
    },
    ToggleSkill {
        skill_id: SkillId,
        enabled: bool,
    },
    CreatePreset {
        preset_id: PresetId,
        name: String,
        members: BTreeSet<SkillId>,
    },
    EditMembers {
        preset_id: PresetId,
        expected_revision: u64,
        members: BTreeSet<SkillId>,
    },
}

impl Draft {
    fn source_screen(&self) -> Screen {
        match self {
            Self::TogglePreset { .. } => Screen::PresetActions,
            Self::ToggleSkill { .. } => Screen::SkillActions,
            Self::CreatePreset { .. } => Screen::CreateMembers,
            Self::EditMembers { .. } => Screen::EditMembers,
        }
    }
}

fn agent_name(agent: &AgentId) -> &'static str {
    match agent {
        AgentId::ClaudeCode => "Claude Code",
        AgentId::Codex => "Codex",
        AgentId::Pi => "Pi",
    }
}

fn scope_label(scope: &Scope, snapshot: &Snapshot) -> String {
    match scope {
        Scope::Global => "全局".to_owned(),
        Scope::Project(id) => match snapshot.projects.get(id) {
            Some(project) => project.name.clone(),
            None => format!("项目 {}", short_id(id.as_str())),
        },
    }
}

fn short_id(id: &str) -> String {
    id.chars().take(8).collect()
}

/// Human label for a target: agent name plus project name, UUID only as a
/// short secondary diagnostic when the project is unknown.
fn target_label(target: &TargetId, snapshot: &Snapshot) -> String {
    match &target.scope {
        Scope::Global => agent_name(&target.agent).to_owned(),
        Scope::Project(id) => {
            let project = snapshot.projects.get(id);
            let name = project.map(|p| p.name.as_str()).unwrap_or("未知项目");
            if project.is_some() {
                format!("{} · {}", agent_name(&target.agent), name)
            } else {
                format!(
                    "{} · {} ({})",
                    agent_name(&target.agent),
                    name,
                    short_id(id.as_str())
                )
            }
        }
    }
}

fn skill_label(skill_id: &SkillId, snapshot: &Snapshot) -> String {
    snapshot
        .skills
        .get(skill_id)
        .map(|skill| skill.name.clone())
        .unwrap_or_else(|| skill_id.as_str().to_owned())
}

/// Targets restricted to one scope; the only place target lists are built.
fn scope_targets<'a>(
    targets: &'a std::collections::BTreeMap<TargetId, TargetState>,
    scope: &Scope,
) -> Vec<(&'a TargetId, &'a TargetState)> {
    targets
        .iter()
        .filter(|(target, _)| &target.scope == scope)
        .collect()
}

fn scope_filter_failure(scope: &Scope) -> String {
    match scope {
        Scope::Global => "global".to_owned(),
        Scope::Project(id) => format!("project:{}", id.as_str()),
    }
}

/// Build the final command from a draft and explicitly selected targets.
/// Cross-scope targets are rejected here as a second line of defense.
fn command_for(draft: &Draft, selected: &[TargetId], scope: &Scope) -> Result<Command, String> {
    if selected.iter().any(|target| &target.scope != scope) {
        return Err(format!(
            "目标超出当前作用域 {}，已拒绝执行",
            scope_filter_failure(scope)
        ));
    }
    Ok(match draft {
        Draft::TogglePreset { preset_id, enabled } => Command::SetPresetEnabled {
            preset_id: preset_id.clone(),
            targets: selected.to_vec(),
            enabled: *enabled,
        },
        Draft::ToggleSkill { skill_id, enabled } => Command::SetSkillEnabled {
            skill_id: skill_id.clone(),
            targets: selected.to_vec(),
            enabled: *enabled,
        },
        Draft::CreatePreset {
            preset_id,
            name,
            members,
        } => Command::CreatePreset {
            preset_id: preset_id.clone(),
            name: name.clone(),
            members: members.clone(),
        },
        Draft::EditMembers {
            preset_id,
            expected_revision,
            members,
        } => Command::UpdatePreset {
            preset_id: preset_id.clone(),
            expected_revision: *expected_revision,
            members: members.clone(),
            sync_targets: selected.to_vec(),
            closed_members: ClosedMembersPolicy::Unspecified,
        },
    })
}

/// Grouped, human-readable preview text. No UUIDs for known projects or
/// known skills.
fn build_preview_text(plan: &Plan, snapshot: &Snapshot) -> String {
    let mut lines = vec![format!(
        "影响预览：{} 项变更，{} 个目标",
        plan.effects.len(),
        plan.target_updates.len()
    )];
    let mut by_target: std::collections::BTreeMap<&TargetId, Vec<String>> = Default::default();
    for effect in &plan.effects {
        match effect {
            packcaptain::domain::Effect::SetSkillEnabled {
                target,
                skill,
                enabled,
            } => {
                let verb = if *enabled { "开启" } else { "关闭" };
                by_target.entry(target).or_default().push(format!(
                    "{} skill {}",
                    verb,
                    skill_label(skill, snapshot)
                ));
            }
        }
    }
    for (target, changes) in by_target {
        lines.push(format!(
            "{}：{}",
            target_label(target, snapshot),
            changes.join("，")
        ));
    }
    for definition in plan.preset_updates.values() {
        let members = definition
            .members
            .iter()
            .map(|id| skill_label(id, snapshot))
            .collect::<Vec<_>>()
            .join("、");
        lines.push(format!(
            "Preset {}（共享定义）r{}，成员：{}",
            definition.name, definition.revision, members
        ));
    }
    for target in plan.target_updates.keys() {
        lines.push(format!("更新目标状态：{}", target_label(target, snapshot)));
    }
    lines.push("Enter 确认写入，Esc 取消且不写入".to_owned());
    lines.join("\n")
}

/// The per-screen control contract. Hints and key gating read the same
/// source so the footer never advertises a key the screen ignores.
struct ScreenCapabilities {
    checkable: bool,
    searchable: bool,
    scrollable: bool,
    input: bool,
}

fn screen_capabilities(screen: Screen, filtering: bool) -> ScreenCapabilities {
    ScreenCapabilities {
        checkable: matches!(
            screen,
            Screen::Targets
                | Screen::CreateMembers
                | Screen::EditMembers
                | Screen::ProjectCandidates
        ),
        searchable: !filtering
            && matches!(
                screen,
                Screen::SkillPick | Screen::CreateMembers | Screen::EditMembers
            ),
        scrollable: screen == Screen::Confirm,
        input: filtering || matches!(screen, Screen::CreateName | Screen::ProjectRoot),
    }
}

fn enter_verb(screen: Screen) -> &'static str {
    match screen {
        Screen::ScopeRoot => "Enter 进入",
        Screen::PresetList => "Enter 查看",
        Screen::PresetActions => "Enter 执行",
        Screen::SkillPick => "Enter 打开 Skill 操作",
        Screen::SkillActions => "Enter 选择",
        Screen::CreateName | Screen::ProjectRoot => "Enter 确认",
        Screen::CreateMembers => "Enter 继续",
        Screen::EditMembers => "Enter 选择同步目标",
        Screen::Targets => "Enter 预览",
        Screen::Confirm => "Enter 确认写入",
        Screen::ProjectCandidates => "Enter 认领",
        Screen::EditPolicy => "Enter 选择策略",
    }
}

fn hint_for(screen: Screen, filtering: bool) -> String {
    let caps = screen_capabilities(screen, filtering);
    let mut parts: Vec<&str> = Vec::new();
    if filtering {
        parts.push("输入搜索词，Enter 结束，Esc 清空");
    } else if caps.input {
        parts.push("直接输入文本");
        parts.push(enter_verb(screen));
    } else if caps.scrollable {
        parts.push("↑↓/j/k 或 PgUp/PgDn 滚动");
        parts.push(enter_verb(screen));
    } else {
        parts.push("↑↓/j/k 移动");
        if caps.searchable {
            parts.push("/ 搜索");
        }
        if caps.checkable {
            parts.push("Space 勾选");
        }
        parts.push(enter_verb(screen));
    }
    if !filtering {
        parts.push(match screen {
            Screen::ScopeRoot => "Esc 退出",
            Screen::Confirm => "Esc 取消且不写入",
            _ => "Esc 返回",
        });
    }
    parts.join("，")
}

/// Pure text-key routing helper; asserted directly by unit tests.
#[derive(Clone, Copy, PartialEq, Debug)]
enum NavAction {
    Up,
    Down,
}

/// Typed SkillActions menu entries; behavior never matches on labels.
#[derive(Clone, Copy, PartialEq, Debug)]
enum SkillToggle {
    Enable,
    Disable,
}

fn skill_toggle_label(toggle: SkillToggle) -> &'static str {
    match toggle {
        SkillToggle::Enable => "开启此 Skill",
        SkillToggle::Disable => "关闭此 Skill",
    }
}

#[derive(Clone)]
enum Row {
    Preset(PresetId, String),
    Skill(SkillId, String),
    Target(TargetId, String),
    Candidate(PathBuf, String),
    Project(ProjectId, String),
    Action(&'static str),
    SkillAction(SkillToggle),
}

impl Row {
    fn key(&self) -> String {
        match self {
            Self::Preset(id, _) => format!("preset:{}", id.as_str()),
            Self::Skill(id, _) => format!("skill:{}", id.as_str()),
            Self::Target(id, _) => format!("target:{id}"),
            Self::Candidate(path, _) => format!("candidate:{}", path.display()),
            Self::Project(id, _) => format!("project:{}", id.as_str()),
            Self::Action(label) => format!("action:{label}"),
            Self::SkillAction(toggle) => match toggle {
                SkillToggle::Enable => "skill-action:enable".to_owned(),
                SkillToggle::Disable => "skill-action:disable".to_owned(),
            },
        }
    }

    fn checkable(&self) -> bool {
        matches!(
            self,
            Self::Skill(..) | Self::Target(..) | Self::Candidate(..)
        )
    }
}

struct App {
    engine: Engine<NativeAdapters>,
    home: PathBuf,
    snapshot: Snapshot,
    discovery: DiscoveryReport,
    theme: Theme,
    screen: Screen,
    scope: Scope,
    focus_preset: Option<PresetId>,
    focus_skill: Option<SkillId>,
    draft: Option<Draft>,
    rows: Vec<Row>,
    selected: usize,
    selected_key: Option<String>,
    checked: BTreeSet<String>,
    filter: String,
    filtering: bool,
    input: String,
    notice: String,
    diagnostics: Vec<String>,
    pending: Option<Command>,
    preview: String,
    preview_plan: Option<Plan>,
    preview_scroll: u16,
    project_root: PathBuf,
    candidates: Vec<ProjectCandidate>,
}

impl App {
    fn new(engine: Engine<NativeAdapters>, home: PathBuf) -> Self {
        Self {
            engine,
            home,
            snapshot: Snapshot::default(),
            discovery: DiscoveryReport::default(),
            theme: MOCHA,
            screen: Screen::ScopeRoot,
            scope: Scope::Global,
            focus_preset: None,
            focus_skill: None,
            draft: None,
            rows: Vec::new(),
            selected: 0,
            selected_key: None,
            checked: BTreeSet::new(),
            filter: String::new(),
            filtering: false,
            input: String::new(),
            notice: "Enter 进入作用域，Esc 退出，q 退出".into(),
            diagnostics: Vec::new(),
            pending: None,
            preview: String::new(),
            preview_plan: None,
            preview_scroll: 0,
            project_root: PathBuf::new(),
            candidates: Vec::new(),
        }
    }

    fn breadcrumb(&self) -> String {
        let mut parts = vec!["PackCaptain".to_owned()];
        if self.screen != Screen::ScopeRoot {
            parts.push(scope_label(&self.scope, &self.snapshot));
            match self.screen {
                Screen::PresetList
                | Screen::SkillPick
                | Screen::SkillActions
                | Screen::CreateName
                | Screen::CreateMembers => parts.push("Presets".to_owned()),
                Screen::PresetActions | Screen::EditMembers | Screen::EditPolicy => {
                    parts.push(
                        self.focus_preset
                            .as_ref()
                            .map(|id| {
                                self.snapshot
                                    .presets
                                    .get(id)
                                    .map(|p| p.name.clone())
                                    .unwrap_or_else(|| id.as_str().to_owned())
                            })
                            .unwrap_or_default(),
                    );
                }
                Screen::Targets | Screen::Confirm => parts.push("目标确认".to_owned()),
                Screen::ProjectRoot | Screen::ProjectCandidates => {
                    parts.push("认领项目".to_owned())
                }
                Screen::ScopeRoot => {}
            }
        }
        parts.join(" / ")
    }

    /// Entering a scope invalidates all per-scope selection state.
    fn set_scope(&mut self, scope: Scope) {
        if self.scope == scope && self.screen != Screen::ScopeRoot {
            return;
        }
        self.scope = scope;
        self.screen = Screen::PresetList;
        self.focus_skill = None;
        self.reset_scope_state();
    }

    fn reset_scope_state(&mut self) {
        self.selected = 0;
        self.selected_key = None;
        self.checked.clear();
        self.filter.clear();
        self.filtering = false;
        self.draft = None;
        self.pending = None;
        self.preview_plan = None;
        self.preview.clear();
        self.preview_scroll = 0;
    }

    fn invalidate_preview(&mut self) {
        self.pending = None;
        self.preview_plan = None;
        self.preview.clear();
        self.preview_scroll = 0;
    }

    fn clear_operation(&mut self) {
        self.invalidate_preview();
        self.draft = None;
        self.checked.clear();
    }

    fn cancel_operation(&mut self) {
        self.clear_operation();
        self.notice = "已取消，未写入".into();
    }

    fn refresh(&mut self) -> Result<(), String> {
        self.snapshot = self.engine.snapshot().map_err(|e| e.to_string())?;
        self.discovery = NativeAdapters.discover(&self.home, &self.snapshot);
        self.diagnostics = self.discovery.diagnostics.clone();
        self.rows.clear();
        match self.screen {
            Screen::ScopeRoot => {
                self.rows.push(Row::Action("全局"));
                self.rows.extend(
                    self.snapshot
                        .projects
                        .values()
                        .map(|project| Row::Project(project.id.clone(), project.name.clone())),
                );
                self.rows.push(Row::Action("扫描并认领项目"));
            }
            Screen::PresetList => {
                self.rows
                    .extend(self.snapshot.presets.values().map(|preset| {
                        Row::Preset(
                            preset.id.clone(),
                            format!(
                                "{}  r{}  [{} 成员]",
                                preset.name,
                                preset.revision,
                                preset.members.len()
                            ),
                        )
                    }));
                self.rows.push(Row::Action("新建 Preset"));
                self.rows.push(Row::Action("Skills 管理"));
            }
            Screen::PresetActions => {
                if let Some(id) = &self.focus_preset
                    && let Some(definition) = self.snapshot.presets.get(id)
                {
                    self.rows.push(Row::Action("开启（选择 agents）"));
                    self.rows.push(Row::Action("关闭（选择 agents）"));
                    self.rows.push(Row::Action("编辑共享成员并同步"));
                    self.notice = format!(
                        "Preset {}（共享定义）r{}，{} 成员",
                        definition.name,
                        definition.revision,
                        definition.members.len()
                    );
                }
            }
            Screen::SkillPick | Screen::CreateMembers | Screen::EditMembers => {
                let query = self.filter.to_lowercase();
                let merged = self.discovery.merge_into(&self.snapshot);
                self.rows.extend(
                    merged
                        .skills
                        .values()
                        .filter(|skill| {
                            query.is_empty()
                                || skill.name.to_lowercase().contains(&query)
                                || skill.id.as_str().to_lowercase().contains(&query)
                                || skill
                                    .source_path
                                    .to_string_lossy()
                                    .to_lowercase()
                                    .contains(&query)
                        })
                        .map(|skill| {
                            Row::Skill(
                                skill.id.clone(),
                                format!(
                                    "{}  [{}]  {}",
                                    skill.name,
                                    availability_label(&skill.availability),
                                    skill.source_path.display()
                                ),
                            )
                        }),
                );
            }
            Screen::SkillActions => {
                if let Some(id) = &self.focus_skill {
                    let merged = self.discovery.merge_into(&self.snapshot);
                    let availability = merged.skills.get(id).map(|skill| skill.availability);
                    self.rows.push(Row::SkillAction(SkillToggle::Enable));
                    self.rows.push(Row::SkillAction(SkillToggle::Disable));
                    self.notice = format!(
                        "Skill {}，{}",
                        skill_label(id, &self.snapshot),
                        availability
                            .as_ref()
                            .map(availability_label)
                            .unwrap_or("状态未知")
                    );
                }
            }
            Screen::ProjectCandidates => {
                self.rows.extend(self.candidates.iter().map(|candidate| {
                    Row::Candidate(
                        candidate.path.clone(),
                        format!("{}  {}", candidate.name, candidate.path.display()),
                    )
                }));
                self.rows.push(Row::Action("主动添加扫描根目录"));
            }
            Screen::EditPolicy => {
                self.rows
                    .push(Row::Action("保留相关手动关闭（keep，默认）"));
                self.rows
                    .push(Row::Action("恢复相关手动关闭成员（restore）"));
            }
            Screen::Targets => {
                let merged = self.discovery.merge_into(&self.snapshot);
                let preset_id = match &self.draft {
                    Some(Draft::EditMembers { preset_id, .. }) => Some(preset_id),
                    _ => None,
                };
                self.rows.extend(
                    scope_targets(&merged.targets, &self.scope)
                        .into_iter()
                        .filter(|(_, state)| match preset_id {
                            Some(id) => {
                                state.presets.get(id).is_some_and(|applied| applied.enabled)
                            }
                            None => true,
                        })
                        .map(|(target, state)| {
                            Row::Target(
                                target.clone(),
                                format!(
                                    "{}  开启 {} skills，手动关闭 {}，缺失 {}",
                                    target_label(target, &self.snapshot),
                                    state.enabled_skills.len(),
                                    state.manual_closed.len(),
                                    state
                                        .enabled_skills
                                        .iter()
                                        .filter(|skill| {
                                            merged
                                                .skills
                                                .get(*skill)
                                                .is_some_and(|record| !matches!(
                                                    record.availability,
                                                    packcaptain::domain::SkillAvailability::Available
                                                ))
                                        })
                                        .count()
                                ),
                            )
                        }),
                );
            }
            Screen::Confirm | Screen::CreateName | Screen::ProjectRoot => {}
        }
        if self.rows.is_empty() {
            self.selected = 0;
            self.selected_key = None;
        } else {
            if let Some(key) = &self.selected_key
                && let Some(position) = self.rows.iter().position(|row| row.key() == *key)
            {
                self.selected = position;
            }
            self.selected = self.selected.min(self.rows.len() - 1);
            self.selected_key = Some(self.rows[self.selected].key());
        }
        Ok(())
    }

    fn toggle_checked(&mut self) {
        if let Some(row) = self.rows.get(self.selected)
            && row.checkable()
        {
            let key = row.key();
            if !self.checked.remove(&key) {
                self.checked.insert(key);
            }
        }
    }

    fn event(&mut self, key: KeyCode) -> bool {
        if self.filtering {
            match key {
                KeyCode::Esc => {
                    self.filtering = false;
                    self.filter.clear();
                }
                KeyCode::Enter => self.filtering = false,
                KeyCode::Backspace => {
                    self.filter.pop();
                }
                KeyCode::Char(character) if !character.is_control() => self.filter.push(character),
                _ => {}
            }
            let _ = self.refresh();
            return false;
        }
        let caps = screen_capabilities(self.screen, self.filtering);
        match key {
            KeyCode::Esc => return self.go_back(),
            KeyCode::Char(' ') if caps.checkable && !caps.input => {
                self.toggle_checked();
                return false;
            }
            KeyCode::Char('/') if caps.searchable => {
                self.filtering = true;
                self.filter.clear();
                return false;
            }
            KeyCode::Char('j') | KeyCode::Char('k') if !caps.input => {
                if caps.scrollable {
                    let delta = if key == KeyCode::Char('j') { 1 } else { -1 };
                    self.preview_scroll = (self.preview_scroll as i32 + delta).max(0) as u16;
                    return false;
                }
                return self.move_cursor(if key == KeyCode::Char('k') {
                    NavAction::Up
                } else {
                    NavAction::Down
                });
            }
            KeyCode::Up if caps.scrollable => {
                self.preview_scroll = self.preview_scroll.saturating_sub(1);
                return false;
            }
            KeyCode::Down if caps.scrollable => {
                self.preview_scroll = self.preview_scroll.saturating_add(1);
                return false;
            }
            KeyCode::Up if !caps.input => {
                return self.move_cursor(NavAction::Up);
            }
            KeyCode::Down if !caps.input => {
                return self.move_cursor(NavAction::Down);
            }
            KeyCode::PageUp if caps.scrollable => {
                self.preview_scroll = self.preview_scroll.saturating_sub(8);
                return false;
            }
            KeyCode::PageDown if caps.scrollable => {
                self.preview_scroll = self.preview_scroll.saturating_add(8);
                return false;
            }
            KeyCode::Char('q') if !caps.input => return true,
            KeyCode::Char('T') if !caps.input => {
                self.theme = tui_theme::next(self.theme);
                return false;
            }
            _ => {}
        }
        match self.screen {
            Screen::ScopeRoot => {
                if key == KeyCode::Enter {
                    match self.rows.get(self.selected) {
                        Some(Row::Action("全局")) => {
                            self.set_scope(Scope::Global);
                            let _ = self.refresh();
                        }
                        Some(Row::Action("扫描并认领项目")) => {
                            self.screen = Screen::ProjectRoot;
                            self.input.clear();
                            self.notice = "输入项目根目录，Enter 扫描候选".into();
                        }
                        Some(Row::Project(project_id, _)) => {
                            let scope = Scope::Project(project_id.clone());
                            self.set_scope(scope);
                            let _ = self.refresh();
                        }
                        _ => {}
                    }
                }
            }
            Screen::PresetList => {
                if key == KeyCode::Enter {
                    let focused = match self.rows.get(self.selected) {
                        Some(Row::Preset(id, _)) => Some(id.clone()),
                        _ => None,
                    };
                    if let Some(id) = focused {
                        self.open_preset_actions(&id);
                    } else if self
                        .rows
                        .get(self.selected)
                        .is_some_and(|row| row.key() == "action:新建 Preset")
                    {
                        self.start_create_preset();
                    } else if self
                        .rows
                        .get(self.selected)
                        .is_some_and(|row| row.key() == "action:Skills 管理")
                    {
                        self.focus_skill = None;
                        self.screen = Screen::SkillPick;
                        self.reset_list_state();
                        let _ = self.refresh();
                    }
                }
            }
            Screen::PresetActions => {
                if key == KeyCode::Enter {
                    let action = match self.rows.get(self.selected) {
                        Some(Row::Action(label)) => *label,
                        _ => return false,
                    };
                    let Some(id) = self.focus_preset.clone() else {
                        return false;
                    };
                    match action {
                        "开启（选择 agents）" => {
                            self.draft = Some(Draft::TogglePreset {
                                preset_id: id,
                                enabled: true,
                            });
                            self.goto_targets("Space 勾选 agents，Enter 预览");
                        }
                        "关闭（选择 agents）" => {
                            self.draft = Some(Draft::TogglePreset {
                                preset_id: id,
                                enabled: false,
                            });
                            self.goto_targets("Space 勾选 agents，Enter 预览");
                        }
                        "编辑共享成员并同步" => {
                            self.open_edit_members(&id);
                        }
                        _ => {}
                    }
                }
            }
            Screen::SkillPick => {
                if key == KeyCode::Enter
                    && let Some(Row::Skill(id, _)) = self.rows.get(self.selected)
                {
                    self.focus_skill = Some(id.clone());
                    self.screen = Screen::SkillActions;
                    self.reset_list_state();
                    let _ = self.refresh();
                }
            }
            Screen::SkillActions => {
                if key == KeyCode::Enter {
                    let toggle = match self.rows.get(self.selected) {
                        Some(Row::SkillAction(toggle)) => *toggle,
                        _ => return false,
                    };
                    let Some(id) = self.focus_skill.clone() else {
                        return false;
                    };
                    self.draft = Some(Draft::ToggleSkill {
                        skill_id: id,
                        enabled: toggle == SkillToggle::Enable,
                    });
                    self.goto_targets("Space 勾选 agents，Enter 预览");
                }
            }
            Screen::CreateName => match key {
                KeyCode::Char(c) if !c.is_control() => self.input.push(c),
                KeyCode::Backspace => {
                    self.input.pop();
                }
                KeyCode::Enter if !self.input.trim().is_empty() => {
                    self.screen = Screen::CreateMembers;
                    self.reset_list_state();
                    let _ = self.refresh();
                    self.notice = "选择成员，Space 勾选，Enter 继续".into();
                }
                _ => {}
            },
            Screen::CreateMembers => {
                if key == KeyCode::Enter {
                    let members = self.checked_skill_ids();
                    self.draft = Some(Draft::CreatePreset {
                        preset_id: PresetId::new(),
                        name: self.input.trim().to_owned(),
                        members,
                    });
                    self.preview_draft(&[]);
                }
            }
            Screen::EditMembers => {
                if key == KeyCode::Enter {
                    let members = self.checked_skill_ids();
                    let Some(Draft::EditMembers {
                        members: draft_members,
                        ..
                    }) = self.draft.as_mut()
                    else {
                        return false;
                    };
                    *draft_members = members;
                    self.goto_targets("选择已开启此 Preset 的 agents，Space 勾选，Enter 预览同步");
                }
            }
            Screen::Targets => {
                if key == KeyCode::Enter {
                    let selected = self
                        .rows
                        .iter()
                        .filter_map(|row| match row {
                            Row::Target(id, _) if self.checked.contains(&row.key()) => {
                                Some(id.clone())
                            }
                            _ => None,
                        })
                        .collect::<Vec<_>>();
                    if selected.is_empty() {
                        self.notice = if self.rows.is_empty()
                            && matches!(self.draft, Some(Draft::EditMembers { .. }))
                        {
                            "当前作用域无已开启目标，无法保存成员；Esc 返回编辑".into()
                        } else {
                            "至少选择一个目标".into()
                        };
                    } else {
                        let targets = selected.clone();
                        self.preview_draft(&targets);
                    }
                }
            }
            Screen::EditPolicy => {
                if key == KeyCode::Enter {
                    let policy = if self.selected == 0 {
                        ClosedMembersPolicy::Keep
                    } else {
                        ClosedMembersPolicy::Restore
                    };
                    if let Some(Command::UpdatePreset { closed_members, .. }) =
                        self.pending.as_mut()
                    {
                        *closed_members = policy;
                    }
                    self.preview_pending();
                }
            }
            Screen::ProjectRoot => match key {
                KeyCode::Char(character) if !character.is_control() => self.input.push(character),
                KeyCode::Backspace => {
                    self.input.pop();
                }
                KeyCode::Enter => {
                    let root = PathBuf::from(self.input.trim());
                    if !root.is_absolute() {
                        self.notice = "项目扫描路径必须是绝对路径".into();
                    } else {
                        match NativeAdapters::scan_candidates(&root) {
                            Ok(candidates) => {
                                self.project_root = root;
                                self.candidates = candidates;
                                self.checked.clear();
                                self.reset_list_state();
                                self.screen = Screen::ProjectCandidates;
                                let _ = self.refresh();
                                self.notice =
                                    "Space 选择候选，Enter 认领；也可主动添加扫描根目录".into();
                            }
                            Err(error) => self.notice = format!("扫描失败：{error}"),
                        }
                    }
                }
                _ => {}
            },
            Screen::ProjectCandidates => {
                if key == KeyCode::Enter {
                    let chosen: Vec<(PathBuf, String)> = self
                        .rows
                        .iter()
                        .filter_map(|row| match row {
                            Row::Candidate(path, name) if self.checked.contains(&row.key()) => {
                                Some((path.clone(), name.clone()))
                            }
                            _ => None,
                        })
                        .collect();
                    let chosen = if chosen.is_empty() {
                        match self.rows.get(self.selected) {
                            Some(Row::Candidate(path, name)) => vec![(path.clone(), name.clone())],
                            Some(Row::Action("主动添加扫描根目录")) => vec![(
                                self.project_root.clone(),
                                self.project_root
                                    .file_name()
                                    .and_then(|name| name.to_str())
                                    .unwrap_or("project")
                                    .to_owned(),
                            )],
                            _ => Vec::new(),
                        }
                    } else {
                        chosen
                    };
                    if chosen.is_empty() {
                        self.notice = "请选择要认领的项目目录".into();
                    } else {
                        let mut errors = Vec::new();
                        for (root, name) in chosen {
                            let project = ProjectRecord {
                                id: ProjectId::new(),
                                name,
                                root,
                            };
                            if let Err(error) = self.engine.claim_project(project) {
                                errors.push(error.to_string());
                            }
                        }
                        self.screen = Screen::ScopeRoot;
                        self.notice = if errors.is_empty() {
                            "项目目录已认领".into()
                        } else {
                            format!("部分认领失败：{}", errors.join("；"))
                        };
                        let _ = self.refresh();
                    }
                }
            }
            Screen::Confirm => {
                if key == KeyCode::Enter {
                    return self.confirm_execute();
                }
            }
        }
        if let Some(row) = self.rows.get(self.selected) {
            self.selected_key = Some(row.key());
        }
        false
    }

    fn move_cursor(&mut self, action: NavAction) -> bool {
        match action {
            NavAction::Up => self.selected = self.selected.saturating_sub(1),
            NavAction::Down => {
                self.selected = (self.selected + 1).min(self.rows.len().saturating_sub(1))
            }
        }
        if let Some(row) = self.rows.get(self.selected) {
            self.selected_key = Some(row.key());
        }
        false
    }

    /// One-level back, preserving the current scope and invalidating any
    /// canceled plan.
    fn go_back(&mut self) -> bool {
        match self.screen {
            Screen::ScopeRoot => return true,
            Screen::Confirm | Screen::Targets | Screen::EditPolicy => {
                self.screen = self
                    .draft
                    .as_ref()
                    .map(Draft::source_screen)
                    .unwrap_or(Screen::PresetList);
                self.invalidate_preview();
                self.reset_list_state();
                if let Some(
                    Draft::EditMembers { members, .. } | Draft::CreatePreset { members, .. },
                ) = &self.draft
                {
                    self.checked = members
                        .iter()
                        .map(|skill| format!("skill:{}", skill.as_str()))
                        .collect();
                }
                self.notice = "已取消，未写入".into();
                if matches!(
                    self.draft,
                    Some(Draft::TogglePreset { .. } | Draft::ToggleSkill { .. })
                ) {
                    self.checked.clear();
                }
            }
            Screen::PresetList => self.screen = Screen::ScopeRoot,
            Screen::PresetActions => self.screen = Screen::PresetList,
            Screen::SkillPick | Screen::CreateName => self.screen = Screen::PresetList,
            Screen::SkillActions => {
                self.draft = None;
                self.screen = Screen::SkillPick;
                self.reset_list_state();
            }
            Screen::CreateMembers => self.screen = Screen::CreateName,
            Screen::EditMembers => {
                self.cancel_operation();
                self.screen = Screen::PresetActions;
            }

            Screen::ProjectRoot => self.screen = Screen::ScopeRoot,
            Screen::ProjectCandidates => self.screen = Screen::ProjectRoot,
        }
        let _ = self.refresh();
        false
    }

    fn reset_list_state(&mut self) {
        self.selected = 0;
        self.selected_key = None;
        self.checked.clear();
    }

    fn start_create_preset(&mut self) {
        self.screen = Screen::CreateName;
        self.input.clear();
        self.notice = "输入新 Preset 名称并按 Enter".into();
    }

    fn open_preset_actions(&mut self, id: &PresetId) {
        self.focus_preset = Some(id.clone());
        self.screen = Screen::PresetActions;
        self.reset_list_state();
        let _ = self.refresh();
    }

    fn open_edit_members(&mut self, id: &PresetId) {
        self.reset_list_state();
        if let Some(definition) = self.snapshot.presets.get(id) {
            self.draft = Some(Draft::EditMembers {
                preset_id: id.clone(),
                expected_revision: definition.revision,
                members: definition.members.clone(),
            });
            self.checked = definition
                .members
                .iter()
                .map(|skill| format!("skill:{}", skill.as_str()))
                .collect();
            self.screen = Screen::EditMembers;
            let _ = self.refresh();
            self.notice = "仅支持同步保存：Space 切换成员，Enter 选择已开启目标".into();
        }
    }

    fn goto_targets(&mut self, notice: &str) {
        self.screen = Screen::Targets;
        self.reset_list_state();
        let _ = self.refresh();
        self.notice =
            if self.rows.is_empty() && matches!(self.draft, Some(Draft::EditMembers { .. })) {
                "当前作用域无已开启目标，无法保存成员；Esc 返回编辑".into()
            } else {
                notice.into()
            };
    }

    fn checked_skill_ids(&self) -> BTreeSet<SkillId> {
        self.checked
            .iter()
            .filter_map(|key| {
                key.strip_prefix("skill:")
                    .map(|id| SkillId::from(id.to_owned()))
            })
            .collect()
    }

    /// Build the final command from the draft plus explicitly selected
    /// targets, then preview.
    fn preview_draft(&mut self, selected: &[TargetId]) {
        let Some(draft) = self.draft.clone() else {
            return;
        };
        match command_for(&draft, selected, &self.scope) {
            Ok(command) => {
                self.pending = Some(command);
                self.preview_pending();
            }
            Err(message) => self.notice = message,
        }
    }

    fn preview_pending(&mut self) {
        if let Some(command) = &self.pending {
            match self.engine.preview_with_native_discovery(command) {
                Ok(plan) => self.show_preview(plan),
                Err(EngineError::Conflict(conflict))
                    if conflict.code == "closed-members-policy-required" =>
                {
                    self.screen = Screen::EditPolicy;
                    self.reset_list_state();
                    let _ = self.refresh();
                    let targets = conflict
                        .affected_targets
                        .iter()
                        .map(|target| target_label(target, &self.snapshot))
                        .collect::<Vec<_>>()
                        .join("，");
                    let skills = conflict
                        .affected_skills
                        .iter()
                        .map(|skill| skill_label(skill, &self.snapshot))
                        .collect::<Vec<_>>()
                        .join("，");
                    self.notice = format!(
                        "{}。受影响 agents：{}；skills：{}。默认 keep，可改选 restore。",
                        conflict.message, targets, skills
                    );
                }
                Err(e) => self.notice = format!("无法预览：{e}"),
            }
        }
    }

    fn show_preview(&mut self, plan: Plan) {
        self.preview = build_preview_text(&plan, &self.snapshot);
        self.preview_scroll = 0;
        self.screen = Screen::Confirm;
        self.preview_plan = Some(plan);
        self.notice = "Enter 确认写入，Esc 取消且不写入".into();
    }

    fn confirm_execute(&mut self) -> bool {
        if let Some(command) = self.pending.clone() {
            let Some(confirmed) = self.preview_plan.as_ref() else {
                self.preview_pending();
                return false;
            };
            match self
                .engine
                .execute_confirmed_with_native_discovery(&command, confirmed)
            {
                Ok(report) => {
                    let applied = report
                        .targets
                        .iter()
                        .filter(|target| target.status == TargetOutcomeStatus::Applied)
                        .count();
                    let failed: Vec<_> = report
                        .targets
                        .iter()
                        .filter(|target| target.status == TargetOutcomeStatus::Failed)
                        .map(|target| {
                            format!(
                                "{}：{}",
                                target_label(&target.target, &self.snapshot),
                                target.message.as_deref().unwrap_or("失败")
                            )
                        })
                        .collect();
                    self.notice = if failed.is_empty() {
                        format!("执行成功：{applied} 个目标")
                    } else {
                        format!(
                            "部分失败：成功 {applied}，失败 {}。{}",
                            failed.len(),
                            failed.join("；")
                        )
                    };
                }
                Err(EngineError::PreviewChanged(plan)) => {
                    self.show_preview(*plan);
                    self.preview = format!("状态已变化，请重新确认\n{}", self.preview);
                    self.notice = "状态已变化，请重新确认".into();
                    return false;
                }
                Err(EngineError::Conflict(conflict))
                    if conflict.code == "closed-members-policy-required" =>
                {
                    self.preview_pending();
                    return false;
                }
                Err(e @ EngineError::Discovery { .. }) => {
                    self.notice = format!("执行失败：{e}。修正配置后重新预览");
                    self.preview = self.notice.clone();
                    return false;
                }
                Err(e) => self.notice = format!("执行失败：{e}。请重新编辑或预览"),
            }
        }
        self.clear_operation();
        self.screen = Screen::PresetList;
        let notice = self.notice.clone();
        let _ = self.refresh();
        self.notice = notice;
        false
    }
}

fn availability_label(availability: &packcaptain::domain::SkillAvailability) -> &'static str {
    match availability {
        packcaptain::domain::SkillAvailability::Available => "可用",
        packcaptain::domain::SkillAvailability::Missing => "缺失",
        packcaptain::domain::SkillAvailability::Unsupported => "不支持",
        packcaptain::domain::SkillAvailability::Unreadable => "不可读",
    }
}

pub fn run(engine: Engine<NativeAdapters>, home: PathBuf) -> i32 {
    if let Err(error) = enable_raw_mode() {
        eprintln!("无法开启终端：{error}");
        return 1;
    }
    let mut stdout = io::stdout();
    if let Err(error) = execute!(stdout, EnterAlternateScreen) {
        let _ = disable_raw_mode();
        eprintln!("无法进入 TUI：{error}");
        return 1;
    }
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = match Terminal::new(backend) {
        Ok(t) => t,
        Err(e) => {
            let _ = execute!(io::stdout(), LeaveAlternateScreen);
            let _ = disable_raw_mode();
            eprintln!("{e}");
            return 1;
        }
    };
    let mut app = App::new(engine, home);
    let _ = app.refresh();
    let result = loop {
        if let Err(e) = terminal.draw(|f| draw(f, &app)) {
            break Err(e.to_string());
        }
        match event::read() {
            Ok(Event::Key(key)) if key.kind == KeyEventKind::Press => {
                if app.event(key.code) {
                    break Ok(());
                }
            }
            Ok(_) => {}
            Err(e) => break Err(e.to_string()),
        }
    };
    let backend = terminal.backend_mut();
    let _ = execute!(backend, LeaveAlternateScreen);
    let _ = disable_raw_mode();
    if let Err(error) = result {
        eprintln!("TUI 错误：{error}");
        1
    } else {
        0
    }
}

fn styled_block<'a>(title: &str, theme: &Theme) -> Block<'a> {
    Block::default()
        .borders(Borders::ALL)
        .title(Span::styled(
            title.to_owned(),
            Style::default().fg(theme.accent),
        ))
        .border_style(Style::default().fg(theme.border))
        .style(Style::default().bg(theme.bg).fg(theme.fg))
}

fn draw(frame: &mut ratatui::Frame<'_>, app: &App) {
    let theme = app.theme;
    frame.render_widget(
        Block::default().style(Style::default().bg(theme.bg)),
        frame.area(),
    );
    let areas = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(if app.diagnostics.is_empty() { 3 } else { 5 }),
            Constraint::Min(5),
            Constraint::Length(4),
        ])
        .split(frame.area());
    let mut header_lines = vec![Line::from(Span::styled(
        format!("{}  ·  主题 {}（T 切换）", app.breadcrumb(), theme.name),
        Style::default().fg(theme.fg),
    ))];
    if !app.diagnostics.is_empty() {
        header_lines.push(Line::from(Span::styled(
            format!(
                "扫描诊断 {} 条：{}",
                app.diagnostics.len(),
                app.diagnostics[0]
            ),
            Style::default().fg(theme.danger),
        )));
    }
    let header = Paragraph::new(header_lines)
        .wrap(Wrap { trim: false })
        .block(styled_block("工作区", &theme));
    frame.render_widget(header, areas[0]);
    if app.screen == Screen::Confirm {
        let preview = Paragraph::new(app.preview.as_str())
            .wrap(Wrap { trim: false })
            .scroll((app.preview_scroll, 0))
            .block(styled_block(
                "逐项影响（j/k 或 PageUp/PageDown 滚动）",
                &theme,
            ));
        frame.render_widget(preview, areas[1]);
    } else if matches!(app.screen, Screen::CreateName | Screen::ProjectRoot) {
        let block = styled_block(screen_title(app.screen), &theme);
        let inner = block.inner(areas[1]);
        frame.render_widget(Paragraph::new(app.input.as_str()).block(block), areas[1]);
        let width = Line::from(app.input.as_str()).width() as u16;
        if inner.width > 0 && inner.height > 0 {
            frame.set_cursor_position((inner.x + width.min(inner.width - 1), inner.y));
        }
    } else {
        let items: Vec<ListItem<'_>> = app
            .rows
            .iter()
            .enumerate()
            .map(|(index, row)| {
                let checked = app.checked.contains(&row.key());
                let (check, label) = match row {
                    Row::Preset(_, label) | Row::Project(_, label) => ("", label.as_str()),
                    _ => (if checked { "[x] " } else { "[ ] " }, row_label(row)),
                };
                let mut spans = vec![Span::raw(if index == app.selected { "› " } else { "  " })];
                if !check.is_empty() {
                    spans.push(Span::styled(
                        check,
                        Style::default().fg(if checked { theme.status } else { theme.dim }),
                    ));
                }
                spans.push(Span::raw(label));
                ListItem::new(Line::from(spans))
            })
            .collect();
        let mut list_state = ListState::default();
        list_state.select(Some(app.selected));
        let list = List::new(items)
            .block(styled_block(screen_title(app.screen), &theme))
            .highlight_style(
                Style::default()
                    .bg(theme.highlight_bg)
                    .fg(theme.highlight_fg)
                    .add_modifier(Modifier::BOLD),
            )
            .highlight_symbol("");
        frame.render_stateful_widget(list, areas[1], &mut list_state);
    }
    let mut footer_lines = vec![Line::from(Span::styled(
        app.notice.clone(),
        Style::default().fg(theme.status),
    ))];
    footer_lines.push(Line::from(Span::styled(
        hint_for(app.screen, app.filtering),
        Style::default().fg(theme.dim),
    )));
    if app.filtering {
        footer_lines.push(Line::from(Span::raw(format!(
            "搜索：{}（Enter 确认，Esc 清空）",
            app.filter
        ))));
    }
    let footer = Paragraph::new(footer_lines).block(styled_block("操作与状态", &theme));
    frame.render_widget(footer, areas[2]);
}

fn row_label(row: &Row) -> &str {
    match row {
        Row::Preset(_, label)
        | Row::Skill(_, label)
        | Row::Target(_, label)
        | Row::Candidate(_, label)
        | Row::Project(_, label) => label,
        Row::Action(label) => label,
        Row::SkillAction(toggle) => skill_toggle_label(*toggle),
    }
}

fn screen_title(screen: Screen) -> &'static str {
    match screen {
        Screen::ScopeRoot => "选择作用域",
        Screen::PresetList => "Presets",
        Screen::PresetActions => "Preset 操作",
        Screen::SkillPick => "Skills 清单（/ 搜索，Enter 打开操作）",
        Screen::SkillActions => "Skill 操作",
        Screen::CreateName => "新建 Preset：输入名称",
        Screen::CreateMembers => "新建 Preset：选择技能成员（/ 搜索）",
        Screen::EditMembers => "编辑共享成员（/ 搜索）",
        Screen::Targets => "选择当前作用域 agents（Space 勾选）",
        Screen::Confirm => "执行预览",
        Screen::ProjectRoot => "输入项目扫描根目录",
        Screen::ProjectCandidates => "选择认领项目（Space 勾选）",
        Screen::EditPolicy => "手动关闭冲突策略（默认 keep）",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use packcaptain::domain::PresetDefinition;

    fn u3_fixture() -> (tempfile::TempDir, App, PresetId, SkillId) {
        let root = tempfile::tempdir().unwrap();
        let sandbox = root.path().canonicalize().unwrap();
        let home = sandbox.join("home");
        let source = home.join(".claude/skills/demo");
        std::fs::create_dir_all(&source).unwrap();
        std::fs::write(
            source.join("SKILL.md"),
            "---\nname: Demo\ndescription: U3 fixture\n---\n",
        )
        .unwrap();
        let mut app = App::new(
            Engine::new(home.clone(), sandbox.join("state"), NativeAdapters),
            home,
        );
        app.refresh().unwrap();
        let skill = app
            .discovery
            .merge_into(&app.snapshot)
            .skills
            .keys()
            .next()
            .unwrap()
            .clone();
        let preset = PresetId::from("u3-preset");
        app.engine
            .execute_with_native_discovery(&Command::CreatePreset {
                preset_id: preset.clone(),
                name: "Demo preset".into(),
                members: BTreeSet::from([skill.clone()]),
            })
            .unwrap();
        app.refresh().unwrap();
        (root, app, preset, skill)
    }

    fn u3_enable(app: &mut App, preset: &PresetId) -> TargetId {
        let target = TargetId {
            agent: AgentId::ClaudeCode,
            scope: Scope::Global,
        };
        app.engine
            .execute_with_native_discovery(&Command::SetPresetEnabled {
                preset_id: preset.clone(),
                targets: vec![target.clone()],
                enabled: true,
            })
            .unwrap();
        app.refresh().unwrap();
        target
    }

    fn u3_edit(app: &mut App) {
        app.event(KeyCode::Enter);
        app.event(KeyCode::Enter);
        app.event(KeyCode::Down);
        app.event(KeyCode::Down);
        app.event(KeyCode::Enter);
        assert_eq!(app.screen, Screen::EditMembers);
    }

    fn u3_render(app: &App) -> String {
        let mut terminal = Terminal::new(ratatui::backend::TestBackend::new(120, 18)).unwrap();
        terminal.draw(|frame| draw(frame, app)).unwrap();
        terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(|cell| cell.symbol())
            .collect()
    }

    #[test]
    fn u3_noop_edit_preserves_members() {
        let (_root, mut app, preset, skill) = u3_fixture();
        let target = u3_enable(&mut app, &preset);
        u3_edit(&mut app);
        assert_eq!(app.checked_skill_ids(), BTreeSet::from([skill.clone()]));
        assert!(u3_render(&app).contains("[x]"));
        app.event(KeyCode::Enter);
        assert_eq!(app.screen, Screen::Targets);
        assert!(app.checked.is_empty());
        assert_eq!(app.rows.len(), 1);
        assert_eq!(app.rows[0].key(), format!("target:{target}"));
        app.event(KeyCode::Char(' '));
        app.event(KeyCode::Enter);
        assert_eq!(app.screen, Screen::Confirm, "{}", app.notice);
        app.event(KeyCode::Enter);
        assert_eq!(
            app.engine.snapshot().unwrap().presets[&preset].members,
            BTreeSet::from([skill])
        );
    }

    #[test]
    fn u3_edit_without_enabled_target_is_unavailable() {
        let (_root, mut app, _preset, skill) = u3_fixture();
        let before = app.engine.snapshot().unwrap();
        u3_edit(&mut app);
        assert!(app.rows.iter().all(|row| !matches!(row, Row::Action(_))));
        assert!(u3_render(&app).replace(' ', "").contains("仅支持同步保存"));
        app.event(KeyCode::Enter);
        assert_eq!(app.screen, Screen::Targets);
        assert!(app.rows.is_empty());
        assert!(
            u3_render(&app)
                .replace(' ', "")
                .contains("当前作用域无已开启目标，无法保存成员")
        );
        app.event(KeyCode::Enter);
        assert!(app.pending.is_none());
        assert!(app.preview_plan.is_none());
        assert_eq!(app.engine.snapshot().unwrap(), before);
        app.event(KeyCode::Esc);
        assert_eq!(app.screen, Screen::EditMembers);
        assert_eq!(app.checked_skill_ids(), BTreeSet::from([skill]));
        app.event(KeyCode::Esc);
        assert_eq!(app.screen, Screen::PresetActions);
        assert_eq!(app.engine.snapshot().unwrap(), before);
    }

    #[test]
    fn u3_edit_targets_stay_in_current_scope() {
        let (_root, mut app, preset, _skill) = u3_fixture();
        u3_enable(&mut app, &preset);
        let project = ProjectRecord {
            id: ProjectId::from("u3-project".to_owned()),
            name: "Other project".into(),
            root: app.home.join("project"),
        };
        std::fs::create_dir_all(&project.root).unwrap();
        app.engine.claim_project(project.clone()).unwrap();
        app.engine
            .execute_with_native_discovery(&Command::SetPresetEnabled {
                preset_id: preset.clone(),
                targets: vec![TargetId {
                    agent: AgentId::ClaudeCode,
                    scope: Scope::Project(project.id.clone()),
                }],
                enabled: true,
            })
            .unwrap();
        app.refresh().unwrap();
        u3_edit(&mut app);
        app.event(KeyCode::Enter);
        assert_eq!(app.rows.len(), 1);
        assert!(matches!(&app.rows[0], Row::Target(id, _) if id.scope == Scope::Global));
        app.event(KeyCode::Esc);
        app.event(KeyCode::Esc);
        app.event(KeyCode::Esc);
        app.event(KeyCode::Esc);
        assert_eq!(app.screen, Screen::ScopeRoot);
        app.event(KeyCode::Down);
        app.event(KeyCode::Enter);
        assert_eq!(app.scope, Scope::Project(project.id));
        app.event(KeyCode::Enter);
        app.event(KeyCode::Down);
        app.event(KeyCode::Down);
        app.event(KeyCode::Enter);
        app.event(KeyCode::Enter);
        assert_eq!(app.rows.len(), 1);
        assert!(matches!(&app.rows[0], Row::Target(id, _) if id.scope == app.scope));
        assert!(app.checked.is_empty());
    }

    #[test]
    fn u3_new_preset_and_quit_do_not_write_without_confirmation() {
        let (_root, mut app, _preset, _skill) = u3_fixture();
        let before = app.engine.snapshot().unwrap();
        assert!(app.event(KeyCode::Char('q')));
        app.event(KeyCode::Enter);
        app.event(KeyCode::Down);
        app.event(KeyCode::Enter);
        for character in "jkTq".chars() {
            assert!(!app.event(KeyCode::Char(character)));
        }
        app.event(KeyCode::Enter);
        app.event(KeyCode::Char(' '));
        app.event(KeyCode::Enter);
        assert_eq!(app.screen, Screen::Confirm);
        assert!(app.event(KeyCode::Char('q')));
        assert_eq!(app.engine.snapshot().unwrap(), before);
        app.event(KeyCode::Enter);
        assert!(
            app.engine
                .snapshot()
                .unwrap()
                .presets
                .values()
                .any(|preset| preset.name == "jkTq")
        );
    }

    #[test]
    fn u3_execution_reports_survive_cleanup() {
        let (_root, mut app, preset, _skill) = u3_fixture();
        app.event(KeyCode::Enter);
        app.event(KeyCode::Enter);
        app.event(KeyCode::Enter);
        app.event(KeyCode::Char(' '));
        app.event(KeyCode::Enter);
        assert_eq!(app.screen, Screen::Confirm);
        app.event(KeyCode::Enter);
        assert_eq!(app.notice, "执行成功：1 个目标");
        assert!(app.home.join(".claude/skills/demo/SKILL.md").exists());
        assert!(
            app.engine.snapshot().unwrap().targets[&TargetId {
                agent: AgentId::ClaudeCode,
                scope: Scope::Global
            }]
                .presets[&preset]
                .enabled
        );
    }

    #[test]
    fn u3_partial_report_survives_cleanup() {
        let (_root, mut app, _preset, _skill) = u3_fixture();
        app.event(KeyCode::Enter);
        app.event(KeyCode::Enter);
        app.event(KeyCode::Enter);
        app.event(KeyCode::Char(' '));
        app.event(KeyCode::Down);
        app.event(KeyCode::Down);
        app.event(KeyCode::Char(' '));
        app.event(KeyCode::Enter);
        assert_eq!(app.screen, Screen::Confirm);
        std::fs::write(app.home.join(".pi"), "foreign blocker").unwrap();
        app.event(KeyCode::Enter);
        assert!(
            app.notice.starts_with("部分失败：成功 1，失败 1。Pi："),
            "{}",
            app.notice
        );
        assert_eq!(
            std::fs::read_to_string(app.home.join(".pi")).unwrap(),
            "foreign blocker"
        );
    }

    #[test]
    fn u3_error_report_survives_cleanup() {
        let (_root, mut app, preset, _skill) = u3_fixture();
        let target = u3_enable(&mut app, &preset);
        u3_edit(&mut app);
        app.event(KeyCode::Enter);
        app.event(KeyCode::Char(' '));
        app.event(KeyCode::Enter);
        assert_eq!(app.screen, Screen::Confirm);
        app.engine
            .execute_with_native_discovery(&Command::UpdatePreset {
                preset_id: preset,
                expected_revision: 1,
                members: BTreeSet::new(),
                sync_targets: vec![target],
                closed_members: ClosedMembersPolicy::Unspecified,
            })
            .unwrap();
        app.event(KeyCode::Enter);
        assert_eq!(
            app.notice,
            "执行失败：preset 'u3-preset' changed from expected revision 1 to 2。请重新编辑或预览"
        );
    }

    #[test]
    fn u3_preview_changed_keeps_fresh_confirmation() {
        let (_root, mut app, preset, _skill) = u3_fixture();
        app.event(KeyCode::Enter);
        app.event(KeyCode::Enter);
        app.event(KeyCode::Enter);
        app.event(KeyCode::Char(' '));
        app.event(KeyCode::Enter);
        let old = app.preview_plan.clone().unwrap();
        let external = app.home.join(".claude/skills/external");
        std::fs::create_dir_all(&external).unwrap();
        std::fs::write(
            external.join("SKILL.md"),
            "---\nname: External\ndescription: Physical plan change\n---\n",
        )
        .unwrap();
        let before = app.engine.snapshot().unwrap();
        app.event(KeyCode::Enter);
        assert_eq!(app.screen, Screen::Confirm);
        assert_eq!(app.notice, "状态已变化，请重新确认");
        assert_ne!(app.preview_plan.as_ref().unwrap(), &old);
        assert!(app.pending.is_some());
        assert_eq!(app.engine.snapshot().unwrap(), before);
        let fresh = app.preview_plan.clone().unwrap();
        app.event(KeyCode::Esc);
        assert_eq!(app.engine.snapshot().unwrap(), before);
        assert!(app.pending.is_none());
        app.event(KeyCode::Enter);
        app.event(KeyCode::Char(' '));
        app.event(KeyCode::Enter);
        assert_eq!(app.screen, Screen::Confirm);
        assert_eq!(app.preview_plan.as_ref().unwrap(), &fresh);
        assert_eq!(app.engine.snapshot().unwrap(), before);
        app.event(KeyCode::Enter);
        assert_eq!(app.notice, "执行成功：1 个目标");
        assert!(
            app.engine.snapshot().unwrap().targets[&TargetId {
                agent: AgentId::ClaudeCode,
                scope: Scope::Global,
            }]
                .presets[&preset]
                .enabled
        );
        assert!(external.join("SKILL.md").exists());
    }

    #[test]
    fn u3_text_entry_and_backspace_are_visible() {
        for screen in [Screen::CreateName, Screen::ProjectRoot] {
            let (_root, mut app, _preset, _skill) = u3_fixture();
            if screen == Screen::CreateName {
                app.event(KeyCode::Enter);
                app.event(KeyCode::Down);
                app.event(KeyCode::Enter);
            } else {
                app.event(KeyCode::Down);
                app.event(KeyCode::Enter);
            }
            assert_eq!(app.screen, screen);
            for character in "jkTq".chars() {
                assert!(!app.event(KeyCode::Char(character)));
            }
            assert_eq!(app.theme, MOCHA);
            assert!(u3_render(&app).contains("jkTq"));
            app.event(KeyCode::Backspace);
            let rendered = u3_render(&app);
            assert!(rendered.contains("jkT"));
            assert!(!rendered.contains("jkTq"));
        }
    }

    #[test]
    fn u3_back_preserves_edit_and_invalidates_confirmation() {
        let (_root, mut app, preset, skill) = u3_fixture();
        let target = u3_enable(&mut app, &preset);
        u3_edit(&mut app);
        app.event(KeyCode::Enter);
        assert_eq!(app.screen, Screen::Targets);
        app.event(KeyCode::Esc);
        assert_eq!(app.screen, Screen::EditMembers);
        assert_eq!(app.checked_skill_ids(), BTreeSet::from([skill.clone()]));
        app.event(KeyCode::Enter);
        app.event(KeyCode::Char(' '));
        app.event(KeyCode::Enter);
        assert_eq!(app.screen, Screen::Confirm);
        let before = app.engine.snapshot().unwrap();
        app.event(KeyCode::Esc);
        assert_eq!(app.screen, Screen::EditMembers);
        assert!(app.pending.is_none());
        assert!(app.preview_plan.is_none());
        assert_eq!(app.engine.snapshot().unwrap(), before);
        assert_eq!(app.checked_skill_ids(), BTreeSet::from([skill.clone()]));
        app.event(KeyCode::Enter);
        assert!(app.checked.is_empty());
        app.event(KeyCode::Char(' '));
        app.event(KeyCode::Enter);
        assert_eq!(app.screen, Screen::Confirm);
        match app.pending.as_ref().unwrap() {
            Command::UpdatePreset {
                sync_targets,
                members,
                ..
            } => {
                assert_eq!(sync_targets, &vec![target]);
                assert_eq!(members, &BTreeSet::from([skill.clone()]));
            }
            other => panic!("unexpected {other:?}"),
        }
        app.event(KeyCode::Enter);
        assert_eq!(
            app.engine.snapshot().unwrap().presets[&preset].members,
            BTreeSet::from([skill])
        );
    }

    #[test]
    fn u3_diagnostic_is_rendered() {
        let (_root, mut app, _preset, _skill) = u3_fixture();
        app.diagnostics = vec!["U3 literal scan diagnostic".into()];
        assert!(u3_render(&app).contains("U3 literal scan diagnostic"));
    }

    fn snapshot_with_project() -> Snapshot {
        let mut snapshot = Snapshot::default();
        let id = ProjectId::from("11111111-1111-1111-1111-111111111111".to_owned());
        snapshot.projects.insert(
            id.clone(),
            ProjectRecord {
                id,
                name: "PackCaptain".to_owned(),
                root: PathBuf::from("/tmp/packcaptain"),
            },
        );
        snapshot
    }

    #[test]
    fn root_rows_are_scope_level_only() {
        let snapshot = snapshot_with_project();
        let mut rows: Vec<String> = vec!["全局".to_owned()];
        rows.extend(snapshot.projects.values().map(|p| p.name.clone()));
        rows.push("扫描并认领项目".to_owned());
        assert_eq!(rows, vec!["全局", "PackCaptain", "扫描并认领项目"]);
        assert!(rows.iter().all(|row| !row.contains(':')));
    }

    #[test]
    fn scope_targets_filter_to_exact_scope() {
        let snapshot = snapshot_with_project();
        let project_id = snapshot.projects.keys().next().unwrap().clone();
        let other = ProjectId::from("22222222-2222-2222-2222-222222222222".to_owned());
        let mut targets = std::collections::BTreeMap::new();
        for scope in [
            Scope::Global,
            Scope::Project(project_id.clone()),
            Scope::Project(other),
        ] {
            targets.insert(
                TargetId {
                    agent: AgentId::ClaudeCode,
                    scope,
                },
                TargetState::default(),
            );
        }
        let global = scope_targets(&targets, &Scope::Global);
        assert_eq!(global.len(), 1);
        assert_eq!(global[0].0.scope, Scope::Global);
        let project = scope_targets(&targets, &Scope::Project(project_id));
        assert_eq!(project.len(), 1);
        assert_eq!(project[0].0.agent, AgentId::ClaudeCode);
    }

    #[test]
    fn command_for_rejects_cross_scope_targets() {
        let snapshot = snapshot_with_project();
        let project_id = snapshot.projects.keys().next().unwrap().clone();
        let draft = Draft::TogglePreset {
            preset_id: PresetId::new(),
            enabled: true,
        };
        let mixed = vec![
            TargetId {
                agent: AgentId::ClaudeCode,
                scope: Scope::Global,
            },
            TargetId {
                agent: AgentId::Codex,
                scope: Scope::Project(project_id.clone()),
            },
        ];
        assert!(command_for(&draft, &mixed, &Scope::Global).is_err());
        let only_global = vec![TargetId {
            agent: AgentId::ClaudeCode,
            scope: Scope::Global,
        }];
        match command_for(&draft, &only_global, &Scope::Global) {
            Ok(Command::SetPresetEnabled {
                targets, enabled, ..
            }) => {
                assert_eq!(targets, only_global);
                assert!(enabled);
            }
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn set_scope_resets_selection_state() {
        let mut app = App::new(
            Engine::new(
                PathBuf::from("/nonexistent-home"),
                PathBuf::from("/nonexistent-state"),
                NativeAdapters,
            ),
            PathBuf::from("/nonexistent-home"),
        );
        app.selected = 3;
        app.selected_key = Some("preset:old".into());
        app.checked.insert("target:stale".into());
        app.filter = "stale".into();
        app.pending = Some(Command::CreatePreset {
            preset_id: PresetId::new(),
            name: "x".into(),
            members: BTreeSet::new(),
        });
        let project_id = ProjectId::from("33333333-3333-3333-3333-333333333333".to_owned());
        app.set_scope(Scope::Project(project_id));
        assert_eq!(app.screen, Screen::PresetList);
        assert_eq!(app.selected, 0);
        assert_eq!(app.selected_key, None);
        assert!(app.checked.is_empty());
        assert!(app.filter.is_empty());
        assert!(app.pending.is_none());
        assert!(app.preview_plan.is_none());
    }

    #[test]
    fn text_keys_route_only_outside_input() {
        let mut app = App::new(
            Engine::new(
                PathBuf::from("/nonexistent-home"),
                PathBuf::from("/nonexistent-state"),
                NativeAdapters,
            ),
            PathBuf::from("/nonexistent-home"),
        );
        assert!(app.event(KeyCode::Char('q')));
        assert!(!app.event(KeyCode::Char('T')));
        assert_eq!(app.theme.name, "Catppuccin Latte");
        assert!(!app.event(KeyCode::Char('T')));
        assert_eq!(app.theme, MOCHA);
        assert!(app.event(KeyCode::Char('j')) == false);
        assert_eq!(app.selected, 0);
        app.screen = Screen::CreateName;
        for character in "jkTq".chars() {
            assert!(!app.event(KeyCode::Char(character)));
        }
        assert_eq!(app.input, "jkTq");
        assert_eq!(app.theme, MOCHA);
        app.screen = Screen::ScopeRoot;
        app.filtering = true;
        assert!(!app.event(KeyCode::Char('q')));
        assert_eq!(app.filter, "q");
    }

    #[test]
    fn theme_palettes_and_toggle() {
        assert_eq!(MOCHA.bg, ratatui::style::Color::Rgb(30, 30, 46));
        assert_eq!(MOCHA.name, "Catppuccin Mocha");
        let latte = tui_theme::next(MOCHA);
        assert_eq!(latte.name, "Catppuccin Latte");
        assert_eq!(latte.bg, ratatui::style::Color::Rgb(239, 241, 245));
        assert_eq!(tui_theme::next(latte), MOCHA);
    }

    #[test]
    fn hints_are_screen_specific() {
        assert!(hint_for(Screen::ScopeRoot, false).contains("Enter 进入"));
        assert!(!hint_for(Screen::ScopeRoot, false).contains("Space"));
        assert!(hint_for(Screen::Targets, false).contains("Space 勾选"));
        assert!(hint_for(Screen::SkillPick, false).contains("Enter 打开 Skill 操作"));
        assert!(hint_for(Screen::SkillPick, false).contains("/ 搜索"));
        assert!(!hint_for(Screen::SkillPick, false).contains("Space"));
        assert!(!hint_for(Screen::SkillPick, false).contains("e 开启"));
        assert!(!hint_for(Screen::PresetList, false).contains("n 新建"));
        assert!(!hint_for(Screen::PresetList, false).contains("/ 搜索"));
        assert!(hint_for(Screen::ScopeRoot, false).contains("Esc 退出"));
        assert!(hint_for(Screen::Confirm, false).contains("Esc 取消"));
        assert!(!hint_for(Screen::SkillPick, true).contains("Esc 返回"));
        assert!(hint_for(Screen::CreateName, false).contains("直接输入文本"));
        assert!(hint_for(Screen::Confirm, false).contains("PgUp/PgDn 滚动"));
        assert!(hint_for(Screen::CreateMembers, false).contains("Space 勾选"));
        assert!(hint_for(Screen::SkillPick, true).contains("输入搜索词"));
    }

    #[test]
    fn preview_uses_human_names_and_groups_by_target() {
        let mut snapshot = snapshot_with_project();
        let project_id = snapshot.projects.keys().next().unwrap().clone();
        let skill = SkillId::from("s1".to_owned());
        snapshot.skills.insert(
            skill.clone(),
            packcaptain::domain::SkillRecord {
                id: skill.clone(),
                name: "代码审查".to_owned(),
                source_path: PathBuf::from("/tmp/s1"),
                source_fingerprint: "fp".into(),
                availability: packcaptain::domain::SkillAvailability::Available,
            },
        );
        let mut preset_updates = std::collections::BTreeMap::new();
        preset_updates.insert(
            PresetId::new(),
            PresetDefinition {
                id: PresetId::new(),
                name: "基础包".to_owned(),
                revision: 2,
                members: BTreeSet::from([skill.clone()]),
            },
        );
        let plan = Plan {
            effects: vec![packcaptain::domain::Effect::SetSkillEnabled {
                target: TargetId {
                    agent: AgentId::ClaudeCode,
                    scope: Scope::Global,
                },
                skill: skill.clone(),
                enabled: true,
            }],
            preset_updates,
            target_updates: std::collections::BTreeMap::from([(
                TargetId {
                    agent: AgentId::Codex,
                    scope: Scope::Project(project_id),
                },
                Default::default(),
            )]),
        };
        let text = build_preview_text(&plan, &snapshot);
        assert!(text.contains("Claude Code"), "{text}");
        assert!(text.contains("PackCaptain"), "{text}");
        assert!(text.contains("代码审查"), "{text}");
        assert!(text.contains("基础包（共享定义）"), "{text}");
        assert!(
            !text.contains("11111111-1111-1111-1111-111111111111"),
            "{text}"
        );
    }

    #[test]
    fn esc_back_invalidates_plan_and_keeps_scope() {
        let mut app = App::new(
            Engine::new(
                PathBuf::from("/nonexistent-home"),
                PathBuf::from("/nonexistent-state"),
                NativeAdapters,
            ),
            PathBuf::from("/nonexistent-home"),
        );
        let project_id = ProjectId::from("44444444-4444-4444-4444-444444444444".to_owned());
        app.set_scope(Scope::Project(project_id));
        app.draft = Some(Draft::TogglePreset {
            preset_id: PresetId::new(),
            enabled: true,
        });
        app.screen = Screen::Confirm;
        app.preview_plan = Some(Plan::default());
        app.pending = Some(Command::CreatePreset {
            preset_id: PresetId::new(),
            name: "x".into(),
            members: BTreeSet::new(),
        });
        assert!(!app.event(KeyCode::Esc));
        assert!(app.preview_plan.is_none());
        assert!(app.pending.is_none());
        assert_eq!(app.screen, Screen::PresetActions);
        assert!(matches!(app.scope, Scope::Project(_)));
    }

    #[test]
    fn target_label_prefers_human_names() {
        let snapshot = snapshot_with_project();
        let project_id = snapshot.projects.keys().next().unwrap().clone();
        assert_eq!(
            target_label(
                &TargetId {
                    agent: AgentId::ClaudeCode,
                    scope: Scope::Global
                },
                &snapshot
            ),
            "Claude Code"
        );
        assert_eq!(
            target_label(
                &TargetId {
                    agent: AgentId::Codex,
                    scope: Scope::Project(project_id)
                },
                &snapshot
            ),
            "Codex · PackCaptain"
        );
        let orphan = ProjectId::from("55555555-5555-5555-5555-555555555555".to_owned());
        let label = target_label(
            &TargetId {
                agent: AgentId::Pi,
                scope: Scope::Project(orphan),
            },
            &snapshot,
        );
        assert!(label.contains("未知项目"), "{label}");
        assert!(label.contains("55555555"), "{label}");
    }

    fn u3_open_skill_actions(app: &mut App, skill: &SkillId) {
        app.event(KeyCode::Enter);
        app.event(KeyCode::Down);
        app.event(KeyCode::Down);
        app.event(KeyCode::Enter);
        assert_eq!(app.screen, Screen::SkillPick);
        app.event(KeyCode::Enter);
        assert_eq!(app.screen, Screen::SkillActions);
        assert_eq!(app.focus_skill.as_ref(), Some(skill));
    }

    #[test]
    fn old_shortcut_keys_are_gone() {
        let (_root, mut app, _preset, _skill) = u3_fixture();
        app.event(KeyCode::Enter);
        assert_eq!(app.screen, Screen::PresetList);
        app.event(KeyCode::Char('/'));
        assert!(!app.filtering);
        assert!(!u3_render(&app).contains("/ 搜索"));
        app.event(KeyCode::Char('n'));
        assert_eq!(app.screen, Screen::PresetList);
        assert!(app.input.is_empty());
        app.event(KeyCode::Char('m'));
        assert_eq!(app.screen, Screen::PresetList);
        assert!(app.draft.is_none());
        app.event(KeyCode::Down);
        app.event(KeyCode::Down);
        app.event(KeyCode::Enter);
        assert_eq!(app.screen, Screen::SkillPick);
        app.event(KeyCode::Char('/'));
        assert!(app.filtering);
        for character in "Demo ".chars() {
            app.event(KeyCode::Char(character));
        }
        assert_eq!(app.filter, "Demo ");
        app.event(KeyCode::Esc);
        assert!(!app.filtering);
        assert!(app.filter.is_empty());
        for screen in [Screen::SkillPick, Screen::SkillActions] {
            if screen == Screen::SkillActions {
                app.event(KeyCode::Enter);
            }
            let before = app.engine.snapshot().unwrap();
            for character in ['e', 'd', ' '] {
                assert!(!app.event(KeyCode::Char(character)));
                assert_eq!(app.screen, screen);
                assert!(app.draft.is_none());
                assert!(app.pending.is_none());
                assert!(app.checked.is_empty());
                assert_eq!(app.engine.snapshot().unwrap(), before);
            }
        }
    }

    #[test]
    fn skill_actions_menu_toggles_with_fresh_targets() {
        let (_root, mut app, _preset, skill) = u3_fixture();
        u3_open_skill_actions(&mut app, &skill);
        let rendered = u3_render(&app).replace(' ', "");
        assert!(rendered.contains("开启此Skill"), "{rendered}");
        assert!(rendered.contains("关闭此Skill"), "{rendered}");
        app.event(KeyCode::Enter);
        assert_eq!(app.screen, Screen::Targets);
        match app.draft.as_ref().unwrap() {
            Draft::ToggleSkill { skill_id, enabled } => {
                assert_eq!(skill_id, &skill);
                assert!(enabled);
            }
            other => panic!("unexpected {other:?}"),
        }
        app.event(KeyCode::Char(' '));
        app.event(KeyCode::Enter);
        assert_eq!(app.screen, Screen::Confirm);
        let before = app.engine.snapshot().unwrap();
        app.event(KeyCode::Esc);
        assert_eq!(app.screen, Screen::SkillActions);
        assert!(app.pending.is_none());
        assert!(app.preview_plan.is_none());
        assert_eq!(app.engine.snapshot().unwrap(), before);
        app.event(KeyCode::Enter);
        assert_eq!(app.screen, Screen::Targets);
        assert!(app.checked.is_empty(), "target selection must not leak");
        app.event(KeyCode::Char(' '));
        app.event(KeyCode::Enter);
        assert_eq!(app.screen, Screen::Confirm);
        app.event(KeyCode::Enter);
        assert_eq!(app.notice, "执行成功：1 个目标");
        assert_eq!(app.screen, Screen::PresetList);
        assert!(app.draft.is_none());
        app.event(KeyCode::Esc);
        u3_open_skill_actions(&mut app, &skill);
        app.event(KeyCode::Down);
        app.event(KeyCode::Enter);
        assert_eq!(app.screen, Screen::Targets);
        match app.draft.as_ref().unwrap() {
            Draft::ToggleSkill { skill_id, enabled } => {
                assert_eq!(skill_id, &skill);
                assert!(!enabled);
            }
            other => panic!("unexpected {other:?}"),
        }
        app.event(KeyCode::Char(' '));
        app.event(KeyCode::Enter);
        assert_eq!(app.screen, Screen::Confirm);
        let before_disable = app.engine.snapshot().unwrap();
        app.event(KeyCode::Esc);
        assert_eq!(app.screen, Screen::SkillActions);
        assert!(app.pending.is_none());
        assert_eq!(app.engine.snapshot().unwrap(), before_disable);
        app.event(KeyCode::Down);
        app.event(KeyCode::Enter);
        assert!(app.checked.is_empty());
        app.event(KeyCode::Char(' '));
        app.event(KeyCode::Enter);
        app.event(KeyCode::Enter);
        assert_eq!(app.notice, "执行成功：1 个目标");
        assert!(
            app.engine.snapshot().unwrap().targets[&TargetId {
                agent: AgentId::ClaudeCode,
                scope: Scope::Global,
            }]
                .manual_closed
                .contains(&skill)
        );
        app.event(KeyCode::Esc);
        assert_eq!(app.screen, Screen::ScopeRoot);
    }

    #[test]
    fn missing_skill_can_request_disable() {
        let (_root, mut app, preset, skill) = u3_fixture();
        let target = u3_enable(&mut app, &preset);
        u3_open_skill_actions(&mut app, &skill);
        std::fs::remove_file(app.home.join(".claude/skills/demo/SKILL.md")).unwrap();
        app.refresh().unwrap();
        assert!(u3_render(&app).replace(' ', "").contains("缺失"));
        app.event(KeyCode::Down);
        app.event(KeyCode::Enter);
        assert_eq!(app.screen, Screen::Targets);
        match app.draft.as_ref().unwrap() {
            Draft::ToggleSkill { skill_id, enabled } => {
                assert_eq!(skill_id, &skill);
                assert!(!enabled);
            }
            other => panic!("unexpected {other:?}"),
        }
        app.event(KeyCode::Char(' '));
        let before = app.engine.snapshot().unwrap();
        app.event(KeyCode::Enter);
        assert_eq!(app.screen, Screen::Targets);
        match app.pending.as_ref().unwrap() {
            Command::SetSkillEnabled {
                skill_id,
                targets,
                enabled,
            } => {
                assert_eq!(skill_id, &skill);
                assert_eq!(targets, &vec![target.clone()]);
                assert!(!enabled);
            }
            other => panic!("unexpected {other:?}"),
        }
        assert_eq!(app.screen, Screen::Targets);
        assert!(app.preview_plan.is_none());
        assert!(app.notice.starts_with("无法预览："), "{}", app.notice);
        assert!(app.notice.contains("is not available"), "{}", app.notice);
        assert_eq!(app.engine.snapshot().unwrap(), before);
        app.event(KeyCode::Esc);
        assert_eq!(app.screen, Screen::SkillActions);
        assert!(app.pending.is_none());
        assert_eq!(app.engine.snapshot().unwrap(), before);
    }

    #[test]
    fn confirm_arrow_keys_scroll_preview() {
        let (_root, mut app, _preset, _skill) = u3_fixture();
        app.event(KeyCode::Enter);
        app.event(KeyCode::Enter);
        app.event(KeyCode::Enter);
        app.event(KeyCode::Char(' '));
        app.event(KeyCode::Enter);
        assert_eq!(app.screen, Screen::Confirm);
        app.event(KeyCode::Up);
        assert_eq!(app.preview_scroll, 0);
        app.event(KeyCode::Down);
        app.event(KeyCode::Down);
        assert_eq!(app.preview_scroll, 2);
        app.event(KeyCode::Up);
        assert_eq!(app.preview_scroll, 1);
    }

    #[test]
    fn input_hint_is_visible_while_typing() {
        let (_root, mut app, _preset, _skill) = u3_fixture();
        app.event(KeyCode::Enter);
        app.event(KeyCode::Down);
        app.event(KeyCode::Enter);
        assert_eq!(app.screen, Screen::CreateName);
        for character in "jkTq".chars() {
            assert!(!app.event(KeyCode::Char(character)));
        }
        app.event(KeyCode::Char(' '));
        assert_eq!(app.input, "jkTq ");
        assert_eq!(app.theme, MOCHA);
        let rendered = u3_render(&app).replace(' ', "");
        assert!(rendered.contains("直接输入文本"), "{rendered}");
        assert!(rendered.contains("jkTq"), "{rendered}");
    }
}
