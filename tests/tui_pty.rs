use std::{
    fs,
    io::{Read, Write},
    path::Path,
    process::Command,
    sync::{Arc, Mutex},
    thread,
    time::Duration,
};

use portable_pty::{CommandBuilder, PtySize, native_pty_system};
use serde_json::Value;
use tempfile::TempDir;

fn cli(home: &Path, state: &Path, args: &[&str]) -> Value {
    let output = Command::new(env!("CARGO_BIN_EXE_packcaptain"))
        .args([
            "--agent-home",
            home.to_str().unwrap(),
            "--state-dir",
            state.to_str().unwrap(),
            "--json",
        ])
        .args(args)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stdout)
    );
    serde_json::from_slice::<Value>(&output.stdout).unwrap()["data"].clone()
}

fn key(writer: &mut dyn Write, bytes: &[u8]) {
    writer.write_all(bytes).unwrap();
    writer.flush().unwrap();
    thread::sleep(Duration::from_millis(250));
}

fn visible_text(raw: &[u8]) -> String {
    let mut visible = Vec::with_capacity(raw.len());
    let mut cursor = 0;
    while cursor < raw.len() {
        if raw[cursor] == 0x1b && raw.get(cursor + 1) == Some(&b'[') {
            cursor += 2;
            while cursor < raw.len() {
                let byte = raw[cursor];
                cursor += 1;
                if (0x40..=0x7e).contains(&byte) {
                    break;
                }
            }
        } else {
            visible.push(raw[cursor]);
            cursor += 1;
        }
    }
    String::from_utf8_lossy(&visible).into_owned()
}

#[test]
fn pty_scans_project_searches_creates_preset_and_applies_explicit_target() {
    let base =
        Path::new("/private/tmp").join(format!("packcaptain-frontend-pty-{}", std::process::id()));
    fs::create_dir_all(&base).unwrap();
    let fixture = TempDir::new_in(&base).unwrap();
    let home = fixture.path().join("home");
    let state = fixture.path().join("state");
    let project = fixture.path().join("project");
    let source = home.join(".claude/skills/pty-skill");
    fs::create_dir_all(&source).unwrap();
    fs::create_dir_all(&project).unwrap();
    fs::write(
        project.join("Cargo.toml"),
        "[package]\nname='pty-fixture'\nversion='0.1.0'\nedition='2024'\n",
    )
    .unwrap();
    fs::write(
        source.join("SKILL.md"),
        "---\nname: PTY Skill\ndescription: interactive\n---\n",
    )
    .unwrap();

    let pty = native_pty_system();
    let pair = pty
        .openpty(PtySize {
            rows: 30,
            cols: 120,
            pixel_width: 0,
            pixel_height: 0,
        })
        .unwrap();
    let mut command = CommandBuilder::new(env!("CARGO_BIN_EXE_packcaptain"));
    command.args([
        "--agent-home",
        home.to_str().unwrap(),
        "--state-dir",
        state.to_str().unwrap(),
    ]);
    let mut child = pair.slave.spawn_command(command).unwrap();
    drop(pair.slave);
    let mut reader = pair.master.try_clone_reader().unwrap();
    let transcript = Arc::new(Mutex::new(Vec::new()));
    let captured = Arc::clone(&transcript);
    let reader_thread = thread::spawn(move || {
        let mut chunk = [0u8; 4096];
        loop {
            match reader.read(&mut chunk) {
                Ok(0) | Err(_) => break,
                Ok(count) => captured.lock().unwrap().extend_from_slice(&chunk[..count]),
            }
        }
    });
    let mut writer = pair.master.take_writer().unwrap();
    thread::sleep(Duration::from_millis(600));

    key(&mut writer, b"\x1b[B");
    key(&mut writer, b"\x1b[B");
    key(&mut writer, b"\r");
    key(&mut writer, project.to_str().unwrap().as_bytes());
    key(&mut writer, b"\r");
    key(&mut writer, b" ");
    key(&mut writer, b"\r");

    let projects = cli(&home, &state, &["projects", "list"]);
    assert_eq!(
        projects.as_array().unwrap().len(),
        1,
        "TUI project scan did not claim its selected project"
    );
    let project_id = projects[0]["id"].as_str().unwrap();
    let project_target = format!("claude-code:project:{project_id}");
    let known_targets = cli(&home, &state, &["targets", "list"]);
    let project_index = known_targets
        .as_array()
        .unwrap()
        .iter()
        .filter(|row| row["id"].as_str().unwrap() < project_target.as_str())
        .count();

    key(&mut writer, b"\r");
    key(&mut writer, b"PTY Preset");
    key(&mut writer, b"\r");
    key(&mut writer, b"/");
    key(&mut writer, b"PTY Skill");
    key(&mut writer, b"\r");
    key(&mut writer, b" ");
    key(&mut writer, b"\r");
    key(&mut writer, b"\r");
    key(&mut writer, b"\r");
    for _ in 0..project_index {
        key(&mut writer, b"\x1b[B");
    }
    key(&mut writer, b" ");
    key(&mut writer, b"\r");
    key(&mut writer, b"\r");

    for _ in 0..8 {
        key(&mut writer, b"\x1b[A");
    }
    key(&mut writer, b"\x1b[B");
    key(&mut writer, b"\x1b[B");
    key(&mut writer, b"\r");
    key(&mut writer, b"/");
    key(&mut writer, b"PTY Skill");
    key(&mut writer, b"\r");
    key(&mut writer, b"d");
    for _ in 0..project_index {
        key(&mut writer, b"\x1b[B");
    }
    key(&mut writer, b" ");
    key(&mut writer, b"\r");
    key(&mut writer, b"\r");

    for _ in 0..8 {
        key(&mut writer, b"\x1b[A");
    }
    key(&mut writer, b"u");
    key(&mut writer, b"\r");
    key(&mut writer, b" ");
    key(&mut writer, b"\r");
    key(&mut writer, b"\x1b");
    let closed_after_cancel = cli(&home, &state, &["targets", "list"]);
    let canceled_row = closed_after_cancel
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["id"] == project_target)
        .unwrap();
    assert_eq!(
        canceled_row["state"]["manual_closed"]
            .as_array()
            .unwrap()
            .len(),
        1,
        "cancel changed the manually closed state"
    );

    for _ in 0..8 {
        key(&mut writer, b"\x1b[A");
    }
    key(&mut writer, b"u");
    key(&mut writer, b"\r");
    key(&mut writer, b" ");
    key(&mut writer, b"\r");
    key(&mut writer, b"\r");
    key(&mut writer, b"\r");

    for _ in 0..8 {
        key(&mut writer, b"\x1b[A");
    }
    key(&mut writer, b"u");
    key(&mut writer, b"\r");
    key(&mut writer, b" ");
    key(&mut writer, b"\r");
    key(&mut writer, b"\x1b[B");
    key(&mut writer, b"\r");
    key(&mut writer, b"\r");
    key(&mut writer, b"q");
    drop(writer);
    let status = child.wait().unwrap();
    reader_thread.join().unwrap();

    let raw = transcript.lock().unwrap().clone();
    let transcript_path = base.join(format!("pty-transcript-{}.bin", std::process::id()));
    fs::write(&transcript_path, &raw).unwrap();
    assert!(
        status.success(),
        "TUI exited with {status}; transcript {}",
        transcript_path.display()
    );
    let text = visible_text(&raw);
    assert!(
        text.contains("PTY") && text.contains("Skill"),
        "search result absent from PTY transcript at {}",
        transcript_path.display()
    );
    assert!(
        text.contains("影响预览"),
        "impact preview absent from PTY transcript at {}",
        transcript_path.display()
    );
    assert!(
        text.contains("执行成功"),
        "execution result absent from PTY transcript at {}",
        transcript_path.display()
    );
    assert!(
        text.contains("keep") && text.contains("restore"),
        "TUI policy choices absent from PTY transcript at {}",
        transcript_path.display()
    );

    let presets = cli(&home, &state, &["presets", "list"]);
    assert_eq!(presets.as_array().unwrap().len(), 1);
    assert_eq!(presets[0]["name"], "PTY Preset");
    let targets = cli(&home, &state, &["targets", "list"]);
    let row = targets
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["id"] == project_target)
        .unwrap();
    assert_eq!(row["state"]["enabled_skills"].as_array().unwrap().len(), 1);
    assert_eq!(row["state"]["manual_closed"].as_array().unwrap().len(), 0);
    assert_eq!(
        fs::read_dir(project.join(".claude/skills"))
            .unwrap()
            .count(),
        1
    );
    eprintln!("raw PTY transcript: {}", transcript_path.display());
}
