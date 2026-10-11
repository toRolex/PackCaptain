use std::{
    fs,
    io::{Read, Write},
    path::PathBuf,
    process::Command,
    sync::{Arc, Mutex},
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

struct Session {
    _master: Box<dyn portable_pty::MasterPty + Send>,
    writer: Option<Box<dyn Write + Send>>,
    child: Box<dyn Child + Send + Sync>,
    reader: Option<thread::JoinHandle<()>>,
    raw: Arc<Mutex<Vec<u8>>>,
    name: String,
    started: Instant,
    observations: String,
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
        let mut reader = pair.master.try_clone_reader().unwrap();
        let raw = Arc::new(Mutex::new(Vec::new()));
        let capture = raw.clone();
        let reader = thread::spawn(move || {
            let mut buf = [0; 8192];
            while let Ok(count) = reader.read(&mut buf) {
                if count == 0 {
                    break;
                }
                capture.lock().unwrap().extend_from_slice(&buf[..count]);
            }
        });
        let session = Self {
            writer: Some(pair.master.take_writer().unwrap()),
            _master: pair.master,
            child,
            reader: Some(reader),
            raw,
            name: name.into(),
            started: Instant::now(),
            observations: String::new(),
        };
        thread::sleep(Duration::from_millis(500));
        session
    }

    fn key(&mut self, bytes: &[u8]) {
        let writer = self.writer.as_mut().unwrap();
        writer.write_all(bytes).unwrap();
        writer.flush().unwrap();
        thread::sleep(Duration::from_millis(100));
    }

    fn screen(&self) -> pty_screen::Screen {
        pty_screen::Screen::replay(&self.raw.lock().unwrap(), 18, 120)
    }

    fn wait_screen(
        &mut self,
        label: &str,
        after_frame: usize,
        predicate: impl Fn(&pty_screen::Screen) -> bool,
    ) {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            let screen = self.screen();
            if screen.frames > after_frame && predicate(&screen) {
                self.observations.push_str(&format!(
                    "{label} elapsed_ms={} bytes={} frames={} viewport=18x120\n{}\n",
                    self.started.elapsed().as_millis(),
                    self.raw.lock().unwrap().len(),
                    screen.frames,
                    screen.text(),
                ));
                return;
            }
            assert!(
                Instant::now() < deadline,
                "{label}: render did not reach expected state; frames={} selected={:?}\n{}",
                screen.frames,
                screen.selected(),
                screen.text()
            );
            thread::sleep(Duration::from_millis(5));
        }
    }

    fn key_observed(
        &mut self,
        bytes: &[u8],
        label: &str,
        predicate: impl Fn(&pty_screen::Screen) -> bool,
    ) {
        let before = self.screen().frames;
        self.observations.push_str(&format!(
            "send {label} elapsed_ms={} after_frame={before} bytes={bytes:?}\n",
            self.started.elapsed().as_millis()
        ));
        let writer = self.writer.as_mut().unwrap();
        writer.write_all(bytes).unwrap();
        writer.flush().unwrap();
        self.wait_screen(label, before, predicate);
    }

    fn down(&mut self, count: usize) {
        for _ in 0..count {
            self.key(b"\x1b[B");
        }
    }

    fn search(&mut self, name: &str) {
        self.key(b"/");
        self.key(name.as_bytes());
        self.key(b"\r");
    }

    fn text(&self) -> String {
        let raw = self.raw.lock().unwrap();
        let mut visible = Vec::new();
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

    fn finish(mut self, expected: &[&str]) {
        self.key(b"q");
        let deadline = Instant::now() + Duration::from_secs(10);
        let status = loop {
            if let Some(status) = self.child.try_wait().unwrap() {
                break status;
            }
            assert!(Instant::now() < deadline, "TUI did not exit");
            thread::sleep(Duration::from_millis(50));
        };
        self.writer.take();
        self.reader.take().unwrap().join().unwrap();
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
                "missing {expected} in {}",
                self.name
            );
        }
    }

    fn save(&self) {
        fs::create_dir_all(evidence()).unwrap();
        fs::write(
            evidence().join(format!("{}.bin", self.name)),
            &*self.raw.lock().unwrap(),
        )
        .unwrap();
        fs::write(evidence().join(format!("{}.txt", self.name)), self.text()).unwrap();
        fs::write(
            evidence().join(format!("{}.viewport.log", self.name)),
            &self.observations,
        )
        .unwrap();
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        if self.reader.is_some() {
            let _ = self.child.kill();
            self.writer.take();
            if let Some(reader) = self.reader.take() {
                let _ = reader.join();
            }
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

    fn select(&self, session: &mut Session, targets: &[String], editing: bool) {
        let rows = self.cli(&["targets", "list"]);
        let ids: Vec<_> = rows
            .as_array()
            .unwrap()
            .iter()
            .filter(|row| {
                !editing
                    || self
                        .targets
                        .contains(&row["id"].as_str().unwrap().to_owned())
            })
            .map(|row| row["id"].as_str().unwrap())
            .collect();
        for _ in 0..ids.len() {
            session.key(b"\x1b[A");
        }
        for (index, id) in ids.iter().enumerate() {
            if index > 0 {
                session.down(1);
            }
            if targets.iter().any(|target| target == id) {
                session.key(b" ");
            }
        }
        session.key(b"\r");
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

    fn edit(&self, name: &str, skill: &str, targets: &[String], cancel: bool) {
        let mut session = Session::start(self, name);
        session.key(b"u");
        session.search(skill);
        session.key(b" ");
        session.key(b"\r");
        self.select(&mut session, targets, true);
        assert!(session.text().contains("影响预览"));
        session.key(if cancel { b"\x1b" } else { b"\r" });
        session.finish(&[if cancel {
            "已取消，未写入"
        } else {
            "执行成功"
        }]);
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
    let presets = fixture.cli(&["presets", "list"]);
    let index = presets
        .as_array()
        .unwrap()
        .iter()
        .position(|row| row["id"] == fixture.preset)
        .unwrap();
    session.down(index);
    session.key(b"e");
    fixture.select(&mut session, &fixture.targets[..1], false);
    assert!(session.text().contains("影响预览"));
    fixture.cli(&["presets", "enable", other, "--target", &fixture.targets[0]]);
    let external =
        packcaptain::storage::StateStore::load_existing_snapshot(&fixture.state).unwrap();
    let external_links = fixture.links(0);
    session.key(b"\r");
    let actual = packcaptain::storage::StateStore::load_existing_snapshot(&fixture.state).unwrap();
    let target: packcaptain::domain::TargetId = fixture.targets[0].parse().unwrap();
    let preset = packcaptain::domain::PresetId::from(fixture.preset.as_str());
    assert!(session.text().contains("状态已变化，请重新确认"));
    assert_eq!(
        actual, external,
        "first confirmation must not write business state"
    );
    assert_eq!(fixture.links(0), external_links);
    session.key(b"\r");
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
    session.finish(&["执行成功"]);
    passed(
        "fresh-shared",
        "external preset, enabled skills and owned entries preserved after reopened SQLite; exact links on selected target only",
    );
}

#[test]
fn pty_changed_preview_rechecks_again_and_cancel_writes_nothing() {
    let fixture = Fixture::new(2);
    let mut session = Session::start(&fixture, "fresh-recheck-cancel");
    session.key(b"e");
    fixture.select(&mut session, &fixture.targets[..1], false);
    for skill in &fixture.skills {
        fixture.cli(&["skills", "enable", skill, "--target", &fixture.targets[0]]);
        let before =
            packcaptain::storage::StateStore::load_existing_snapshot(&fixture.state).unwrap();
        let links = fixture.links(0);
        session.key(b"\r");
        assert!(session.text().contains("状态已变化，请重新确认"));
        assert_eq!(
            packcaptain::storage::StateStore::load_existing_snapshot(&fixture.state).unwrap(),
            before
        );
        assert_eq!(fixture.links(0), links);
    }
    let before = fixture.snapshot();
    session.key(b"\x1b");
    assert_eq!(fixture.snapshot(), before);
    session.finish(&["已取消，未写入"]);
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
    session.key(b"u");
    session.search("Skill001");
    session.key(b" ");
    session.key(b"\r");
    fixture.select(&mut session, &fixture.targets[..1], true);
    assert!(session.text().contains("影响预览"));
    fixture.cli(&[
        "skills",
        "disable",
        &fixture.skills[0],
        "--target",
        &fixture.targets[0],
    ]);
    let before = packcaptain::storage::StateStore::load_existing_snapshot(&fixture.state).unwrap();
    let links = fixture.links(0);
    session.key(b"\r");
    assert_eq!(
        packcaptain::storage::StateStore::load_existing_snapshot(&fixture.state).unwrap(),
        before
    );
    assert_eq!(fixture.links(0), links);
    assert!(session.text().contains("手动关闭冲突策略"));
    session.key(b"\r");
    session.key(b"\r");
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
    session.finish(&["执行成功"]);
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
    session.key(b"d");
    fixture.select(&mut session, &fixture.targets[..1], false);
    let before = packcaptain::storage::StateStore::load_existing_snapshot(&fixture.state).unwrap();
    let path = fixture.projects[0].join(format!(".claude/skills/pc-{}", fixture.skills[0]));
    fs::remove_file(&path).unwrap();
    fs::create_dir(&path).unwrap();
    fs::write(path.join("foreign"), "do not touch").unwrap();
    session.key(b"\r");
    assert_eq!(
        packcaptain::storage::StateStore::load_existing_snapshot(&fixture.state).unwrap(),
        before
    );
    assert_eq!(
        fs::read_to_string(path.join("foreign")).unwrap(),
        "do not touch"
    );
    session.finish(&["部分失败", "失败 1"]);
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
    session.key(b"d");
    fixture.select(&mut session, &fixture.targets[..1], false);
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
    session.key(b"\r");
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
    session.finish(&["存在未完成操作，需先完成恢复后重新预览"]);
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
    let selected = fixture.targets[..2].to_vec();
    fixture.edit("members-add", "Skill001", &selected, false);
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
    fixture.edit("members-remove", "Skill000", &selected, false);
    fixture.assert_members(&fixture.skills[1..]);
    for index in 0..2 {
        let links = fixture.links(index);
        assert_eq!(links.len(), 1);
        assert!(links[0].1.ends_with("skill-001"));
    }
    assert_eq!(row(&before), row(&fixture.snapshot()));
    passed(
        "members-and-scope",
        "added Skill001 and removed Skill000 on two explicit targets; third target state and links unchanged",
    );
}

#[test]
fn pty_cancel_preserves_entire_semantic_state_and_links() {
    let fixture = Fixture::new(2);
    fixture.enable_all();
    let before = fixture.snapshot();
    fixture.edit("cancel-edit", "Skill001", &fixture.targets[..2], true);
    assert_eq!(fixture.snapshot(), before);
    passed(
        "cancel",
        "preset definitions, all target states and all symlink names/destinations exactly equal before and after canceled confirmation",
    );
}

#[test]
fn pty_partial_failure_identifies_failed_target_and_success_count() {
    let fixture = Fixture::new(1);
    let mut session = Session::start(&fixture, "partial-failure");
    session.key(b"e");
    fixture.select(&mut session, &fixture.targets[..2], false);
    assert!(session.text().contains("影响预览"));
    let blocked = fixture.projects[1].join(".claude");
    fs::write(&blocked, "foreign blocker").unwrap();
    session.key(b"\r");
    session.finish(&["部分失败", "成功 1", "失败 1", &fixture.targets[1]]);
    assert_eq!(fixture.links(0).len(), 1);
    assert_eq!(fs::read_to_string(blocked).unwrap(), "foreign blocker");
    let targets = fixture.cli(&["targets", "list"]);
    let successful = targets
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["id"] == fixture.targets[0])
        .unwrap();
    assert_eq!(
        successful["state"]["enabled_skills"],
        json!([fixture.skills[0]])
    );
    passed(
        "partial-failure",
        "PTY displays partial failure with success=1 failure=1 and failed target ID; successful target link applied and foreign blocker preserved",
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
    let last = name(89);
    let mut session = Session::start(&fixture, "long-list");
    session.wait_screen("home", 0, |screen| screen.text().contains("Presets"));
    for _ in 0..2 {
        session.key_observed(b"\x1b[B", "home-down", |_| true);
    }
    session.key_observed(b"\r", "inventory", |screen| {
        screen.text().contains("Skills 状态") && screen.selected().contains(name(0))
    });
    for index in 1..90 {
        session.key_observed(b"\x1b[B", name(index), |screen| {
            screen.selected().contains(name(index))
        });
    }
    for _ in 0..6 {
        session.key_observed(b"\x1b[B", "bottom-boundary", |screen| {
            screen.selected().contains(last)
        });
    }
    assert!(
        session.screen().selected().contains(last),
        "last skill was not rendered"
    );
    for index in (0..89).rev() {
        session.key_observed(b"\x1b[A", name(index), |screen| {
            screen.selected().contains(name(index))
        });
    }
    for _ in 0..6 {
        session.key_observed(b"\x1b[A", "top-boundary", |screen| {
            screen.selected().contains(name(0))
        });
    }
    assert!(
        session.screen().selected().contains(name(0)),
        "did not scroll back to top"
    );
    session.key_observed(b"/", "search-start", |screen| {
        screen.text().contains("搜索过滤")
    });
    let mut filter = String::new();
    for byte in b"Skill089" {
        filter.push(*byte as char);
        session.key_observed(&[*byte], &format!("filter-{filter}"), |screen| {
            screen.text().contains(&format!("搜索过滤：{filter}（"))
        });
    }
    session.key_observed(b"\r", "search-confirmed", |screen| {
        screen.selected().contains("Skill089") && !screen.text().contains("搜索过滤")
    });
    assert!(session.screen().selected().contains("Skill089"));
    session.key_observed(b"\x1b", "back-home", |screen| {
        screen.text().contains("Presets")
    });
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
    session.key(b"d");
    fixture.select(&mut session, &fixture.targets[..1], false);
    let before = packcaptain::storage::StateStore::load_existing_snapshot(&fixture.state).unwrap();
    let links = fixture.links(0);
    let config = fixture.projects[0].join(".claude/settings.local.json");
    fs::write(&config, "{broken-json").unwrap();
    session.key(b"\r");
    assert!(session.screen().text().contains("state is unknown"));
    assert_eq!(
        packcaptain::storage::StateStore::load_existing_snapshot(&fixture.state).unwrap(),
        before
    );
    assert_eq!(fixture.links(0), links);
    assert_eq!(fs::read(&config).unwrap(), b"{broken-json");
    fs::write(&config, "{}").unwrap();
    session.key(b"\r");
    assert!(
        fixture.links(0).is_empty(),
        "pending command must remain available after config error"
    );
    assert!(session.screen().text().contains("执行成功"));
    session.finish(&[]);
    passed(
        "config-error-pending",
        "parse error keeps pending Command and stored snapshot/links; fixing config allows the same confirmation to converge",
    );
}
