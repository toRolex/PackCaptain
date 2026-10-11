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
        ClosedMembersPolicy, Command, Plan, PresetId, ProjectId, ProjectRecord, SkillId, Snapshot,
        TargetId,
    },
    engine::Engine,
};
use ratatui::{
    Terminal,
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, ListState, Paragraph, Wrap},
};

#[derive(Clone, Copy, PartialEq)]
enum Screen {
    Home,
    CreateName,
    CreateSkills,
    SkillInventory,
    ProjectRoot,
    ProjectCandidates,
    EditMembers,
    EditTargets,
    EditPolicy,
    Targets,
    Confirm,
}

struct App {
    engine: Engine<NativeAdapters>,
    home: PathBuf,
    snapshot: Snapshot,
    discovery: DiscoveryReport,
    screen: Screen,
    rows: Vec<Row>,
    selected: usize,
    selected_key: Option<String>,
    checked: BTreeSet<String>,
    filter: String,
    filtering: bool,
    input: String,
    notice: String,
    pending: Option<Command>,
    preview: String,
    preview_plan: Option<Plan>,
    preview_scroll: u16,
    project_root: PathBuf,
    candidates: Vec<ProjectCandidate>,
    editing_members: BTreeSet<SkillId>,
}

enum Row {
    Preset(PresetId, String),
    Skill(SkillId, String),
    Target(TargetId, String),
    Candidate(PathBuf, String),
    Action(&'static str),
}

impl Row {
    fn key(&self) -> String {
        match self {
            Self::Preset(id, _) => format!("preset:{}", id.as_str()),
            Self::Skill(id, _) => format!("skill:{}", id.as_str()),
            Self::Target(id, _) => format!("target:{id}"),
            Self::Candidate(path, _) => format!("candidate:{}", path.display()),
            Self::Action(label) => format!("action:{label}"),
        }
    }
}

impl App {
    fn refresh(&mut self) -> Result<(), String> {
        self.snapshot = self.engine.snapshot().map_err(|e| e.to_string())?;
        self.discovery = NativeAdapters.discover(&self.home, &self.snapshot);
        if !self.discovery.diagnostics.is_empty() {
            self.notice = format!(
                "{} 条扫描诊断：{}",
                self.discovery.diagnostics.len(),
                self.discovery.diagnostics[0]
            );
        }
        self.rows.clear();
        match self.screen {
            Screen::Home => {
                self.rows.extend(self.snapshot.presets.values().map(|p| {
                    Row::Preset(
                        p.id.clone(),
                        format!(
                            "Preset  {}  r{}  [{} members]",
                            p.name,
                            p.revision,
                            p.members.len()
                        ),
                    )
                }));
                self.rows.push(Row::Action("新建 Preset"));
                self.rows.push(Row::Action("Skills inventory"));
                self.rows.push(Row::Action("扫描并认领项目"));
            }
            Screen::CreateSkills | Screen::SkillInventory | Screen::EditMembers => {
                let query = self.filter.to_lowercase();
                let merged = self.discovery.merge_into(&self.snapshot);
                self.rows.extend(
                    merged
                        .skills
                        .values()
                        .filter(|skill| {
                            query.is_empty()
                                || skill.name.to_lowercase().contains(&query)
                                || skill.id.as_str().contains(&query)
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
                                    skill.availability.as_label(),
                                    skill.source_path.display()
                                ),
                            )
                        }),
                );
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
            Screen::Targets | Screen::EditTargets => {
                let merged = self.discovery.merge_into(&self.snapshot);
                let preset_id = match &self.pending {
                    Some(Command::UpdatePreset { preset_id, .. }) => Some(preset_id),
                    _ => None,
                };
                self.rows.extend(
                    merged
                        .targets
                        .iter()
                        .filter(|(target, state)| {
                            if self.screen == Screen::EditTargets {
                                let _ = target;
                                preset_id
                                    .and_then(|id| state.presets.get(id))
                                    .is_some_and(|applied| applied.enabled)
                            } else {
                                true
                            }
                        })
                        .map(|(target, state)| {
                            Row::Target(
                                target.clone(),
                                format!(
                                    "{}  enabled:{} closed:{} presets:{}",
                                    target,
                                    state.enabled_skills.len(),
                                    state.manual_closed.len(),
                                    state
                                        .presets
                                        .values()
                                        .filter(|preset| preset.enabled)
                                        .count()
                                ),
                            )
                        }),
                );
            }
            _ => {}
        }
        if !self.rows.is_empty() {
            if let Some(key) = &self.selected_key {
                self.selected = self
                    .rows
                    .iter()
                    .position(|row| row.key() == *key)
                    .unwrap_or(self.selected.min(self.rows.len() - 1));
            } else {
                self.selected = self.selected.min(self.rows.len() - 1);
            }
            self.selected_key = Some(self.rows[self.selected].key());
        } else {
            self.selected = 0;
            self.selected_key = None;
        }
        Ok(())
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
        if matches!(key, KeyCode::Esc) {
            match self.screen {
                Screen::Home => return true,
                Screen::Confirm => {
                    self.pending = None;
                    self.screen = Screen::Home;
                    self.notice = "已取消，未写入".into();
                }
                _ => {
                    self.screen = Screen::Home;
                    self.pending = None;
                    self.notice = "已取消，未写入".into();
                }
            }
            let _ = self.refresh();
            return false;
        }
        match self.screen {
            Screen::Home => match key {
                KeyCode::Up => self.selected = self.selected.saturating_sub(1),
                KeyCode::Down => {
                    self.selected = (self.selected + 1).min(self.rows.len().saturating_sub(1))
                }
                KeyCode::Char('q') => return true,
                KeyCode::Char('u') => {
                    if let Some(Row::Preset(id, _)) = self.rows.get(self.selected)
                        && let Some(definition) = self.snapshot.presets.get(id)
                    {
                        self.editing_members = definition.members.clone();
                        self.pending = Some(Command::UpdatePreset {
                            preset_id: id.clone(),
                            expected_revision: definition.revision,
                            members: definition.members.clone(),
                            sync_targets: Vec::new(),
                            closed_members: ClosedMembersPolicy::Unspecified,
                        });
                        self.checked = definition
                            .members
                            .iter()
                            .map(|id| format!("skill:{}", id.as_str()))
                            .collect();
                        self.screen = Screen::EditMembers;
                        self.selected = 0;
                        let _ = self.refresh();
                        self.notice = "编辑成员：Space 切换，Enter 选择已启用目标".into();
                    }
                }
                KeyCode::Char('e') | KeyCode::Char('d') => {
                    if let Some(Row::Preset(id, _)) = self.rows.get(self.selected) {
                        self.pending = Some(Command::SetPresetEnabled {
                            preset_id: id.clone(),
                            targets: Vec::new(),
                            enabled: key == KeyCode::Char('e'),
                        });
                        self.screen = Screen::Targets;
                        self.checked.clear();
                        let _ = self.refresh();
                    }
                }
                KeyCode::Enter => match self.rows.get(self.selected) {
                    Some(Row::Action("新建 Preset")) => {
                        self.screen = Screen::CreateName;
                        self.input.clear();
                        self.notice = "输入新 Preset 名称并按 Enter".into();
                    }
                    Some(Row::Action("Skills inventory")) => {
                        self.screen = Screen::SkillInventory;
                        self.selected = 0;
                        self.checked.clear();
                        let _ = self.refresh();
                    }
                    Some(Row::Action("扫描并认领项目")) => {
                        self.screen = Screen::ProjectRoot;
                        self.input.clear();
                        self.notice = "输入项目根目录，Enter 扫描候选".into();
                    }
                    Some(Row::Preset(id, _)) => {
                        let command = Command::SetPresetEnabled {
                            preset_id: id.clone(),
                            targets: Vec::new(),
                            enabled: true,
                        };
                        self.pending = Some(command);
                        self.screen = Screen::Targets;
                        self.checked.clear();
                        let _ = self.refresh();
                        self.notice = "选择目标，Space 勾选，Enter 预览".into();
                    }
                    _ => {}
                },
                _ => {}
            },
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
                                self.selected = 0;
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
            Screen::ProjectCandidates => match key {
                KeyCode::Up => self.selected = self.selected.saturating_sub(1),
                KeyCode::Down => {
                    self.selected = (self.selected + 1).min(self.rows.len().saturating_sub(1))
                }
                KeyCode::Char(' ') => {
                    if let Some(row) = self.rows.get(self.selected) {
                        let key = row.key();
                        if self.checked.contains(&key) {
                            self.checked.remove(&key);
                        } else {
                            self.checked.insert(key);
                        }
                    }
                }
                KeyCode::Enter => {
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
                        self.screen = Screen::Home;
                        self.notice = if errors.is_empty() {
                            "项目目录已认领".into()
                        } else {
                            format!("部分认领失败：{}", errors.join("；"))
                        };
                        let _ = self.refresh();
                    }
                }
                _ => {}
            },
            Screen::CreateName => match key {
                KeyCode::Char(c) if !c.is_control() => self.input.push(c),
                KeyCode::Backspace => {
                    self.input.pop();
                }
                KeyCode::Enter if !self.input.trim().is_empty() => {
                    self.screen = Screen::CreateSkills;
                    self.selected = 0;
                    self.checked.clear();
                    let _ = self.refresh();
                    self.notice = "选择成员，Space 勾选，Enter 预览".into();
                }
                _ => {}
            },
            Screen::CreateSkills => match key {
                KeyCode::Char('/') => {
                    self.filtering = true;
                    self.filter.clear();
                }
                KeyCode::Up => self.selected = self.selected.saturating_sub(1),
                KeyCode::Down => {
                    self.selected = (self.selected + 1).min(self.rows.len().saturating_sub(1))
                }
                KeyCode::Char(' ') => {
                    if let Some(row) = self.rows.get(self.selected) {
                        let key = row.key();
                        if self.checked.contains(&key) {
                            self.checked.remove(&key);
                        } else {
                            self.checked.insert(key);
                        }
                    }
                }
                KeyCode::Enter => {
                    let members = self
                        .checked
                        .iter()
                        .filter_map(|key| {
                            key.strip_prefix("skill:")
                                .map(|id| SkillId::from(id.to_owned()))
                        })
                        .collect::<BTreeSet<_>>();
                    self.pending = Some(Command::CreatePreset {
                        preset_id: PresetId::new(),
                        name: self.input.trim().to_owned(),
                        members,
                    });
                    self.preview_pending();
                }
                _ => {}
            },
            Screen::EditMembers => match key {
                KeyCode::Char('/') => {
                    self.filtering = true;
                    self.filter.clear();
                }
                KeyCode::Up => self.selected = self.selected.saturating_sub(1),
                KeyCode::Down => {
                    self.selected = (self.selected + 1).min(self.rows.len().saturating_sub(1))
                }
                KeyCode::Char(' ') => {
                    if let Some(row) = self.rows.get(self.selected) {
                        let key = row.key();
                        if self.checked.contains(&key) {
                            self.checked.remove(&key);
                        } else {
                            self.checked.insert(key);
                        }
                    }
                }
                KeyCode::Enter => {
                    self.editing_members = self
                        .checked
                        .iter()
                        .filter_map(|key| {
                            key.strip_prefix("skill:")
                                .map(|id| SkillId::from(id.to_owned()))
                        })
                        .collect();
                    if let Some(Command::UpdatePreset { members, .. }) = self.pending.as_mut() {
                        *members = self.editing_members.clone();
                    }
                    self.screen = Screen::EditTargets;
                    self.checked.clear();
                    self.selected = 0;
                    let _ = self.refresh();
                    self.notice =
                        "选择已开启此 Preset 的 targets，Space 勾选，Enter 预览同步".into();
                }
                _ => {}
            },
            Screen::EditPolicy => match key {
                KeyCode::Up => self.selected = self.selected.saturating_sub(1),
                KeyCode::Down => {
                    self.selected = (self.selected + 1).min(self.rows.len().saturating_sub(1))
                }
                KeyCode::Enter => {
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
                _ => {}
            },
            Screen::SkillInventory => match key {
                KeyCode::Char('/') => {
                    self.filtering = true;
                    self.filter.clear();
                }
                KeyCode::Up => self.selected = self.selected.saturating_sub(1),
                KeyCode::Down => {
                    self.selected = (self.selected + 1).min(self.rows.len().saturating_sub(1))
                }
                KeyCode::Char('e') | KeyCode::Char('d') => {
                    if let Some(Row::Skill(id, _)) = self.rows.get(self.selected) {
                        self.pending = Some(Command::SetSkillEnabled {
                            skill_id: id.clone(),
                            targets: Vec::new(),
                            enabled: key == KeyCode::Char('e'),
                        });
                        self.screen = Screen::Targets;
                        self.checked.clear();
                        let _ = self.refresh();
                        self.notice = "选择明确目标，Space 勾选，Enter 预览".into();
                    }
                }
                _ => {}
            },
            Screen::Targets | Screen::EditTargets => match key {
                KeyCode::Up => self.selected = self.selected.saturating_sub(1),
                KeyCode::Down => {
                    self.selected = (self.selected + 1).min(self.rows.len().saturating_sub(1))
                }
                KeyCode::Char(' ') => {
                    if let Some(row) = self.rows.get(self.selected) {
                        let key = row.key();
                        if self.checked.contains(&key) {
                            self.checked.remove(&key);
                        } else {
                            self.checked.insert(key);
                        }
                    }
                }
                KeyCode::Enter => {
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
                        self.notice = "至少选择一个目标".into();
                    } else if let Some(command) = self.pending.take() {
                        self.pending = Some(match command {
                            Command::SetPresetEnabled {
                                preset_id, enabled, ..
                            } => Command::SetPresetEnabled {
                                preset_id,
                                targets: selected,
                                enabled,
                            },
                            Command::SetSkillEnabled {
                                skill_id, enabled, ..
                            } => Command::SetSkillEnabled {
                                skill_id,
                                targets: selected,
                                enabled,
                            },
                            Command::UpdatePreset {
                                preset_id,
                                expected_revision,
                                members,
                                closed_members,
                                ..
                            } => Command::UpdatePreset {
                                preset_id,
                                expected_revision,
                                members,
                                sync_targets: selected,
                                closed_members,
                            },
                            other => other,
                        });
                        self.preview_pending();
                    }
                }
                _ => {}
            },
            Screen::Confirm => match key {
                KeyCode::Up => self.preview_scroll = self.preview_scroll.saturating_sub(1),
                KeyCode::Down => self.preview_scroll = self.preview_scroll.saturating_add(1),
                KeyCode::PageUp => self.preview_scroll = self.preview_scroll.saturating_sub(8),
                KeyCode::PageDown => self.preview_scroll = self.preview_scroll.saturating_add(8),
                KeyCode::Enter => {
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
                                    .filter(|target| {
                                        target.status
                                            == packcaptain::engine::TargetOutcomeStatus::Applied
                                    })
                                    .count();
                                let failed: Vec<_> = report
                                    .targets
                                    .iter()
                                    .filter(|target| {
                                        target.status
                                            == packcaptain::engine::TargetOutcomeStatus::Failed
                                    })
                                    .map(|target| {
                                        format!(
                                            "{}：{}",
                                            target.target,
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
                            Err(packcaptain::engine::EngineError::PreviewChanged(plan)) => {
                                self.show_preview(*plan);
                                self.preview = format!("状态已变化，请重新确认\n{}", self.preview);
                                return false;
                            }
                            Err(packcaptain::engine::EngineError::Conflict(conflict))
                                if conflict.code == "closed-members-policy-required" =>
                            {
                                self.preview_pending();
                                return false;
                            }
                            Err(e @ packcaptain::engine::EngineError::Discovery { .. }) => {
                                self.notice = format!("执行失败：{e}。修正配置后重新预览");
                                self.preview = self.notice.clone();
                                return false;
                            }
                            Err(e) => self.notice = format!("执行失败：{e}。请重新编辑或预览"),
                        };
                    }
                    self.pending = None;
                    self.preview_plan = None;
                    self.screen = Screen::Home;
                    let notice = self.notice.clone();
                    let _ = self.refresh();
                    self.notice = notice;
                }
                _ => {}
            },
        }
        if let Some(row) = self.rows.get(self.selected) {
            self.selected_key = Some(row.key());
        }
        false
    }

    fn preview_pending(&mut self) {
        if let Some(command) = &self.pending {
            match self.engine.preview_with_native_discovery(command) {
                Ok(plan) => self.show_preview(plan),
                Err(packcaptain::engine::EngineError::Conflict(conflict))
                    if conflict.code == "closed-members-policy-required" =>
                {
                    self.screen = Screen::EditPolicy;
                    self.selected = 0;
                    self.selected_key = None;
                    let _ = self.refresh();
                    let targets = conflict
                        .affected_targets
                        .iter()
                        .map(ToString::to_string)
                        .collect::<Vec<_>>()
                        .join(", ");
                    let skills = conflict
                        .affected_skills
                        .iter()
                        .map(|skill| skill.as_str())
                        .collect::<Vec<_>>()
                        .join(", ");
                    self.notice = format!(
                        "{}。受影响targets：{}；skills：{}。默认选择 keep，可改选 restore。",
                        conflict.message, targets, skills
                    );
                }
                Err(e) => self.notice = format!("无法预览：{e}"),
            }
        }
    }

    fn show_preview(&mut self, plan: Plan) {
        let mut details = vec![format!(
            "影响预览：{} 项变更，{} 个目标",
            plan.effects.len(),
            plan.target_updates.len()
        )];
        for effect in &plan.effects {
            match effect {
                packcaptain::domain::Effect::SetSkillEnabled {
                    target,
                    skill,
                    enabled,
                } => details.push(format!(
                    "{}：{} skill {}",
                    target,
                    if *enabled { "开启" } else { "关闭" },
                    skill.as_str()
                )),
            }
        }
        for definition in plan.preset_updates.values() {
            details.push(format!(
                "Preset {} r{}，成员：{}",
                definition.name,
                definition.revision,
                definition
                    .members
                    .iter()
                    .map(|id| id.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }
        for target in plan.target_updates.keys() {
            details.push(format!("更新目标状态：{target}"));
        }
        details.push("Enter 确认写入，Esc 取消且不写入".to_owned());
        self.preview = details.join("\n");
        self.preview_scroll = 0;
        self.screen = Screen::Confirm;
        self.notice = self.preview.clone();
        self.preview_plan = Some(plan);
    }
}

trait AvailabilityLabel {
    fn as_label(&self) -> &'static str;
}
impl AvailabilityLabel for packcaptain::domain::SkillAvailability {
    fn as_label(&self) -> &'static str {
        match self {
            Self::Available => "可用",
            Self::Missing => "缺失",
            Self::Unsupported => "不支持",
            Self::Unreadable => "不可读",
        }
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
    let mut app = App {
        engine,
        home,
        snapshot: Snapshot::default(),
        discovery: DiscoveryReport::default(),
        screen: Screen::Home,
        rows: Vec::new(),
        selected: 0,
        selected_key: None,
        checked: BTreeSet::new(),
        filter: String::new(),
        filtering: false,
        input: String::new(),
        notice: "方向键移动，Enter 选择，Space 勾选，Esc 返回，q 退出".into(),
        pending: None,
        preview: String::new(),
        preview_plan: None,
        preview_scroll: 0,
        project_root: PathBuf::new(),
        candidates: Vec::new(),
        editing_members: BTreeSet::new(),
    };
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

fn draw(frame: &mut ratatui::Frame<'_>, app: &App) {
    let areas = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Min(5),
            Constraint::Length(3),
        ])
        .split(frame.area());
    let title = Paragraph::new("PackCaptain  |  Preset 管理")
        .block(Block::default().borders(Borders::ALL).title("工作区"));
    frame.render_widget(title, areas[0]);
    let items: Vec<ListItem<'_>> = app
        .rows
        .iter()
        .enumerate()
        .map(|(i, row)| {
            let (checked, label) = match row {
                Row::Preset(_, s) => (false, s.as_str()),
                Row::Skill(_, s) => (app.checked.contains(&row.key()), s.as_str()),
                Row::Target(_, s) => (app.checked.contains(&row.key()), s.as_str()),
                Row::Candidate(_, s) => (app.checked.contains(&row.key()), s.as_str()),
                Row::Action(s) => (app.checked.contains(&row.key()), *s),
            };
            let cursor = if i == app.selected { "> " } else { "  " };
            let check = if checked { "[x] " } else { "[ ] " };
            ListItem::new(Line::from(vec![
                Span::raw(cursor),
                Span::raw(check),
                Span::raw(label),
            ]))
        })
        .collect();
    let list = List::new(items)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(match app.screen {
                    Screen::Home => "Presets",
                    Screen::CreateName => "新建 Preset",
                    Screen::CreateSkills => "选择技能成员（/ 搜索，Space 勾选）",
                    Screen::SkillInventory => "Skills 状态（/ 搜索，e 开启，d 关闭）",
                    Screen::ProjectRoot => "输入项目扫描根目录",
                    Screen::ProjectCandidates => "选择认领项目（Space 勾选）",
                    Screen::EditMembers => "编辑 Preset 成员",
                    Screen::EditTargets => "选择已开启 Preset 的目标",
                    Screen::EditPolicy => "手动关闭冲突策略（默认 keep）",
                    Screen::Targets => "选择目标",
                    Screen::Confirm => "执行预览",
                }),
        )
        .highlight_style(
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        );
    if app.screen == Screen::Confirm {
        frame.render_widget(
            Paragraph::new(app.preview.as_str())
                .wrap(Wrap { trim: false })
                .scroll((app.preview_scroll, 0))
                .block(
                    Block::default()
                        .borders(Borders::ALL)
                        .title("逐项影响（方向键/PageUp/PageDown 滚动）"),
                ),
            areas[1],
        );
    } else {
        let mut list_state = ListState::default();
        list_state.select(Some(app.selected));
        frame.render_stateful_widget(list, areas[1], &mut list_state);
    }
    let footer = if app.filtering {
        format!("搜索过滤：{}（Enter 确认，Esc 清空）", app.filter)
    } else if app.screen == Screen::CreateName {
        format!("名称：{}", app.input)
    } else if app.screen == Screen::ProjectRoot {
        format!("项目根目录：{}", app.input)
    } else if app.screen == Screen::Confirm {
        app.preview.clone()
    } else {
        app.notice.clone()
    };
    frame.render_widget(
        Paragraph::new(footer).block(Block::default().borders(Borders::ALL).title("操作与状态")),
        areas[2],
    );
}
