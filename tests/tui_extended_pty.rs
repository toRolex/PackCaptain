use std::{
    fs,
    io::{Read, Write},
    path::PathBuf,
    process::Command,
    sync::{Arc, Mutex, mpsc},
    thread,
    time::{Duration, Instant},
};

use portable_pty::{Child, CommandBuilder, PtySize, native_pty_system};
use serde_json::{Value, json};
use tempfile::TempDir;

#[path = "support/pty_screen.rs"]
mod pty_screen;

fn evidence() -> PathBuf {
    std::env::var_os("PACKCAPTAIN_PTY_EVIDENCE")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            std::env::temp_dir().join(format!("packcaptain-extended-pty-{}", std::process::id()))
        })
}

/// Bounded PTY session: every key waits on a rendered frame, the child is
/// killed by a watchdog, the transcript is flushed continuously, and the
/// reader is joined through a channel with a timeout instead of an
/// unbounded `join`.
struct Session {
    child: Box<dyn Child + Send + Sync>,
    writer: Box<dyn Write + Send>,
    raw: Arc<Mutex<Vec<u8>>>,
    transcript_path: PathBuf,
    observations: String,
    name: String,
    started: Instant,
    watchdog_stop: mpsc::Sender<()>,
    reader_done: mpsc::Receiver<()>,
}

impl Session {
    fn start(fixture: &Fixture, name: &str) -> Self {
        let pair = native_pty_system()
            .openpty(PtySize {
                rows: 18,
                cols: 120,
                pixel_width: 0,
                pixel_height: 0,
            })
            .unwrap();
        let mut command = CommandBuilder::new(env!("CARGO_BIN_EXE_packcaptain"));
        command.args([
            "--agent-home",
            fixture.home.to_str().unwrap(),
            "--state-dir",
            fixture.state.to_str().unwrap(),
        ]);
        command.env("HOME", &fixture.home);
        command.env("TERM", "xterm-256color");
        command.cwd(fixture.root.path());
        let child = pair.slave.spawn_command(command).unwrap();
        drop(pair.slave);
        fs::create_dir_all(evidence()).unwrap();
        let transcript_path = evidence().join(format!("{name}.transcript.bin"));
        let log = fs::File::create(&transcript_path).unwrap();
        let raw = Arc::new(Mutex::new(Vec::new()));
        let capture = Arc::clone(&raw);
        let mut reader = pair.master.try_clone_reader().unwrap();
        let (done_tx, reader_done) = mpsc::channel();
        thread::spawn(move || {
            let mut log = log;
            let mut buf = [0; 8192];
            while let Ok(count) = reader.read(&mut buf) {
                if count == 0 {
                    break;
                }
                let _ = log.write_all(&buf[..count]);
                let _ = log.sync_data();
                capture.lock().unwrap().extend_from_slice(&buf[..count]);
            }
            let _ = done_tx.send(());
        });
        let (watchdog_stop, stop_rx) = mpsc::channel();
        let mut killer = child.clone_killer();
        thread::spawn(move || {
            if matches!(
                stop_rx.recv_timeout(Duration::from_secs(55)),
                Err(mpsc::RecvTimeoutError::Timeout)
            ) {
                let _ = killer.kill();
            }
        });
        Self {
            child,
            writer: pair.master.take_writer().unwrap(),
            raw,
            transcript_path,
            observations: String::new(),
            name: name.into(),
            started: Instant::now(),
            watchdog_stop,
            reader_done,
        }
    }

    fn screen(&self) -> pty_screen::Screen {
        let raw = self.raw.lock().unwrap();
        // Text-input draws finish with Show instead of Hide.
        let normalized = String::from_utf8_lossy(&raw).replace("\x1b[?25h", "\x1b[?25l");
        pty_screen::Screen::replay(normalized.as_bytes(), 18, 120)
    }

    fn text(&self) -> String {
        self.screen().text()
    }

    fn selected(&self) -> String {
        self.text()
            .lines()
            .map(|line| line.trim_matches('│').trim())
            .find(|line| line.starts_with("› "))
            .unwrap_or("")
            .to_owned()
    }

    fn key(&mut self, bytes: &[u8]) {
        self.writer.write_all(bytes).unwrap();
        self.writer.flush().unwrap();
    }

    /// Send bytes and wait for a new frame whose screen satisfies `predicate`.
    fn key_when(&mut self, bytes: &[u8], label: &str, predicate: impl Fn(&str) -> bool) {
        // Let pending renders settle so `before` cannot match a frame drawn
        // for the previous key.
        let mut last = self.screen().frames;
        let quiesce = Instant::now() + Duration::from_secs(1);
        loop {
            thread::sleep(Duration::from_millis(40));
            let now = self.screen().frames;
            if now == last || Instant::now() > quiesce {
                break;
            }
            last = now;
        }
        let before = last;
        self.observations.push_str(&format!(
            "send {label} elapsed_ms={} after_frame={before}\n",
            self.started.elapsed().as_millis()
        ));
        self.key(bytes);
        let deadline = Instant::now() + Duration::from_secs(4);
        loop {
            let screen = self.screen();
            if screen.frames > before && predicate(&screen.text()) {
                return;
            }
            assert!(
                Instant::now() < deadline,
                "{label}: render did not reach expected state; selected={:?} transcript {}\n{}",
                self.selected(),
                self.transcript_path.display(),
                screen.text()
            );
            assert!(
                self.child.try_wait().unwrap().is_none(),
                "PTY child exited during {label}\n{}",
                screen.text()
            );
            thread::sleep(Duration::from_millis(10));
        }
    }

    /// Wait for the first frame containing `expected` without sending keys.
    fn wait_for(&mut self, expected: &str) {
        let deadline = Instant::now() + Duration::from_secs(4);
        loop {
            let screen = self.screen();
            if screen.frames > 0 && screen.text().contains(expected) {
                return;
            }
            assert!(
                Instant::now() < deadline,
                "initial frame missing {expected}; transcript {}\n{}",
                self.transcript_path.display(),
                screen.text()
            );
            assert!(
                self.child.try_wait().unwrap().is_none(),
                "PTY child exited at startup"
            );
            thread::sleep(Duration::from_millis(10));
        }
    }

    fn step(&mut self, bytes: &[u8], label: &str, expected: &str) {
        self.key_when(bytes, label, |text| text.contains(expected));
    }

    /// Press Down until the selected row matches `needle`, bounded by the
    /// number of rows on screen.
    fn select_row(&mut self, needle: &str) {
        for attempt in 0..12 {
            if self.selected().contains(needle) {
                return;
            }
            let before = self.screen().frames;
            self.key(b"\x1b[B");
            let deadline = Instant::now() + Duration::from_secs(4);
            loop {
                let screen = self.screen();
                if screen.frames > before {
                    break;
                }
                assert!(
                    Instant::now() < deadline,
                    "{needle}: no frame after Down (attempt {attempt}); transcript {}\n{}",
                    self.transcript_path.display(),
                    screen.text()
                );
                assert!(
                    self.child.try_wait().unwrap().is_none(),
                    "PTY child exited selecting {needle}"
                );
                thread::sleep(Duration::from_millis(10));
            }
        }
        panic!(
            "row {needle} never selected; selected={:?} transcript {}",
            self.selected(),
            self.transcript_path.display()
        );
    }

    fn finish(mut self, expected: &[&str]) {
        // A bare ESC followed immediately by q is parsed as Alt+q, which
        // would skip the cancel frame; keep the two keys in separate reads.
        thread::sleep(Duration::from_millis(200));
        self.key(b"q");
        let deadline = Instant::now() + Duration::from_secs(4);
        let status = loop {
            if let Some(status) = self.child.try_wait().unwrap() {
                break status;
            }
            assert!(
                Instant::now() < deadline,
                "TUI did not exit; transcript {}\n{}",
                self.transcript_path.display(),
                self.text()
            );
            thread::sleep(Duration::from_millis(20));
        };
        let _ = self.watchdog_stop.send(());
        assert!(
            self.reader_done
                .recv_timeout(Duration::from_secs(2))
                .is_ok(),
            "PTY reader did not finish; transcript {}",
            self.transcript_path.display()
        );
        self.observations.push_str(&format!(
            "q exit={status:?} elapsed_ms={}\n",
            self.started.elapsed().as_millis()
        ));
        self.save();
        assert!(status.success(), "TUI exit failed for {}", self.name);
        let text = self.text();
        for expected in expected {
            assert!(
                text.contains(expected),
                "missing {expected} in {} (transcript {})",
                self.name,
                self.transcript_path.display()
            );
        }
    }

    fn save(&self) {
        fs::create_dir_all(evidence()).unwrap();
        fs::write(
            evidence().join(format!("{}.viewport.log", self.name)),
            &self.observations,
        )
        .unwrap();
        fs::write(evidence().join(format!("{}.txt", self.name)), self.text()).unwrap();
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
        let _ = self.reader_done.recv_timeout(Duration::from_secs(2));
        if thread::panicking() {
            eprintln!(
                "failure transcript {}\nlast screen:\n{}",
                self.transcript_path.display(),
                self.text()
            );
        }
        self.save();
    }
}

struct Fixture {
    root: TempDir,
    home: PathBuf,
    state: PathBuf,
    projects: Vec<PathBuf>,
    targets: Vec<String>,
    skills: Vec<String>,
    preset: String,
}

impl Fixture {
    fn new(skill_count: usize) -> Self {
        fs::create_dir_all(evidence()).unwrap();
        let root = TempDir::new_in(evidence()).unwrap();
        let home = root.path().join("home");
        let state = root.path().join("state");
        for index in 0..skill_count {
            let source = home.join(format!(".claude/skills/skill-{index:03}"));
            fs::create_dir_all(&source).unwrap();
            fs::write(
                source.join("SKILL.md"),
                format!("---\nname: Skill{index:03}\ndescription: PTY fixture\n---\n"),
            )
            .unwrap();
        }
        let mut fixture = Self {
            root,
            home,
            state,
            projects: Vec::new(),
            targets: Vec::new(),
            skills: Vec::new(),
            preset: String::new(),
        };
        let skills = fixture.cli(&["skills", "list"]);
        for index in 0..skill_count {
            let name = format!("Skill{index:03}");
            fixture.skills.push(
                skills
                    .as_array()
                    .unwrap()
                    .iter()
                    .find(|row| row["name"] == name)
                    .unwrap()["id"]
                    .as_str()
                    .unwrap()
                    .into(),
            );
        }
        for index in 0..3 {
            let project = fixture.root.path().join(format!("project-{index}"));
            fs::create_dir_all(&project).unwrap();
            let record = fixture.cli(&["projects", "add", project.to_str().unwrap()]);
            fixture.targets.push(format!(
                "claude-code:project:{}",
                record["id"].as_str().unwrap()
            ));
            fixture.projects.push(project);
        }
        let created = fixture.cli(&[
            "presets",
            "create",
            "--name",
            "PTY Set",
            "--skill",
            &fixture.skills[0],
        ]);
        fixture.preset = created["plan"]["preset_updates"]
            .as_object()
            .unwrap()
            .keys()
            .next()
            .unwrap()
            .clone();
        fixture
    }

    fn cli(&self, args: &[&str]) -> Value {
        let output = Command::new(env!("CARGO_BIN_EXE_packcaptain"))
            .env("HOME", &self.home)
            .current_dir(self.root.path())
            .args([
                "--agent-home",
                self.home.to_str().unwrap(),
                "--state-dir",
                self.state.to_str().unwrap(),
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

    fn enable_all(&self) {
        for target in &self.targets {
            self.cli(&["presets", "enable", &self.preset, "--target", target]);
        }
    }

    /// From the scope root, enter project `index` and land on its PresetList.
    fn enter_project(&self, session: &mut Session, index: usize) {
        // Wait for the first home frame so keys cannot race raw-mode setup
        // (an early Enter would be echoed as a bare control byte).
        session.wait_for("全局");
        let needle = format!("project-{index}");
        session.select_row(&needle);
        session.step(b"\r", &format!("enter {needle}"), "Skills 管理");
    }

    /// From a project PresetList, open the preset action menu.
    fn open_preset_actions(&self, session: &mut Session) {
        session.step(b"\r", "open preset actions", "开启（选择 agents）");
    }

    /// From a project PresetList, open the preset and reach the target
    /// checkbox screen for 开启/关闭.
    fn preset_toggle_targets(&self, session: &mut Session, enable: bool) {
        self.open_preset_actions(session);
        if enable {
            session.step(b"\r", "choose enable", "选择当前作用域 agents");
        } else {
            session.step(b"\x1b[B", "choose close", "关闭（选择 agents）");
            session.step(b"\r", "confirm close", "选择当前作用域 agents");
        }
    }

    /// From a project PresetList, open a skill and reach the target checkbox
    /// screen for enabling/disabling it.
    fn skill_toggle_targets(&self, session: &mut Session, skill: &str, enable: bool) {
        session.select_row("Skills 管理");
        session.step(b"\r", "open skill pick", "Skills 清单");
        session.step(b"/", "start search", "输入搜索词");
        session.step(skill.as_bytes(), "type filter", skill);
        session.step(b"\r", "end search", "Skills 清单");
        session.step(b"\r", "open skill actions", "Skill 操作");
        if enable {
            session.step(b"\r", "choose enable skill", "选择当前作用域 agents");
        } else {
            session.step(b"\x1b[B", "choose disable skill", "关闭");
            session.step(b"\r", "confirm disable skill", "选择当前作用域 agents");
        }
    }

    fn links(&self, index: usize) -> Vec<(String, PathBuf)> {
        let directory = self.projects[index].join(".claude/skills");
        let mut links: Vec<_> = fs::read_dir(directory)
            .into_iter()
            .flatten()
            .map(|entry| {
                let entry = entry.unwrap();
                (
                    entry.file_name().to_string_lossy().into_owned(),
                    fs::read_link(entry.path()).unwrap(),
                )
            })
            .collect();
        links.sort();
        links
    }

    fn snapshot(&self) -> Value {
        json!({"presets": self.cli(&["presets", "list"]), "targets": self.cli(&["targets", "list"]), "links": (0..3).map(|index| self.links(index)).collect::<Vec<_>>()})
    }

    fn assert_members(&self, expected: &[String]) {
        let presets = self.cli(&["presets", "list"]);
        let mut actual: Vec<_> = presets[0]["members"]
            .as_array()
            .unwrap()
            .iter()
            .map(|id| id.as_str().unwrap().to_owned())
            .collect();
        let mut expected = expected.to_vec();
        actual.sort();
        expected.sort();
        assert_eq!(actual, expected);
    }

    /// Edit shared members inside project `index`'s scope: add or remove
    /// `skill` on that scope's preset-enabled targets, then confirm or cancel.
    fn edit_members(
        &self,
        name: &str,
        index: usize,
        skill: &str,
        remove: bool,
        toggle: bool,
        cancel: bool,
    ) {
        let mut session = Session::start(self, name);
        self.enter_project(&mut session, index);
        self.open_preset_actions(&mut session);
        session.step(b"\x1b[B", "menu down 1", "关闭（选择 agents）");
        session.step(b"\x1b[B", "menu down 2", "编辑共享成员并同步");
        session.step(b"\r", "open member edit", "仅支持同步保存");
        for _ in 0..6 {
            session.step(b"\x1b[A", "cursor up", "仅支持同步保存");
        }
        session.step(b"/", "member search", "输入搜索词");
        session.step(skill.as_bytes(), "member filter", skill);
        session.step(b"\r", "member filter end", "仅支持同步保存");
        let want = if remove { "[ ]" } else { "[x]" };
        if toggle {
            session.key_when(b" ", "toggle member", move |text| {
                text.lines().any(|line| {
                    let line = line.trim_matches('│').trim();
                    line.starts_with("› ") && line.contains(want) && line.contains(skill)
                })
            });
        } else {
            let deadline = Instant::now() + Duration::from_secs(4);
            loop {
                let selected = session.selected();
                if selected.contains(skill) && selected.contains(want) {
                    break;
                }
                assert!(
                    Instant::now() < deadline,
                    "member row {skill} not in state {want}: {selected}"
                );
                thread::sleep(Duration::from_millis(10));
            }
        }
        session.step(
            b"\r",
            "choose sync targets",
            "选择已开启此 Preset 的 agents",
        );
        session.step(b" ", "check target", "[x]");
        session.step(b"\r", "member preview", "影响预览");
        session.key(if cancel { b"\x1b" } else { b"\r" });
        let expected = if cancel {
            // EditMembers is its own cancel source screen and its refresh does
            // not replace the notice, so the cancel feedback stays visible.
            "已取消，未写入"
        } else {
            "执行成功"
        };
        session.finish(&[expected]);
    }
}

fn passed(name: &str, details: &str) {
    fs::write(
        evidence().join(format!("{name}.assertions.txt")),
        format!("PASS {details}\n"),
    )
    .unwrap();
}

#[test]
fn pty_preview_then_external_shared_preset_enable_preserves_fresh_state() {
    let fixture = Fixture::new(2);
    let other = fixture.cli(&[
        "presets",
        "create",
        "--name",
        "External Set",
        "--skill",
        &fixture.skills[0],
        "--skill",
        &fixture.skills[1],
    ]);
    let other = other["plan"]["preset_updates"]
        .as_object()
        .unwrap()
        .keys()
        .next()
        .unwrap();
    let mut session = Session::start(&fixture, "fresh-shared");
    fixture.enter_project(&mut session, 0);
    fixture.preset_toggle_targets(&mut session, true);
    session.step(b" ", "check target", "[x]");
    session.step(b"\r", "preview enable", "影响预览");
    fixture.cli(&["presets", "enable", other, "--target", &fixture.targets[0]]);
    let external =
        packcaptain::storage::StateStore::load_existing_snapshot(&fixture.state).unwrap();
    let external_links = fixture.links(0);
    session.step(b"\r", "first confirm", "状态已变化，请重新确认");
    let actual = packcaptain::storage::StateStore::load_existing_snapshot(&fixture.state).unwrap();
    let target: packcaptain::domain::TargetId = fixture.targets[0].parse().unwrap();
    let preset = packcaptain::domain::PresetId::from(fixture.preset.as_str());
    assert_eq!(
        actual, external,
        "first confirmation must not write business state"
    );
    assert_eq!(fixture.links(0), external_links);
    session.step(b"\r", "second confirm", "执行成功");
    let actual = packcaptain::storage::StateStore::load_existing_snapshot(&fixture.state).unwrap();
    assert!(actual.targets[&target].presets[&preset].enabled);
    assert!(
        actual.targets[&target].presets[&packcaptain::domain::PresetId::from(other.as_str())]
            .enabled
    );
    assert_eq!(
        actual.targets[&target].enabled_skills,
        external.targets[&target].enabled_skills
    );
    assert_eq!(actual.entries[&target], external.entries[&target]);
    let mut expected: Vec<_> = (0..2)
        .map(|index| {
            (
                format!("pc-{}", fixture.skills[index]),
                fs::canonicalize(
                    fixture
                        .home
                        .join(format!(".claude/skills/skill-{index:03}")),
                )
                .unwrap(),
            )
        })
        .collect();
    expected.sort();
    assert_eq!(fixture.links(0), expected);
    assert!(fixture.links(1).is_empty());
    assert!(fixture.links(2).is_empty());
    session.finish(&[]);
    passed(
        "fresh-shared",
        "external preset, enabled skills and owned entries preserved after reopened SQLite; exact links on selected target only",
    );
}

#[test]
fn pty_changed_preview_rechecks_again_and_cancel_writes_nothing() {
    let fixture = Fixture::new(2);
    let mut session = Session::start(&fixture, "fresh-recheck-cancel");
    fixture.enter_project(&mut session, 0);
    fixture.preset_toggle_targets(&mut session, true);
    session.step(b" ", "check target", "[x]");
    session.step(b"\r", "preview enable", "影响预览");
    for skill in &fixture.skills {
        fixture.cli(&["skills", "enable", skill, "--target", &fixture.targets[0]]);
        let before =
            packcaptain::storage::StateStore::load_existing_snapshot(&fixture.state).unwrap();
        let links = fixture.links(0);
        session.step(b"\r", "recheck confirm", "状态已变化，请重新确认");
        assert_eq!(
            packcaptain::storage::StateStore::load_existing_snapshot(&fixture.state).unwrap(),
            before
        );
        assert_eq!(fixture.links(0), links);
    }
    let before = fixture.snapshot();
    session.step(b"\x1b", "cancel", "开启（选择 agents）");
    assert_eq!(fixture.snapshot(), before);
    session.finish(&[]);
    passed(
        "fresh-recheck-cancel",
        "two successive real CLI changes both require reconfirmation; Escape preserves entire semantic state and links",
    );
}

#[test]
fn pty_manual_close_during_preview_requires_explicit_policy() {
    let fixture = Fixture::new(2);
    fixture.cli(&[
        "presets",
        "enable",
        &fixture.preset,
        "--target",
        &fixture.targets[0],
    ]);
    let mut session = Session::start(&fixture, "fresh-late-policy");
    fixture.enter_project(&mut session, 0);
    fixture.open_preset_actions(&mut session);
    session.step(b"\x1b[B", "menu down 1", "关闭（选择 agents）");
    session.step(b"\x1b[B", "menu down 2", "编辑共享成员并同步");
    session.step(b"\r", "open member edit", "仅支持同步保存");
    for _ in 0..6 {
        session.step(b"\x1b[A", "cursor up", "仅支持同步保存");
    }
    session.step(b"/", "member search", "输入搜索词");
    session.step(b"Skill001", "member filter", "Skill001");
    session.step(b"\r", "member filter end", "仅支持同步保存");
    session.step(b" ", "toggle member", "[x]");
    session.step(
        b"\r",
        "choose sync targets",
        "选择已开启此 Preset 的 agents",
    );
    session.step(b" ", "check target", "[x]");
    session.step(b"\r", "member preview", "影响预览");
    fixture.cli(&[
        "skills",
        "disable",
        &fixture.skills[0],
        "--target",
        &fixture.targets[0],
    ]);
    let before = packcaptain::storage::StateStore::load_existing_snapshot(&fixture.state).unwrap();
    let links = fixture.links(0);
    session.step(b"\r", "conflict confirm", "手动关闭冲突策略");
    assert_eq!(
        packcaptain::storage::StateStore::load_existing_snapshot(&fixture.state).unwrap(),
        before
    );
    assert_eq!(fixture.links(0), links);
    session.step(b"\r", "keep policy", "影响预览");
    session.step(b"\r", "policy confirm", "执行成功");
    let after = packcaptain::storage::StateStore::load_existing_snapshot(&fixture.state).unwrap();
    let target: packcaptain::domain::TargetId = fixture.targets[0].parse().unwrap();
    assert_eq!(
        after.targets[&target].manual_closed,
        before.targets[&target].manual_closed
    );
    assert_eq!(
        after.targets[&target].enabled_skills,
        std::collections::BTreeSet::from([packcaptain::domain::SkillId::from(
            fixture.skills[1].as_str()
        )])
    );
    session.finish(&[]);
    passed(
        "fresh-late-policy",
        "new manual-close conflict writes nothing until explicit Keep selection and a fresh preview confirmation; closed member stays closed",
    );
}

#[test]
fn pty_foreign_replacement_after_preview_is_rejected_without_intent_change() {
    let fixture = Fixture::new(1);
    fixture.cli(&[
        "presets",
        "enable",
        &fixture.preset,
        "--target",
        &fixture.targets[0],
    ]);
    let mut session = Session::start(&fixture, "fresh-foreign");
    fixture.enter_project(&mut session, 0);
    fixture.preset_toggle_targets(&mut session, false);
    session.step(b" ", "check target", "[x]");
    session.step(b"\r", "preview disable", "影响预览");
    let before = packcaptain::storage::StateStore::load_existing_snapshot(&fixture.state).unwrap();
    let path = fixture.projects[0].join(format!(".claude/skills/pc-{}", fixture.skills[0]));
    fs::remove_file(&path).unwrap();
    fs::create_dir(&path).unwrap();
    fs::write(path.join("foreign"), "do not touch").unwrap();
    session.step(b"\r", "foreign confirm", "部分失败");
    assert_eq!(
        packcaptain::storage::StateStore::load_existing_snapshot(&fixture.state).unwrap(),
        before
    );
    assert_eq!(
        fs::read_to_string(path.join("foreign")).unwrap(),
        "do not touch"
    );
    session.finish(&["失败 1"]);
    passed(
        "fresh-foreign",
        "foreign directory content and stored intent/owned metadata unchanged after rejected disable",
    );
}

#[test]
fn pty_pending_journal_is_not_consumed_by_confirmation() {
    use packcaptain::domain::{Effect, TargetId};
    use packcaptain::storage::StateStore;
    use rusqlite::{Connection, params};
    let fixture = Fixture::new(1);
    fixture.cli(&[
        "presets",
        "enable",
        &fixture.preset,
        "--target",
        &fixture.targets[0],
    ]);
    let mut session = Session::start(&fixture, "fresh-pending");
    fixture.enter_project(&mut session, 0);
    fixture.preset_toggle_targets(&mut session, false);
    session.step(b" ", "check target", "[x]");
    session.step(b"\r", "preview disable", "影响预览");
    let before = StateStore::load_existing_snapshot(&fixture.state).unwrap();
    let target: TargetId = fixture.targets[0].parse().unwrap();
    let skill = packcaptain::domain::SkillId::from(fixture.skills[0].as_str());
    let mut after = before.targets[&target].clone();
    after.enabled_skills.clear();
    let journal = json!({"id":"d2-pending", "target":target, "status":"Applying", "before":before.targets[&target], "after":after, "entries_before":before.entries[&target], "entries_after":before.entries[&target], "planned_target_states":{target.to_string():after}, "effects":[Effect::SetSkillEnabled{target:target.clone(), skill, enabled:false}], "failure":null}).to_string();
    let connection = Connection::open(fixture.state.join("packcaptain.sqlite3")).unwrap();
    connection
        .execute(
            "INSERT INTO operation_journal VALUES (?1, ?2)",
            params!["d2-pending", journal],
        )
        .unwrap();
    let path = fixture.projects[0].join(format!(".claude/skills/pc-{}", fixture.skills[0]));
    #[cfg(unix)]
    let identity = {
        use std::os::unix::fs::MetadataExt;
        let metadata = fs::symlink_metadata(&path).unwrap();
        (metadata.dev(), metadata.ino())
    };
    let links = fixture.links(0);
    session.step(b"\r", "pending confirm", "存在未完成操作");
    assert_eq!(
        StateStore::load_existing_snapshot(&fixture.state).unwrap(),
        before
    );
    assert_eq!(
        connection
            .query_row::<String, _, _>("SELECT payload FROM operation_journal", [], |row| row
                .get(0))
            .unwrap(),
        journal
    );
    assert_eq!(fixture.links(0), links);
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        let metadata = fs::symlink_metadata(&path).unwrap();
        assert_eq!((metadata.dev(), metadata.ino()), identity);
    }
    session.finish(&[]);
    passed(
        "fresh-pending",
        "real SQLite Applying journal retained byte-for-byte; snapshot and symlink no-follow device/inode/readlink preserved",
    );
}

#[test]
fn pty_members_add_remove_and_sync_only_explicit_targets() {
    let fixture = Fixture::new(2);
    fixture.enable_all();
    let before = fixture.snapshot();
    fixture.edit_members("members-add-0", 0, "Skill001", false, true, false);
    fixture.edit_members("members-add-1", 1, "Skill001", false, false, false);
    fixture.assert_members(&fixture.skills);
    for index in 0..2 {
        assert_eq!(fixture.links(index).len(), 2);
    }
    assert_eq!(fixture.links(2).len(), 1);
    let after = fixture.snapshot();
    let target = &fixture.targets[2];
    let row = |snapshot: &Value| {
        snapshot["targets"]
            .as_array()
            .unwrap()
            .iter()
            .find(|row| row["id"] == *target)
            .unwrap()
            .clone()
    };
    assert_eq!(row(&before), row(&after));
    assert_eq!(before["links"][2], after["links"][2]);
    fixture.edit_members("members-remove-0", 0, "Skill000", true, true, false);
    fixture.edit_members("members-remove-1", 1, "Skill001", false, false, false);
    fixture.assert_members(&fixture.skills[1..]);
    for index in 0..2 {
        let links = fixture.links(index);
        assert_eq!(links.len(), 1);
        assert!(links[0].1.ends_with("skill-001"));
    }
    assert_eq!(row(&before), row(&fixture.snapshot()));
    passed(
        "members-and-scope",
        "added Skill001 and removed Skill000 on the two explicit project scopes; third target state and links unchanged",
    );
}

#[test]
fn pty_cancel_preserves_entire_semantic_state_and_links() {
    let fixture = Fixture::new(2);
    fixture.enable_all();
    let before = fixture.snapshot();
    fixture.edit_members("cancel-edit", 0, "Skill001", false, true, true);
    assert_eq!(fixture.snapshot(), before);
    passed(
        "cancel",
        "preset definitions, all target states and all symlink names/destinations exactly equal before and after canceled confirmation",
    );
}

#[test]
fn pty_failure_identifies_failed_target_and_preserves_success() {
    use packcaptain::domain::{AgentId, PresetId, Scope, TargetId};
    use packcaptain::storage::StateStore;

    let fixture = Fixture::new(1);
    let codex = fixture.targets[0].replacen("claude-code:", "codex:", 1);
    for target in [&fixture.targets[0], &codex] {
        fixture.cli(&["presets", "enable", &fixture.preset, "--target", target]);
    }
    let before = StateStore::load_existing_snapshot(&fixture.state).unwrap();
    let successful: TargetId = fixture.targets[0].parse().unwrap();
    let failed: TargetId = codex.parse().unwrap();
    assert_eq!(failed.agent, AgentId::Codex);
    assert_eq!(failed.scope, successful.scope);
    let Scope::Project(project_id) = &failed.scope else {
        panic!("failure target must be project-scoped");
    };
    assert_eq!(before.projects[project_id].name, "project-0");
    assert_eq!(
        before.projects[project_id].root,
        fs::canonicalize(&fixture.projects[0]).unwrap()
    );
    assert_eq!(
        before
            .projects
            .values()
            .filter(|project| project.name == "project-0")
            .count(),
        1
    );
    assert!(before.targets.contains_key(&failed));
    let preset = PresetId::from(fixture.preset.as_str());
    let expected_source = fs::canonicalize(fixture.home.join(".claude/skills/skill-000")).unwrap();
    let blocked = fixture.projects[0].join(format!(".codex/skills/pc-{}", fixture.skills[0]));
    assert_eq!(fs::read_link(&blocked).unwrap(), expected_source);
    assert_eq!(
        fixture.links(0),
        vec![(format!("pc-{}", fixture.skills[0]), expected_source)]
    );

    let mut session = Session::start(&fixture, "partial-failure");
    fixture.enter_project(&mut session, 0);
    fixture.preset_toggle_targets(&mut session, false);
    assert!(session.selected().contains("Claude Code"));
    session.step(b" ", "check Claude", "[x] Claude Code");
    session.select_row("Codex");
    session.step(b" ", "check Codex", "[x] Codex");
    session.step(b"\r", "preview both targets", "影响预览");
    assert!(session.text().contains("Claude Code") && session.text().contains("Codex"));
    assert_eq!(
        StateStore::load_existing_snapshot(&fixture.state).unwrap(),
        before
    );

    // A foreign replacement passes discovery but must fail the owned-link
    // identity check during execution, without rolling back the other agent.
    fs::remove_file(&blocked).unwrap();
    fs::create_dir(&blocked).unwrap();
    fs::write(blocked.join("foreign"), "do not touch").unwrap();
    session.step(b"\r", "confirm mixed result", "部分失败");
    session.finish(&[
        "部分失败",
        "成功 1",
        "失败 1",
        "Codex · project-0：skill entry was replaced:",
    ]);

    let after = StateStore::load_existing_snapshot(&fixture.state).unwrap();
    assert!(!after.targets[&successful].presets[&preset].enabled);
    assert!(after.targets[&successful].enabled_skills.is_empty());
    assert!(after.targets[&successful].manual_closed.is_empty());
    let skill = packcaptain::domain::SkillId::from(fixture.skills[0].as_str());
    assert_eq!(after.entries[&successful].len(), 1);
    let routes = &after.entries[&successful][&skill];
    assert_eq!(routes.len(), 1);
    assert!(!routes[0].active);
    assert!(!routes[0].present);
    assert_eq!(routes[0].entry_fingerprint, "missing");
    assert_eq!(routes[0].path, before.entries[&successful][&skill][0].path);
    assert!(fixture.links(0).is_empty());
    assert_eq!(after.targets[&failed], before.targets[&failed]);
    assert_eq!(after.entries[&failed], before.entries[&failed]);
    assert!(fs::symlink_metadata(&blocked).unwrap().is_dir());
    assert_eq!(fs::read_dir(&blocked).unwrap().count(), 1);
    assert_eq!(
        fs::read_to_string(blocked.join("foreign")).unwrap(),
        "do not touch"
    );
    assert!(fixture.links(1).is_empty() && fixture.links(2).is_empty());
    passed(
        "partial-failure",
        "one project, one PTY confirmation, two real agents: success 1/failure 1 and unique failed Codex/project label mapped to snapshot TargetId; Claude disabled and link removed, Codex intent/entries and foreign directory preserved",
    );
}

#[test]
fn pty_long_list_scrolls_to_last_skill_and_back_without_crash() {
    let fixture = Fixture::new(90);
    let rows = fixture.cli(&["skills", "list"]);
    let mut ordered: Vec<_> = rows.as_array().unwrap().iter().collect();
    ordered.sort_by_key(|row| row["id"].as_str().unwrap());
    assert_eq!(ordered.len(), 90);
    let name = |index: usize| ordered[index]["name"].as_str().unwrap();
    let first = name(0);
    let last = name(89);
    let mut session = Session::start(&fixture, "long-list");
    session.wait_for("全局");
    session.step(b"\r", "enter global", "Skills 管理");
    session.select_row("Skills 管理");
    session.step(b"\r", "open inventory", "Skills 清单");
    session.step(b"\r", "select first skill", "Skill 操作");
    session.step(b"\x1b", "back to list", "Skills 清单");
    for index in 1..90 {
        let expected = name(index).to_owned();
        session.key_when(b"\x1b[B", name(index), move |text| {
            text.lines().any(|line| {
                let line = line.trim_matches('│').trim();
                line.starts_with("› ") && line.contains(&expected)
            })
        });
    }
    assert!(
        session.selected().contains(last),
        "last skill was not rendered"
    );
    let bottom_row = session.selected();
    for _ in 0..6 {
        session.key(b"\x1b[B");
        // FIFO input makes the theme frame a barrier for the preceding Down.
        session.step(b"T", "bottom-boundary barrier", "主题 Catppuccin Latte");
        assert_eq!(session.selected(), bottom_row, "bottom-boundary moved row");
        assert!(
            session.child.try_wait().unwrap().is_none(),
            "PTY child exited at bottom-boundary"
        );
        session.observations.push_str(&format!(
            "bottom-boundary selected={bottom_row:?} alive=true frames={}\n",
            session.screen().frames
        ));
        session.step(
            b"T",
            "bottom-boundary restore theme",
            "主题 Catppuccin Mocha",
        );
        assert_eq!(session.selected(), bottom_row);
    }
    for index in (0..89).rev() {
        let expected = name(index).to_owned();
        session.key_when(b"\x1b[A", name(index), move |text| {
            text.lines().any(|line| {
                let line = line.trim_matches('│').trim();
                line.starts_with("› ") && line.contains(&expected)
            })
        });
    }
    assert!(
        session.selected().contains(first),
        "did not scroll back to top"
    );
    let top_row = session.selected();
    for _ in 0..6 {
        session.key(b"\x1b[A");
        session.step(b"T", "top-boundary barrier", "主题 Catppuccin Latte");
        assert_eq!(session.selected(), top_row, "top-boundary moved row");
        assert!(
            session.child.try_wait().unwrap().is_none(),
            "PTY child exited at top-boundary"
        );
        session.observations.push_str(&format!(
            "top-boundary selected={top_row:?} alive=true frames={}\n",
            session.screen().frames
        ));
        session.step(b"T", "top-boundary restore theme", "主题 Catppuccin Mocha");
        assert_eq!(session.selected(), top_row);
    }
    session.step(b"/", "search-start", "输入搜索词");
    for byte in b"Skill089" {
        session.key(&[*byte]);
    }
    session.step(b"\r", "search-confirmed", "Skill089");
    assert!(session.selected().contains("Skill089"));
    session.step(b"\x1b", "back-to-presets", "Skills 管理");
    session.step(b"\x1b", "back-home", "全局");
    session.finish(&[]);
    passed(
        "long-list",
        "90 skills in 18-row PTY; bottom item rendered, scroll back and search work, process exits 0",
    );
}

#[test]
fn pty_config_error_does_not_consume_pending_confirmation() {
    let fixture = Fixture::new(1);
    fixture.cli(&[
        "presets",
        "enable",
        &fixture.preset,
        "--target",
        &fixture.targets[0],
    ]);
    let mut session = Session::start(&fixture, "config-error-pending");
    fixture.enter_project(&mut session, 0);
    fixture.preset_toggle_targets(&mut session, false);
    session.step(b" ", "check target", "[x]");
    session.step(b"\r", "preview disable", "影响预览");
    let before = packcaptain::storage::StateStore::load_existing_snapshot(&fixture.state).unwrap();
    let links = fixture.links(0);
    let config = fixture.projects[0].join(".claude/settings.local.json");
    fs::write(&config, "{broken-json").unwrap();
    session.step(b"\r", "broken confirm", "state is unknown");
    assert_eq!(
        packcaptain::storage::StateStore::load_existing_snapshot(&fixture.state).unwrap(),
        before
    );
    assert_eq!(fixture.links(0), links);
    assert_eq!(fs::read(&config).unwrap(), b"{broken-json");
    fs::write(&config, "{}").unwrap();
    session.step(b"\r", "retry confirm", "执行成功");
    assert!(
        fixture.links(0).is_empty(),
        "pending command must remain available after config error"
    );
    session.finish(&[]);
    passed(
        "config-error-pending",
        "parse error keeps pending Command and stored snapshot/links; fixing config allows the same confirmation to converge",
    );
}
