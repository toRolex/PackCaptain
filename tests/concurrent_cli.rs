use std::collections::BTreeSet;
use std::fs::{self, OpenOptions};
use std::path::Path;
use std::process::{Child, Command, Output, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use fs2::FileExt;
use packcaptain::domain::TargetId;
use packcaptain::storage::StateStore;
use serde_json::Value;
use tempfile::TempDir;

fn cli(home: &Path, state: &Path, project: &Path, args: &[&str]) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_packcaptain"));
    command
        .env("HOME", home)
        .env("XDG_CONFIG_HOME", home.join("xdg-config"))
        .env("XDG_STATE_HOME", home.join("xdg-state"))
        .current_dir(project)
        .args([
            "--agent-home",
            home.to_str().unwrap(),
            "--state-dir",
            state.to_str().unwrap(),
            "--json",
        ])
        .args(args);
    command
}

fn data(output: Output) -> Value {
    assert!(
        output.status.success(),
        "exit={:?} stdout={} stderr={}",
        output.status.code(),
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let result: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(result["ok"], true);
    result["data"].clone()
}

fn spawn_enable(home: &Path, state: &Path, project: &Path, preset: &str, target: &str) -> Child {
    cli(
        home,
        state,
        project,
        &["presets", "enable", preset, "--target", target],
    )
    .stdout(Stdio::piped())
    .stderr(Stdio::piped())
    .spawn()
    .unwrap()
}

#[test]
fn overlapping_cli_enables_preserve_shared_member_and_both_presets() {
    for round in 1..=5 {
        let fixture = TempDir::new_in("/private/tmp").unwrap();
        let home = fixture.path().join("home");
        let state = fixture.path().join("state");
        let project = fixture.path().join("project");
        fs::create_dir_all(&project).unwrap();
        for name in ["a", "s", "b"] {
            let source = home.join(".claude/skills").join(name);
            fs::create_dir_all(&source).unwrap();
            fs::write(
                source.join("SKILL.md"),
                format!("---\nname: {name}\ndescription: concurrent fixture\n---\n"),
            )
            .unwrap();
        }
        let registered = data(
            cli(
                &home,
                &state,
                &project,
                &["projects", "add", project.to_str().unwrap()],
            )
            .output()
            .unwrap(),
        );
        let target = format!("claude-code:project:{}", registered["id"].as_str().unwrap());
        let skills = data(
            cli(&home, &state, &project, &["skills", "list"])
                .output()
                .unwrap(),
        );
        let skill_id = |name: &str| {
            skills
                .as_array()
                .unwrap()
                .iter()
                .find(|row| row["name"] == name)
                .unwrap()["id"]
                .as_str()
                .unwrap()
                .to_owned()
        };
        let a = skill_id("a");
        let s = skill_id("s");
        let b = skill_id("b");
        let mut presets = Vec::new();
        for (name, first, second) in [("A", &a, &s), ("B", &s, &b)] {
            let created = data(
                cli(
                    &home,
                    &state,
                    &project,
                    &[
                        "presets", "create", "--name", name, "--skill", first, "--skill", second,
                    ],
                )
                .output()
                .unwrap(),
            );
            presets.push(
                created["plan"]["preset_updates"]
                    .as_object()
                    .unwrap()
                    .keys()
                    .next()
                    .unwrap()
                    .clone(),
            );
        }

        let lock = OpenOptions::new()
            .read(true)
            .write(true)
            .open(state.join("packcaptain.lock"))
            .unwrap();
        lock.lock_exclusive().unwrap();
        let started = Instant::now();
        let mut child_a = spawn_enable(&home, &state, &project, &presets[0], &target);
        let mut child_b = spawn_enable(&home, &state, &project, &presets[1], &target);
        assert_ne!(child_a.id(), child_b.id());
        thread::sleep(Duration::from_millis(300));
        let a_waiting = child_a.try_wait().unwrap();
        let b_waiting = child_b.try_wait().unwrap();
        FileExt::unlock(&lock).unwrap();
        let output_a = child_a.wait_with_output().unwrap();
        let output_b = child_b.wait_with_output().unwrap();
        println!(
            "round={round} overlap_gate_ms={} A={:?} B={:?} exits={:?},{:?}",
            started.elapsed().as_millis(),
            a_waiting,
            b_waiting,
            output_a.status.code(),
            output_b.status.code()
        );
        assert!(
            a_waiting.is_none() && b_waiting.is_none(),
            "both processes must be alive at the locked barrier"
        );
        data(output_a);
        data(output_b);

        let root = project.join(".claude/skills");
        let links: Vec<_> = fs::read_dir(&root)
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .collect();
        assert_eq!(links.len(), 3, "no extra aliases");
        let sources: BTreeSet<_> = links
            .iter()
            .map(|entry| {
                assert!(
                    fs::symlink_metadata(entry)
                        .unwrap()
                        .file_type()
                        .is_symlink()
                );
                fs::canonicalize(entry).unwrap()
            })
            .collect();
        let expected: BTreeSet<_> = ["a", "s", "b"]
            .into_iter()
            .map(|name| fs::canonicalize(home.join(".claude/skills").join(name)).unwrap())
            .collect();
        assert_eq!(sources, expected);
        let snapshot = StateStore::open(&state).unwrap().load_snapshot().unwrap();
        let target_id: TargetId = target.parse().unwrap();
        let target_state = &snapshot.targets[&target_id];
        assert_eq!(target_state.presets.len(), 2);
        for preset in &presets {
            assert!(target_state.presets[&preset.as_str().into()].enabled);
        }
        let enabled: BTreeSet<_> = target_state
            .enabled_skills
            .iter()
            .map(|id| id.as_str())
            .collect();
        assert_eq!(
            enabled,
            BTreeSet::from([a.as_str(), s.as_str(), b.as_str()])
        );
        assert_eq!(snapshot.entries[&target_id].len(), 3);
    }
}

#[cfg(unix)]
struct DisableFixture {
    _temp: TempDir,
    home: std::path::PathBuf,
    state: std::path::PathBuf,
    project: std::path::PathBuf,
    target: String,
    skills: Vec<String>,
    presets: Vec<String>,
}

#[cfg(unix)]
impl DisableFixture {
    fn new() -> Self {
        let temp = TempDir::new_in("/private/tmp").unwrap();
        let home = temp.path().join("home");
        let state = temp.path().join("state");
        let project = temp.path().join("project");
        fs::create_dir_all(&project).unwrap();
        for name in ["a", "s", "b"] {
            let source = home.join(".claude/skills").join(name);
            fs::create_dir_all(&source).unwrap();
            fs::write(
                source.join("SKILL.md"),
                format!("---\nname: {name}\ndescription: disable fixture\n---\n"),
            )
            .unwrap();
        }
        let registered = data(
            cli(
                &home,
                &state,
                &project,
                &["projects", "add", project.to_str().unwrap()],
            )
            .output()
            .unwrap(),
        );
        let target = format!("claude-code:project:{}", registered["id"].as_str().unwrap());
        let rows = data(
            cli(&home, &state, &project, &["skills", "list"])
                .output()
                .unwrap(),
        );
        let skills: Vec<String> = ["a", "s", "b"]
            .into_iter()
            .map(|name| {
                rows.as_array()
                    .unwrap()
                    .iter()
                    .find(|row| row["name"] == name)
                    .unwrap()["id"]
                    .as_str()
                    .unwrap()
                    .to_owned()
            })
            .collect();
        let mut presets = Vec::new();
        for (name, first, second) in [("A", &skills[0], &skills[1]), ("B", &skills[1], &skills[2])]
        {
            let created = data(
                cli(
                    &home,
                    &state,
                    &project,
                    &[
                        "presets", "create", "--name", name, "--skill", first, "--skill", second,
                    ],
                )
                .output()
                .unwrap(),
            );
            presets.push(
                created["plan"]["preset_updates"]
                    .as_object()
                    .unwrap()
                    .keys()
                    .next()
                    .unwrap()
                    .clone(),
            );
        }
        data(
            cli(
                &home,
                &state,
                &project,
                &["presets", "enable", &presets[0], "--target", &target],
            )
            .output()
            .unwrap(),
        );
        Self {
            _temp: temp,
            home,
            state,
            project,
            target,
            skills,
            presets,
        }
    }
}

#[cfg(unix)]
fn tree_identity(
    root: &Path,
) -> std::collections::BTreeMap<std::path::PathBuf, (u64, String, Vec<u8>)> {
    use std::os::unix::fs::MetadataExt;
    let mut result = std::collections::BTreeMap::new();
    for item in walkdir::WalkDir::new(root).follow_links(false) {
        let item = item.unwrap();
        let path = item.path();
        let metadata = fs::symlink_metadata(path).unwrap();
        let (kind, bytes) = if metadata.file_type().is_symlink() {
            (
                "symlink",
                fs::read_link(path)
                    .unwrap()
                    .as_os_str()
                    .as_encoded_bytes()
                    .to_vec(),
            )
        } else if metadata.is_file() {
            ("file", fs::read(path).unwrap())
        } else {
            ("directory", Vec::new())
        };
        result.insert(
            path.strip_prefix(root).unwrap().to_owned(),
            (metadata.ino(), kind.to_owned(), bytes),
        );
    }
    result
}

#[cfg(unix)]
fn journal_count(store: &StateStore) -> i64 {
    rusqlite::Connection::open_with_flags(
        store.database_path(),
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .unwrap()
    .query_row("SELECT COUNT(*) FROM operation_journal", [], |row| {
        row.get(0)
    })
    .unwrap()
}

#[cfg(unix)]
#[test]
fn overlapping_cli_disable_and_enable_converge_in_both_lock_orders() {
    use packcaptain::domain::EntryOwnership;
    for disable_first in [true, false] {
        for round in 1..=5 {
            let f = DisableFixture::new();
            let external_before = tree_identity(&f.home);
            let lock = OpenOptions::new()
                .read(true)
                .write(true)
                .open(f.state.join("packcaptain.lock"))
                .unwrap();
            lock.lock_exclusive().unwrap();
            let started = Instant::now();
            let spawn_disable = || {
                cli(
                    &f.home,
                    &f.state,
                    &f.project,
                    &["presets", "disable", &f.presets[0], "--target", &f.target],
                )
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
                .unwrap()
            };
            let (mut disable, mut enable) = if disable_first {
                let disable = spawn_disable();
                thread::sleep(Duration::from_millis(300));
                (
                    disable,
                    spawn_enable(&f.home, &f.state, &f.project, &f.presets[1], &f.target),
                )
            } else {
                let enable = spawn_enable(&f.home, &f.state, &f.project, &f.presets[1], &f.target);
                thread::sleep(Duration::from_millis(300));
                (spawn_disable(), enable)
            };
            let disable_pid = disable.id();
            let enable_pid = enable.id();
            assert_ne!(disable_pid, enable_pid);
            thread::sleep(Duration::from_millis(300));
            let disable_waiting = disable.try_wait().unwrap();
            let enable_waiting = enable.try_wait().unwrap();
            assert!(
                disable_waiting.is_none() && enable_waiting.is_none(),
                "both CLI PIDs must be alive at the lock barrier"
            );
            FileExt::unlock(&lock).unwrap();
            let (first, second) = if disable_first {
                (disable, enable)
            } else {
                (enable, disable)
            };
            let first_output = first.wait_with_output().unwrap();
            let first_exit = first_output.status.code();
            let second_output = second.wait_with_output().unwrap();
            println!(
                "disable_round={round} disable_first={disable_first} gate_ms={} pids={disable_pid},{enable_pid} alive={disable_waiting:?},{enable_waiting:?} exits={first_exit:?},{:?}",
                started.elapsed().as_millis(),
                second_output.status.code()
            );
            let first_report = data(first_output);
            let second_report = data(second_output);
            let disable_report = if disable_first {
                &first_report
            } else {
                &second_report
            };
            let disable_effects = disable_report["plan"]["effects"].as_array().unwrap().len();
            assert_eq!(
                disable_effects,
                if disable_first { 2 } else { 1 },
                "actual disable plan must prove the intended lock order"
            );
            println!(
                "disable_round={round} disable_first={disable_first} actual_disable_effects={disable_effects}"
            );
            let root = f.project.join(".claude/skills");
            let links: Vec<_> = fs::read_dir(&root)
                .unwrap()
                .map(|item| item.unwrap().path())
                .collect();
            assert_eq!(
                links.len(),
                2,
                "links must be exactly s,b without a repair command"
            );
            let sources: BTreeSet<_> = links
                .iter()
                .map(|path| {
                    assert!(fs::symlink_metadata(path).unwrap().file_type().is_symlink());
                    fs::canonicalize(path).unwrap()
                })
                .collect();
            assert_eq!(
                sources,
                BTreeSet::from([
                    fs::canonicalize(f.home.join(".claude/skills/s")).unwrap(),
                    fs::canonicalize(f.home.join(".claude/skills/b")).unwrap()
                ])
            );
            assert_eq!(
                tree_identity(&f.home),
                external_before,
                "source contents, inode identities and real directories must be unchanged"
            );
            let store = StateStore::open(&f.state).unwrap();
            let snapshot = store.load_snapshot().unwrap();
            assert_eq!(journal_count(&store), 0);
            let target: TargetId = f.target.parse().unwrap();
            let state = &snapshot.targets[&target];
            assert_eq!(state.presets.len(), 2);
            assert!(!state.presets[&f.presets[0].as_str().into()].enabled);
            assert!(state.presets[&f.presets[1].as_str().into()].enabled);
            assert_eq!(
                state
                    .enabled_skills
                    .iter()
                    .map(|id| id.as_str())
                    .collect::<BTreeSet<_>>(),
                BTreeSet::from([f.skills[1].as_str(), f.skills[2].as_str()])
            );
            let entries = &snapshot.entries[&target];
            assert_eq!(entries.len(), 3);
            for (index, id) in f.skills.iter().enumerate() {
                let routes = &entries[&id.as_str().into()];
                assert_eq!(routes.len(), 1);
                let route = &routes[0];
                assert_eq!(route.ownership, EntryOwnership::PackCaptainLink);
                assert_eq!((route.present, route.active), (index != 0, index != 0));
                if index != 0 {
                    assert_eq!(fs::canonicalize(&route.path).unwrap(), route.source_path);
                } else {
                    assert!(fs::symlink_metadata(&route.path).is_err());
                }
            }
        }
    }
}

#[cfg(unix)]
#[test]
fn claude_project_native_and_non_owned_disable_guards_preserve_every_file() {
    use packcaptain::domain::EntryOwnership;
    for symlink in [false, true] {
        let f = DisableFixture::new();
        let native = f.project.join(".claude/skills/native");
        if symlink {
            std::os::unix::fs::symlink(f.home.join(".claude/skills/b"), &native).unwrap();
        } else {
            fs::create_dir(&native).unwrap();
            fs::write(
                native.join("SKILL.md"),
                "---\nname: native\ndescription: guard fixture\n---\n",
            )
            .unwrap();
        }
        fs::write(
            f.project.join(".claude/settings.local.json"),
            "{\"unrelated\":{\"preserve\":true}}\n",
        )
        .unwrap();
        let rows = data(
            cli(&f.home, &f.state, &f.project, &["skills", "list"])
                .output()
                .unwrap(),
        );
        let name = if symlink { "b" } else { "native" };
        let id = rows
            .as_array()
            .unwrap()
            .iter()
            .find(|row| row["name"] == name)
            .unwrap()["id"]
            .as_str()
            .unwrap();
        data(
            cli(
                &f.home,
                &f.state,
                &f.project,
                &["presets", "create", "--name", "guard-prime", "--skill", id],
            )
            .output()
            .unwrap(),
        );
        let before = StateStore::open(&f.state).unwrap().load_snapshot().unwrap();
        let target: TargetId = f.target.parse().unwrap();
        let routes = &before.entries[&target][&id.into()];
        assert_eq!(routes.len(), 1);
        assert_eq!(
            routes[0].ownership,
            if symlink {
                EntryOwnership::UnmanagedSymlink
            } else {
                EntryOwnership::DirectDirectory
            }
        );
        let home_before = tree_identity(&f.home);
        let project_before = tree_identity(&f.project);
        let state_before = tree_identity(&f.state);
        let output = cli(
            &f.home,
            &f.state,
            &f.project,
            &["skills", "disable", id, "--target", &f.target],
        )
        .output()
        .unwrap();
        assert_eq!(output.status.code(), Some(1));
        let value: Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(value["ok"], false);
        assert!(String::from_utf8_lossy(&output.stdout).contains("target-local disable for an existing Claude project directory is not verified; no files were changed"));
        assert_eq!(tree_identity(&f.home), home_before);
        assert_eq!(tree_identity(&f.project), project_before);
        assert_eq!(
            tree_identity(&f.state),
            state_before,
            "SQLite bytes, inode identities and journal files must remain unchanged"
        );
        let store = StateStore::open(&f.state).unwrap();
        assert_eq!(store.load_snapshot().unwrap(), before);
        assert_eq!(journal_count(&store), 0);
        println!(
            "native_guard symlink={symlink} exit=1 ok=false home/project/state identities unchanged"
        );
    }
}
