use std::{
    fs,
    io::{Read, Write},
    path::Path,
    process::Command,
    sync::mpsc,
    sync::{Arc, Mutex},
    thread,
    time::{Duration, Instant},
};

use portable_pty::{CommandBuilder, PtySize, native_pty_system};
use serde_json::Value;
use tempfile::TempDir;

#[path = "support/pty_screen.rs"]
mod pty_screen;

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

struct Session {
    child: Box<dyn portable_pty::Child + Send + Sync>,
    writer: Box<dyn Write + Send>,
    transcript: Arc<Mutex<Vec<u8>>>,
    transcript_path: std::path::PathBuf,
    watchdog_stop: mpsc::Sender<()>,
    reader_done: mpsc::Receiver<()>,
}

impl Session {
    fn start(home: &Path, state: &Path, log_dir: &Path) -> Self {
        let pair = native_pty_system()
            .openpty(PtySize {
                rows: 30,
                cols: 180,
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
        command.env("HOME", home);
        command.cwd(home);
        let child = pair.slave.spawn_command(command).unwrap();
        drop(pair.slave);
        let transcript_path = log_dir.join("transcript.bin");
        let mut log = fs::File::create(&transcript_path).unwrap();
        eprintln!("raw PTY transcript: {}", transcript_path.display());
        let transcript = Arc::new(Mutex::new(Vec::new()));
        let captured = Arc::clone(&transcript);
        let mut reader = pair.master.try_clone_reader().unwrap();
        let (done_tx, reader_done) = mpsc::channel();
        thread::spawn(move || {
            let mut chunk = [0u8; 4096];
            while let Ok(count) = reader.read(&mut chunk) {
                if count == 0 {
                    break;
                }
                log.write_all(&chunk[..count]).unwrap();
                log.sync_data().unwrap();
                captured.lock().unwrap().extend_from_slice(&chunk[..count]);
            }
            let _ = done_tx.send(());
        });
        let (watchdog_stop, stop_rx) = mpsc::channel();
        let mut killer = child.clone_killer();
        thread::spawn(move || {
            if matches!(
                stop_rx.recv_timeout(Duration::from_secs(45)),
                Err(mpsc::RecvTimeoutError::Timeout)
            ) {
                let _ = killer.kill();
            }
        });
        Self {
            child,
            writer: pair.master.take_writer().unwrap(),
            transcript,
            transcript_path,
            watchdog_stop,
            reader_done,
        }
    }

    fn screen(&self) -> pty_screen::Screen {
        let raw = self.transcript.lock().unwrap();
        // Text-input draws finish with Show instead of Hide.
        let normalized = String::from_utf8_lossy(&raw).replace("\x1b[?25h", "\x1b[?25l");
        pty_screen::Screen::replay(normalized.as_bytes(), 30, 180)
    }

    fn wait_until(&mut self, after: usize, expected: &str) {
        let deadline = Instant::now() + Duration::from_secs(4);
        loop {
            let screen = self.screen();
            if screen.frames > after && screen.text().contains(expected) {
                return;
            }
            assert!(
                Instant::now() < deadline,
                "waiting for {expected:?}; transcript {}\nlast screen:\n{}",
                self.transcript_path.display(),
                screen.text()
            );
            assert!(
                self.child.try_wait().unwrap().is_none(),
                "PTY child exited while waiting for {expected:?}\n{}",
                screen.text()
            );
            thread::sleep(Duration::from_millis(20));
        }
    }

    fn step(&mut self, bytes: &[u8], expected: &str) {
        let before = self.screen().frames;
        self.writer.write_all(bytes).unwrap();
        self.writer.flush().unwrap();
        self.wait_until(before, expected);
    }

    fn finish(&mut self) {
        self.writer.write_all(b"q").unwrap();
        self.writer.flush().unwrap();
        let deadline = Instant::now() + Duration::from_secs(4);
        loop {
            if let Some(status) = self.child.try_wait().unwrap() {
                assert!(
                    status.success(),
                    "TUI exited with {status}\n{}",
                    self.screen().text()
                );
                break;
            }
            assert!(
                Instant::now() < deadline,
                "TUI did not exit; transcript {}\n{}",
                self.transcript_path.display(),
                self.screen().text()
            );
            thread::sleep(Duration::from_millis(20));
        }
        let _ = self.watchdog_stop.send(());
        assert!(
            self.reader_done
                .recv_timeout(Duration::from_secs(2))
                .is_ok(),
            "PTY reader did not finish; transcript {}",
            self.transcript_path.display()
        );
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let deadline = Instant::now() + Duration::from_secs(2);
        while matches!(self.child.try_wait(), Ok(None)) && Instant::now() < deadline {
            thread::sleep(Duration::from_millis(20));
        }
        let _ = self.watchdog_stop.send(());
        if thread::panicking() {
            eprintln!(
                "failure transcript {}\nlast screen:\n{}",
                self.transcript_path.display(),
                self.screen().text()
            );
        }
    }
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

    let log_dir = tempfile::Builder::new()
        .prefix("packcaptain-u2c-pty-")
        .tempdir_in("/tmp")
        .unwrap()
        .keep();
    let mut tui = Session::start(&home, &state, &log_dir);
    tui.wait_until(0, "扫描并认领项目");
    tui.step(b"\x1b[B", "扫描并认领项目");
    tui.step(b"\r", "输入项目扫描根目录");
    tui.step(
        project.to_str().unwrap().as_bytes(),
        project.to_str().unwrap(),
    );
    tui.step(b"\r", "Enter 认领");
    tui.step(b" ", "[x]");
    tui.step(b"\r", "扫描并认领项目");

    let projects = cli(&home, &state, &["projects", "list"]);
    assert_eq!(
        projects.as_array().unwrap().len(),
        1,
        "TUI project scan did not claim its selected project"
    );
    let project_id = projects[0]["id"].as_str().unwrap();
    let project_target = format!("claude-code:project:{project_id}");

    // After claiming, the scope root selection is back on 全局; move to the
    // project row explicitly (project rows render without a checkbox).
    tui.step(b"\x1b[B", "› project");
    tui.step(b"\r", "Presets");
    tui.step(b"\r", "输入新 Preset 名称");
    tui.step(b"PTY Preset", "PTY Preset");
    tui.step(b"\r", "选择成员，Space 勾选");
    // The footer shows the notice and the filter hint, so the filter echo
    // itself is clipped; wait on the hint and the post-filter row instead.
    tui.step(b"/", "输入搜索词");
    tui.step(b"PTY Skill", "PTY Skill");
    tui.step(b"\r", "选择成员，Space 勾选");
    tui.step(b" ", "[x]");
    tui.step(b"\r", "影响预览");
    tui.step(b"\r", "执行成功");

    tui.step(b"\r", "开启（选择 agents）");
    tui.step(b"\r", "Space 勾选 agents");
    let target_screen = tui
        .screen()
        .text()
        .lines()
        .map(|line| line.trim_matches('│').trim())
        .find(|line| line.starts_with("› "))
        .unwrap()
        .to_owned();
    assert!(
        target_screen.starts_with("› [ ] Claude Code · project")
            && target_screen.contains("开启 0 skills，手动关闭 0，缺失 0"),
        "selected target row should be the project target: {target_screen}"
    );

    let target_text = tui.screen().text();
    assert!(target_text.find("Claude Code").unwrap() < target_text.find("Codex").unwrap());
    assert!(target_text.find("Codex").unwrap() < target_text.find("Pi").unwrap());
    tui.step(b" ", "[x]");
    tui.step(b"\r", "影响预览");
    tui.step(b"\r", "执行成功");

    tui.step(b"\x1b[B", "Skills 管理");
    tui.step(b"\x1b[B", "Skills 管理");
    tui.step(b"\r", "Enter 打开操作");
    tui.step(b"/", "输入搜索词");
    tui.step(b"PTY Skill", "PTY Skill");
    tui.step(b"\r", "Skills 清单");
    tui.step(b"\r", "Skill 操作");
    tui.step(b"\x1b[B", "关闭");
    tui.step(b"\r", "Space 勾选 agents");
    let disable_screen = tui.screen().text();
    let selected = disable_screen
        .lines()
        .map(|line| line.trim_matches('│').trim())
        .find(|line| line.starts_with("› "))
        .unwrap();
    assert!(
        selected.contains("› [ ] Claude Code · project")
            && selected.contains("开启 1 skills，手动关闭 0，缺失 0"),
        "selected target row after disable flow: {selected}"
    );
    tui.step(b" ", "[x]");
    tui.step(b"\r", "影响预览");
    tui.step(b"\r", "执行成功");

    // Canceling a preset preview must preserve both state and filesystem.
    let state_before_cancel = cli(&home, &state, &["targets", "list"]);
    assert_eq!(
        fs::read_dir(project.join(".claude/skills"))
            .unwrap()
            .count(),
        0
    );
    tui.step(b"\r", "开启（选择 agents）");
    tui.step(b"\r", "Space 勾选 agents");
    tui.step(b" ", "[x]");
    tui.step(b"\r", "影响预览");
    // Esc returns to the preset actions screen; its refresh replaces the
    // cancel notice, so we wait for the returned screen and verify non-write
    // through state assertions below.
    tui.step(b"\x1b", "开启（选择 agents）");
    let closed_after_cancel = cli(&home, &state, &["targets", "list"]);
    assert_eq!(
        closed_after_cancel, state_before_cancel,
        "cancel mutated target state"
    );
    assert_eq!(
        fs::read_dir(project.join(".claude/skills"))
            .unwrap()
            .count(),
        0
    );
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

    // Restore is the shared-member edit policy, not a preset toggle option.
    tui.step(b"\x1b[B", "编辑共享成员并同步");
    tui.step(b"\x1b[B", "编辑共享成员并同步");
    tui.step(b"\r", "仅支持同步保存");
    tui.step(b"\r", "选择已开启此 Preset 的 agents");
    tui.step(b" ", "[x]");
    tui.step(b"\r", "keep");
    tui.step(b"\x1b[B", "restore");
    tui.step(b"\r", "影响预览");
    tui.step(b"\r", "执行成功");
    tui.finish();

    let raw = tui.transcript.lock().unwrap().clone();
    let transcript_path = &tui.transcript_path;
    let text = visible_text(&raw);
    eprintln!("raw PTY transcript: {}", transcript_path.display());
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
