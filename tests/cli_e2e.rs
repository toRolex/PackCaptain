use std::{
    fs,
    path::Path,
    process::{Command, Output},
};

use serde_json::Value;
use tempfile::TempDir;

fn fixture() -> TempDir {
    let base =
        Path::new("/private/tmp").join(format!("packcaptain-frontend-cli-{}", std::process::id()));
    fs::create_dir_all(&base).unwrap();
    TempDir::new_in(base).unwrap()
}

fn run(home: &Path, state: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_packcaptain"))
        .args([
            "--agent-home",
            home.to_str().unwrap(),
            "--state-dir",
            state.to_str().unwrap(),
            "--json",
        ])
        .args(args)
        .output()
        .unwrap()
}

fn data(output: &Output) -> Value {
    assert!(
        output.status.success(),
        "stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let value: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(value["ok"], true);
    value["data"].clone()
}

#[test]
fn cli_creates_preset_and_applies_it_to_explicit_project_target() {
    let fixture = fixture();
    let home = fixture.path().join("home");
    let state = fixture.path().join("state");
    let project = fixture.path().join("project");
    let source = home.join(".claude/skills/fixture-skill");
    fs::create_dir_all(&source).unwrap();
    fs::create_dir_all(&project).unwrap();
    fs::write(
        source.join("SKILL.md"),
        "---\nname: Fixture Skill\ndescription: e2e\n---\n",
    )
    .unwrap();

    let skills = data(&run(&home, &state, &["skills", "list"]));
    assert_eq!(skills.as_array().unwrap().len(), 1);
    let skill_id = skills[0]["id"].as_str().unwrap();
    let project_record = data(&run(
        &home,
        &state,
        &["projects", "add", project.to_str().unwrap()],
    ));
    let target = format!(
        "claude-code:project:{}",
        project_record["id"].as_str().unwrap()
    );
    let created = data(&run(
        &home,
        &state,
        &[
            "presets",
            "create",
            "--name",
            "Fixture Set",
            "--skill",
            skill_id,
        ],
    ));
    let preset_id = created["plan"]["preset_updates"]
        .as_object()
        .unwrap()
        .keys()
        .next()
        .unwrap()
        .clone();

    let preview = data(&run(
        &home,
        &state,
        &[
            "presets",
            "enable",
            &preset_id,
            "--target",
            &target,
            "--preview",
        ],
    ));
    assert_eq!(preview["preview"], true);
    assert!(
        !project.join(".claude/skills").exists(),
        "preview wrote a native entry"
    );

    let applied = data(&run(
        &home,
        &state,
        &["presets", "enable", &preset_id, "--target", &target],
    ));
    assert_eq!(applied["targets"][0]["status"], "applied");
    let entries = fs::read_dir(project.join(".claude/skills"))
        .unwrap()
        .count();
    assert_eq!(entries, 1);

    let targets = data(&run(&home, &state, &["targets", "list"]));
    let row = targets
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["id"] == target)
        .unwrap();
    assert_eq!(row["state"]["enabled_skills"][0], skill_id);
    let listed = data(&run(&home, &state, &["presets", "list"]));
    assert_eq!(listed[0]["id"], preset_id);
}

#[test]
fn cli_json_errors_include_impact_and_missing_target_is_nonzero() {
    let fixture = fixture();
    let home = fixture.path().join("home");
    let state = fixture.path().join("state");
    let bad = run(&home, &state, &["presets", "enable", "unknown-preset"]);
    assert!(!bad.status.success());
    let value: Value = serde_json::from_slice(&bad.stdout).unwrap();
    assert_eq!(value["ok"], false);
    assert!(value["error"]["code"].is_string());
    assert!(value["error"].get("impact").is_some());

    let missing_args = Command::new(env!("CARGO_BIN_EXE_packcaptain"))
        .args([
            "--agent-home",
            home.to_str().unwrap(),
            "--state-dir",
            state.to_str().unwrap(),
            "--json",
            "presets",
            "enable",
            "p",
        ])
        .output()
        .unwrap();
    assert!(!missing_args.status.success());
    let parse_error: Value = serde_json::from_slice(&missing_args.stdout).unwrap();
    assert_eq!(parse_error["ok"], false);
    assert_eq!(parse_error["error"]["code"], "invalid-arguments");
    assert_eq!(parse_error["error"]["impact"], serde_json::json!([]));
    assert!(missing_args.stderr.is_empty());
}

#[test]
fn cli_version_exits_successfully_without_initializing_storage() {
    let fixture = fixture();
    let home = fixture.path().join("home");
    let state = fixture.path().join("state");
    for args in [&["--version"][..]] {
        let output = run(&home, &state, args);
        assert_eq!(output.status.code(), Some(0));
        assert_eq!(
            String::from_utf8_lossy(&output.stdout),
            "packcaptain 0.1.0\n"
        );
        assert!(output.stderr.is_empty());
        assert!(!state.exists());
    }
}

#[test]
fn cli_help_exits_successfully_without_initializing_storage() {
    let fixture = fixture();
    let home = fixture.path().join("home");
    let state = fixture.path().join("state");
    for args in [&["--help"][..], &["presets", "--help"][..]] {
        let output = run(&home, &state, args);
        assert_eq!(output.status.code(), Some(0));
        assert!(String::from_utf8_lossy(&output.stdout).contains("Usage:"));
        assert!(output.stderr.is_empty());
        assert!(!state.exists());
    }
}
