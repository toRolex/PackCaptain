use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::str::FromStr;

use clap::{Args, Parser, Subcommand};
use packcaptain::adapters::{DiscoveryReport, NativeAdapters};
use packcaptain::domain::{
    ClosedMembersPolicy, Command, PresetId, ProjectId, ProjectRecord, SkillId, Snapshot, TargetId,
};
use packcaptain::engine::{Engine, ExecutionReport, TargetOutcomeStatus};
use serde::Serialize;
use serde_json::{Value, json};

#[derive(Debug, Parser)]
#[command(
    name = "packcaptain",
    version,
    about = "管理多个 Agent 的 Skills 与 Presets"
)]
pub struct Cli {
    #[arg(long, global = true, value_name = "PATH")]
    agent_home: Option<PathBuf>,
    #[arg(long, global = true, value_name = "PATH")]
    state_dir: Option<PathBuf>,
    #[arg(long, global = true)]
    json: bool,
    #[command(subcommand)]
    command: Option<RootCommand>,
}

#[derive(Debug, Subcommand)]
enum RootCommand {
    Tui,
    Skills {
        #[command(subcommand)]
        command: SkillsCommand,
    },
    Projects {
        #[command(subcommand)]
        command: ProjectsCommand,
    },
    Targets {
        #[command(subcommand)]
        command: TargetsCommand,
    },
    Presets {
        #[command(subcommand)]
        command: PresetsCommand,
    },
}

#[derive(Debug, Subcommand)]
enum SkillsCommand {
    List {
        #[arg(long)]
        search: Option<String>,
    },
    Enable(SkillMutation),
    Disable(SkillMutation),
}

#[derive(Debug, Args)]
struct SkillMutation {
    id: String,
    #[arg(long = "target", required = true)]
    targets: Vec<String>,
    #[arg(long)]
    preview: bool,
}

#[derive(Debug, Subcommand)]
enum ProjectsCommand {
    List,
    Add {
        path: PathBuf,
        #[arg(long)]
        preview: bool,
    },
    Scan {
        #[arg(long)]
        root: PathBuf,
    },
    Discover {
        #[arg(long)]
        root: PathBuf,
    },
}

#[derive(Debug, Subcommand)]
enum TargetsCommand {
    List,
}

#[derive(Debug, Subcommand)]
enum PresetsCommand {
    List,
    Create {
        #[arg(long)]
        name: String,
        #[arg(long = "skill", required = true)]
        skills: Vec<String>,
        #[arg(long)]
        preview: bool,
    },
    Enable(PresetMutation),
    Disable(PresetMutation),
    Update {
        id: String,
        #[arg(long = "add-skill")]
        add: Vec<String>,
        #[arg(long = "remove-skill")]
        remove: Vec<String>,
        #[arg(long = "sync-target", required = true)]
        targets: Vec<String>,
        #[arg(long, value_parser = ["keep", "restore"])]
        closed_members: Option<String>,
        #[arg(long)]
        preview: bool,
    },
}

#[derive(Debug, Args)]
struct PresetMutation {
    id: String,
    #[arg(long = "target", required = true)]
    targets: Vec<String>,
    #[arg(long)]
    preview: bool,
}

fn write_json(value: &Value) {
    println!("{}", value);
}

fn failure(code: &str, message: impl Into<String>, impact: Value) -> Value {
    json!({"ok":false,"error":{"code":code,"message":message.into(),"impact":impact}})
}

fn success<T: Serialize>(data: T) -> Value {
    json!({"ok":true,"data":data})
}

trait CliId: Sized {
    fn parse(value: String) -> Self;
}
impl CliId for SkillId {
    fn parse(value: String) -> Self {
        Self::from(value)
    }
}
impl CliId for PresetId {
    fn parse(value: String) -> Self {
        Self::from(value.as_str())
    }
}
fn parse_id<T: CliId>(value: String) -> T {
    T::parse(value)
}

fn targets(values: Vec<String>) -> Result<Vec<TargetId>, String> {
    values
        .into_iter()
        .map(|value| TargetId::from_str(&value).map_err(|e| e.to_string()))
        .collect()
}

fn discovered(
    engine: &Engine<NativeAdapters>,
    home: &Path,
) -> Result<(Snapshot, DiscoveryReport), String> {
    let snapshot = engine.snapshot().map_err(|e| e.to_string())?;
    let report = NativeAdapters.discover(home, &snapshot);
    Ok((snapshot, report))
}

pub fn run() -> i32 {
    let raw: Vec<_> = std::env::args_os().collect();
    let wants_json = raw.iter().any(|arg| arg == "--json");
    let cli = match Cli::try_parse_from(raw) {
        Ok(cli) => cli,
        Err(error) => {
            if !error.use_stderr() {
                let exit_code = error.exit_code();
                let _ = error.print();
                return exit_code;
            }
            if wants_json {
                write_json(&failure("invalid-arguments", error.to_string(), json!([])));
                return 2;
            }
            let _ = error.print();
            return 2;
        }
    };
    let home = cli.agent_home.unwrap_or_else(dirs_default_home);
    let state_dir = cli.state_dir.unwrap_or_else(|| home.join(".packcaptain"));
    let mut engine = Engine::new(home.clone(), state_dir, NativeAdapters);
    let result = match cli.command {
        None | Some(RootCommand::Tui) => return crate::tui::run(engine, home),
        Some(RootCommand::Skills { command }) => skills(command, &mut engine, &home),
        Some(RootCommand::Projects { command }) => projects(command, &mut engine, &home),
        Some(RootCommand::Targets { command }) => targets_list(command, &mut engine, &home),
        Some(RootCommand::Presets { command }) => presets(command, &mut engine, &home),
    };
    match result {
        Ok(value) => {
            write_json(&value);
            0
        }
        Err((code, message, impact)) => {
            write_json(&failure(&code, message, impact));
            1
        }
    }
}

fn dirs_default_home() -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
}

type CliResult = Result<Value, (String, String, Value)>;
fn err(code: impl Into<String>, message: impl Into<String>) -> (String, String, Value) {
    (code.into(), message.into(), json!([]))
}

fn skills(command: SkillsCommand, engine: &mut Engine<NativeAdapters>, home: &Path) -> CliResult {
    let (snapshot, discovery) = discovered(engine, home).map_err(|e| err("discovery-failed", e))?;
    let merged = discovery.merge_into(&snapshot);
    match command {
        SkillsCommand::List { search } => {
            let query = search.unwrap_or_default().to_lowercase();
            let rows: Vec<_> = merged
                .skills
                .values()
                .filter(|skill| {
                    skill.name.to_lowercase().contains(&query) || skill.id.as_str().contains(&query)
                })
                .collect();
            Ok(success(rows))
        }
        SkillsCommand::Enable(args) => {
            let enabled = true;
            let command = Command::SetSkillEnabled {
                skill_id: parse_id(args.id),
                targets: targets(args.targets).map_err(|e| err("invalid-target", e))?,
                enabled,
            };
            run_mutation(engine, &command, &discovery, args.preview)
        }
        SkillsCommand::Disable(args) => {
            let command = Command::SetSkillEnabled {
                skill_id: parse_id(args.id),
                targets: targets(args.targets).map_err(|e| err("invalid-target", e))?,
                enabled: false,
            };
            run_mutation(engine, &command, &discovery, args.preview)
        }
    }
}

fn projects(
    command: ProjectsCommand,
    engine: &mut Engine<NativeAdapters>,
    _home: &Path,
) -> CliResult {
    match command {
        ProjectsCommand::List => {
            let snapshot = engine
                .snapshot()
                .map_err(|e| err("state-read-failed", e.to_string()))?;
            Ok(success(snapshot.projects.values().collect::<Vec<_>>()))
        }
        ProjectsCommand::Add { path, preview } => {
            if !path.is_absolute() {
                return Err(err("absolute-path-required", "项目路径必须是绝对路径"));
            }
            let canonical = std::fs::canonicalize(&path)
                .map_err(|e| err("project-path-invalid", e.to_string()))?;
            if !canonical.is_dir() {
                return Err(err("project-path-invalid", "项目路径不是目录"));
            }
            let name = canonical
                .file_name()
                .and_then(|x| x.to_str())
                .unwrap_or("project")
                .to_owned();
            let project = ProjectRecord {
                id: ProjectId::new(),
                name,
                root: canonical,
            };
            if preview {
                return Ok(success(json!({"preview":true,"project":project})));
            }
            let claimed = project.clone();
            engine
                .claim_project(project)
                .map_err(|e| err("project-claim-failed", e.to_string()))?;
            Ok(success(claimed))
        }
        ProjectsCommand::Scan { root } | ProjectsCommand::Discover { root } => {
            let candidates = NativeAdapters::scan_candidates(&root)
                .map_err(|e| err("scan-failed", e.to_string()))?;
            let rows: Vec<_> = candidates
                .into_iter()
                .map(|candidate| json!({"name":candidate.name,"path":candidate.path}))
                .collect();
            Ok(success(rows))
        }
    }
}

fn targets_list(_: TargetsCommand, engine: &mut Engine<NativeAdapters>, home: &Path) -> CliResult {
    let (snapshot, discovery) = discovered(engine, home).map_err(|e| err("discovery-failed", e))?;
    let merged = discovery.merge_into(&snapshot);
    let rows: Vec<_> = merged
        .targets
        .iter()
        .map(|(id, state)| {
            let mut row = json!({"id":id.to_string(),"state":state});
            if let Some(error) = discovery.target_errors.get(id) {
                row["observation"] = json!("unknown");
                row["diagnostic"] = json!(error.to_string());
            }
            row
        })
        .collect();
    Ok(success(rows))
}

fn presets(command: PresetsCommand, engine: &mut Engine<NativeAdapters>, home: &Path) -> CliResult {
    let (snapshot, discovery) = discovered(engine, home).map_err(|e| err("discovery-failed", e))?;
    let merged = discovery.merge_into(&snapshot);
    match command {
        PresetsCommand::List => Ok(success(merged.presets.values().collect::<Vec<_>>())),
        PresetsCommand::Create {
            name,
            skills,
            preview,
        } => {
            let members: BTreeSet<SkillId> = skills.into_iter().map(parse_id).collect();
            let command = Command::CreatePreset {
                preset_id: PresetId::new(),
                name,
                members,
            };
            run_mutation(engine, &command, &discovery, preview)
        }
        PresetsCommand::Enable(args) => {
            let command = Command::SetPresetEnabled {
                preset_id: parse_id(args.id),
                targets: targets(args.targets).map_err(|e| err("invalid-target", e))?,
                enabled: true,
            };
            run_mutation(engine, &command, &discovery, args.preview)
        }
        PresetsCommand::Disable(args) => {
            let command = Command::SetPresetEnabled {
                preset_id: parse_id(args.id),
                targets: targets(args.targets).map_err(|e| err("invalid-target", e))?,
                enabled: false,
            };
            run_mutation(engine, &command, &discovery, args.preview)
        }
        PresetsCommand::Update {
            id,
            add,
            remove,
            targets: sync_targets,
            closed_members,
            preview,
        } => {
            let preset_id: PresetId = parse_id(id);
            let current = merged
                .presets
                .get(&preset_id)
                .ok_or_else(|| err("preset-not-found", "Preset 不存在"))?;
            let mut members = current.members.clone();
            for skill in add {
                members.insert(parse_id(skill));
            }
            for skill in remove {
                members.remove(&parse_id(skill));
            }
            let policy = match closed_members.as_deref() {
                Some("keep") => ClosedMembersPolicy::Keep,
                Some("restore") => ClosedMembersPolicy::Restore,
                _ => ClosedMembersPolicy::Unspecified,
            };
            let command = Command::UpdatePreset {
                preset_id,
                expected_revision: current.revision,
                members,
                sync_targets: targets(sync_targets).map_err(|e| err("invalid-target", e))?,
                closed_members: policy,
            };
            run_mutation(engine, &command, &discovery, preview)
        }
    }
}

fn run_mutation(
    engine: &mut Engine<NativeAdapters>,
    command: &Command,
    discovery: &DiscoveryReport,
    preview: bool,
) -> CliResult {
    if preview {
        let plan = engine
            .preview_with_discovery(command, discovery)
            .map_err(|e| {
                let (code, message, impact) = engine_error(e);
                (code, message, impact)
            })?;
        return Ok(success(json!({"preview":true,"plan":plan})));
    }
    let report = engine
        .execute_with_native_discovery(command)
        .map_err(engine_error)?;
    let ok = report
        .targets
        .iter()
        .all(|target| target.status == TargetOutcomeStatus::Applied);
    let data = execution_json(report);
    if ok {
        Ok(success(data))
    } else {
        Err((
            "partial-failure".to_owned(),
            "一个或多个目标执行失败".to_owned(),
            data,
        ))
    }
}

fn engine_error(error: packcaptain::engine::EngineError) -> (String, String, Value) {
    match error {
        packcaptain::engine::EngineError::Conflict(conflict) => (
            conflict.code,
            conflict.message,
            json!({"targets":conflict.affected_targets,"skills":conflict.affected_skills}),
        ),
        other => ("operation-failed".to_owned(), other.to_string(), json!([])),
    }
}

fn execution_json(report: ExecutionReport) -> Value {
    serde_json::to_value(report).unwrap_or(Value::Null)
}
