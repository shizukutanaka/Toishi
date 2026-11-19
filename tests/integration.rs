//! Integration tests: build real git repositories with a repeated fix
//! pattern, run the `toishi` binary, and check the emitted skills —
//! including byte-identical output across two runs (the determinism
//! guarantee).

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

fn git(dir: &Path, args: &[&str], date: &str) {
    let status = Command::new("git")
        .args(args)
        .current_dir(dir)
        .env("GIT_AUTHOR_DATE", date)
        .env("GIT_COMMITTER_DATE", date)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("LC_ALL", "C")
        .status()
        .expect("git should run");
    assert!(status.success(), "git {args:?} failed");
}

fn write(dir: &Path, name: &str, content: &str) {
    fs::write(dir.join(name), content).unwrap();
}

fn commit_all(dir: &Path, message: &str, date: &str) {
    git(dir, &["add", "-A"], date);
    git(dir, &["commit", "-q", "-m", message], date);
}

/// Build a repository where the same fix shape recurs in three commits:
/// each commit changes `timeout` and `retries` (which normalize to the
/// same lines) plus one file-specific line. Pairwise Jaccard of the
/// normalized sets is exactly 0.5, so the three commits form one cluster
/// with support 3.
fn build_pattern_repo(dir: &Path) {
    fs::create_dir_all(dir).unwrap();
    git(dir, &["init", "-q", "-b", "main"], "2026-01-01T00:00:00Z");
    git(
        dir,
        &["config", "user.email", "dev@example.com"],
        "2026-01-01T00:00:00Z",
    );
    git(dir, &["config", "user.name", "Dev"], "2026-01-01T00:00:00Z");

    for (file, mode) in [("a.cfg", "alpha"), ("b.cfg", "beta"), ("c.cfg", "gamma")] {
        write(
            dir,
            file,
            &format!("timeout = 10\nretries = 1\nmode = {mode}\n"),
        );
    }
    commit_all(dir, "chore: initial import", "2026-01-01T00:00:00Z");

    let days = [
        "2026-01-02T00:00:00Z",
        "2026-01-03T00:00:00Z",
        "2026-01-04T00:00:00Z",
    ];
    for (i, (file, mode)) in [("a.cfg", "alpha"), ("b.cfg", "beta"), ("c.cfg", "gamma")]
        .iter()
        .enumerate()
    {
        write(
            dir,
            file,
            &format!("timeout = 20\nretries = 2\nmode = {mode}-v2\n"),
        );
        commit_all(dir, "fix: raise timeout and retries", days[i]);
    }
}

/// Read a directory tree into a map of relative path -> bytes.
fn read_tree(root: &Path) -> BTreeMap<String, Vec<u8>> {
    let mut map = BTreeMap::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        for entry in fs::read_dir(&dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                stack.push(path);
            } else {
                let rel = path
                    .strip_prefix(root)
                    .unwrap()
                    .to_string_lossy()
                    .replace('\\', "/");
                map.insert(rel, fs::read(&path).unwrap());
            }
        }
    }
    map
}

fn run_mine(repo: &Path, out: &Path, extra: &[&str]) -> std::process::Output {
    let mut args = vec![
        "mine",
        "--repo",
        repo.to_str().unwrap(),
        "--out",
        out.to_str().unwrap(),
    ];
    args.extend_from_slice(extra);
    Command::new(env!("CARGO_BIN_EXE_toishi"))
        .args(&args)
        .output()
        .expect("toishi should run")
}

fn run_verify(out: &Path, repos: &[&Path]) -> std::process::Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_toishi"));
    command
        .args(["verify", "raise-timeout-and-retries", "--out"])
        .arg(out);
    for repo in repos {
        command.arg("--repo").arg(repo);
    }
    command.output().expect("toishi should run")
}

fn temp_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("toishi-it-{tag}-{}", std::process::id()));
    if dir.exists() {
        fs::remove_dir_all(&dir).unwrap();
    }
    fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn mines_a_skill_and_is_deterministic() {
    let base = temp_dir("pattern");
    let repo = base.join("repo");
    build_pattern_repo(&repo);

    let out1 = base.join("out1");
    let result = run_mine(&repo, &out1, &[]);
    assert!(
        result.status.success(),
        "mine failed: {}",
        String::from_utf8_lossy(&result.stderr)
    );
    let stdout = String::from_utf8_lossy(&result.stdout).to_string();
    assert!(stdout.contains("emitted 1 skill(s)"), "stdout: {stdout}");

    // Exactly one skill directory plus the registry.
    let tree1 = read_tree(&out1);
    let skill_files: Vec<&String> = tree1.keys().filter(|k| k.ends_with("SKILL.md")).collect();
    assert_eq!(
        skill_files.len(),
        1,
        "tree: {:?}",
        tree1.keys().collect::<Vec<_>>()
    );
    assert!(tree1.contains_key("registry.jsonl"));

    let skill_md = String::from_utf8(tree1[skill_files[0]].clone()).unwrap();
    assert!(skill_md.contains("name: raise-timeout-and-retries"));
    assert!(skill_md.contains("Support: 3 commits"));
    // last_verified is the newest source commit's date (2026-01-04).
    assert!(skill_md.contains("Last verified: 2026-01-04"));

    let registry = String::from_utf8(tree1["registry.jsonl"].clone()).unwrap();
    assert!(registry.contains("\"confidence\":0.600"));
    assert!(registry.contains("\"last_verified\":\"2026-01-04\""));

    // Determinism: a second run produces byte-identical output.
    let out2 = base.join("out2");
    let result2 = run_mine(&repo, &out2, &[]);
    assert!(result2.status.success());
    let tree2 = read_tree(&out2);
    assert_eq!(tree1, tree2, "output must be byte-identical across runs");

    // `toishi list` shows the mined skill.
    let list = Command::new(env!("CARGO_BIN_EXE_toishi"))
        .args(["list", "--out", out1.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(list.status.success());
    let list_out = String::from_utf8_lossy(&list.stdout).to_string();
    assert!(
        list_out.contains("raise-timeout-and-retries"),
        "list: {list_out}"
    );

    fs::remove_dir_all(&base).ok();
}

#[test]
fn verifies_recorded_sources_without_requiring_them_on_head() {
    let base = temp_dir("verify");
    let repo = base.join("repo");
    build_pattern_repo(&repo);
    let out = base.join("out");
    let result = run_mine(&repo, &out, &[]);
    assert!(result.status.success(), "{result:?}");
    let original = read_tree(&out);

    git(
        &repo,
        &["switch", "--detach", "HEAD~3"],
        "2026-01-05T00:00:00Z",
    );
    let replay = run_verify(&out, &[&repo]);
    assert!(replay.status.success(), "{replay:?}");
    assert!(String::from_utf8_lossy(&replay.stdout)
        .contains("reproduced SKILL.md from 3 source commits"));
    assert_eq!(original, read_tree(&out));

    let skill_dir = out.join("raise-timeout-and-retries");
    write(&skill_dir, "SKILL.md", "edited skill\n");
    let replay = run_verify(&out, &[&repo]);
    assert_eq!(replay.status.code(), Some(1), "{replay:?}");
    assert!(String::from_utf8_lossy(&replay.stderr).contains("does not match"));

    write(
        &skill_dir,
        "sources.txt",
        "toishi-sources-v1 short\n--all\n",
    );
    let replay = run_verify(&out, &[&repo]);
    assert_eq!(replay.status.code(), Some(1), "{replay:?}");
    assert!(String::from_utf8_lossy(&replay.stderr).contains("invalid source manifest"));

    fs::remove_file(skill_dir.join("sources.txt")).unwrap();
    let replay = run_verify(&out, &[&repo]);
    assert_eq!(replay.status.code(), Some(1), "{replay:?}");
    assert!(String::from_utf8_lossy(&replay.stderr).contains("re-mine"));
    fs::remove_dir_all(&base).ok();
}

#[test]
fn mines_across_repositories_without_counting_shared_history_twice() {
    let base = temp_dir("cross-repo");
    let repo = base.join("repo");
    build_pattern_repo(&repo);
    let second = base.join("second");
    git(
        &base,
        &[
            "clone",
            "-q",
            repo.to_str().unwrap(),
            second.to_str().unwrap(),
        ],
        "2026-01-05T00:00:00Z",
    );
    let overlap = base.join("overlap");
    let result = run_mine(
        &repo,
        &overlap,
        &[
            "--repo",
            repo.to_str().unwrap(),
            "--repo",
            second.to_str().unwrap(),
            "--support",
            "4",
        ],
    );
    assert!(result.status.success(), "{result:?}");
    assert_eq!(
        fs::read_to_string(overlap.join("registry.jsonl")).unwrap(),
        ""
    );

    let commit = |dir: &Path, subject: &str, date: &str| {
        git(
            dir,
            &[
                "-c",
                "user.email=dev@example.com",
                "-c",
                "user.name=Dev",
                "commit",
                "-qm",
                subject,
            ],
            date,
        );
    };
    for (dir, name, mode) in [(&repo, "d.cfg", "delta"), (&second, "e.cfg", "echo")] {
        write(
            dir,
            name,
            &format!("timeout = 10\nretries = 1\nmode = {mode}\n"),
        );
        git(dir, &["add", name], "2026-01-05T00:00:00Z");
        commit(dir, "chore: add another fixture", "2026-01-05T00:00:00Z");
        write(
            dir,
            name,
            &format!("timeout = 20\nretries = 2\nmode = {mode}-v2\n"),
        );
        git(dir, &["add", name], "2026-01-06T00:00:00Z");
        commit(
            dir,
            "fix: raise timeout and retries",
            "2026-01-06T00:00:00Z",
        );
    }

    // Each diverged history is below the support threshold on its own.
    let out = base.join("combined");
    let result = run_mine(
        &repo,
        &out,
        &["--repo", second.to_str().unwrap(), "--support", "5"],
    );
    assert!(result.status.success(), "{result:?}");
    let skill = fs::read_to_string(out.join("raise-timeout-and-retries/SKILL.md")).unwrap();
    assert!(skill.contains("Support: 5 commits"), "{skill}");
    let head = Command::new("git")
        .args(["rev-parse", "HEAD"])
        .current_dir(&second)
        .output()
        .unwrap();
    let head = String::from_utf8(head.stdout).unwrap();
    assert!(skill.contains(&format!("`{}`", head.trim())), "{skill}");
    assert!(!skill.contains(&*base.to_string_lossy()), "{skill}");
    let reverse = base.join("reverse");
    let result = run_mine(
        &second,
        &reverse,
        &["--repo", repo.to_str().unwrap(), "--support", "5"],
    );
    assert!(result.status.success(), "{result:?}");
    assert_eq!(read_tree(&out), read_tree(&reverse));
    let replay = run_verify(&out, &[&second, &repo]);
    assert!(replay.status.success(), "{replay:?}");
    assert!(String::from_utf8_lossy(&replay.stdout)
        .contains("reproduced SKILL.md from 5 source commits"));
    let replay = run_verify(&out, &[&repo]);
    assert_eq!(replay.status.code(), Some(1), "{replay:?}");
    assert!(String::from_utf8_lossy(&replay.stderr).contains("not found in supplied repositories"));
    fs::remove_dir_all(&base).ok();
}

#[test]
fn bad_repository_fails_combined_mining_without_writing_outputs() {
    let base = temp_dir("cross-repo-invalid");
    let repo = base.join("repo");
    build_pattern_repo(&repo);
    let out = base.join("combined");
    let missing = base.join("z-missing");
    let result = run_mine(&repo, &out, &["--repo", missing.to_str().unwrap()]);
    assert_eq!(result.status.code(), Some(1), "{result:?}");
    assert!(String::from_utf8_lossy(&result.stderr).contains("z-missing"));
    assert!(!out.exists());
    fs::remove_dir_all(&base).ok();
}

#[test]
fn zero_skills_is_success_with_reason() {
    let base = temp_dir("empty");
    let repo = base.join("repo");
    fs::create_dir_all(&repo).unwrap();
    git(&repo, &["init", "-q", "-b", "main"], "2026-01-01T00:00:00Z");
    git(
        &repo,
        &["config", "user.email", "dev@example.com"],
        "2026-01-01T00:00:00Z",
    );
    git(
        &repo,
        &["config", "user.name", "Dev"],
        "2026-01-01T00:00:00Z",
    );
    write(&repo, "x.txt", "hello\n");
    commit_all(&repo, "fix: one-off tweak", "2026-01-01T00:00:00Z");

    let out = base.join("out");
    let result = run_mine(&repo, &out, &[]);
    assert_eq!(result.status.code(), Some(0));
    let stdout = String::from_utf8_lossy(&result.stdout).to_string();
    assert!(stdout.contains("0 skills emitted"), "stdout: {stdout}");
    assert!(stdout.contains("reason:"), "stdout: {stdout}");

    fs::remove_dir_all(&base).ok();
}

#[test]
fn support_threshold_is_configurable() {
    let base = temp_dir("support");
    let repo = base.join("repo");
    build_pattern_repo(&repo);

    // With --support 4 the support-3 pattern no longer qualifies.
    let out = base.join("out");
    let result = run_mine(&repo, &out, &["--support", "4"]);
    assert_eq!(result.status.code(), Some(0));
    let stdout = String::from_utf8_lossy(&result.stdout).to_string();
    assert!(stdout.contains("0 skills emitted"), "stdout: {stdout}");

    fs::remove_dir_all(&base).ok();
}

#[test]
fn usage_errors_exit_2() {
    let result = Command::new(env!("CARGO_BIN_EXE_toishi"))
        .args(["mine"])
        .output()
        .unwrap();
    assert_eq!(result.status.code(), Some(2));
    let result = Command::new(env!("CARGO_BIN_EXE_toishi"))
        .args(["frobnicate"])
        .output()
        .unwrap();
    assert_eq!(result.status.code(), Some(2));
}

#[test]
fn non_utf8_diff_does_not_abort_mining() {
    let base = temp_dir("latin1");
    let repo = base.join("repo");
    build_pattern_repo(&repo);

    // A legacy-encoded (Latin-1) file: its diff is not valid UTF-8.
    fs::write(repo.join("legacy.txt"), b"caf\xe9 = 1\n").unwrap();
    commit_all(&repo, "fix: legacy encoding file", "2026-01-05T00:00:00Z");

    let out = base.join("out");
    let result = run_mine(&repo, &out, &[]);
    assert_eq!(
        result.status.code(),
        Some(0),
        "stderr: {}",
        String::from_utf8_lossy(&result.stderr)
    );
    let stdout = String::from_utf8_lossy(&result.stdout).to_string();
    assert!(stdout.contains("scanned 5 commits"), "stdout: {stdout}");
    assert!(stdout.contains("emitted 1 skill(s)"), "stdout: {stdout}");

    // Lossy decoding is deterministic: a second run is byte-identical.
    let out2 = base.join("out2");
    assert!(run_mine(&repo, &out2, &[]).status.success());
    assert_eq!(read_tree(&out), read_tree(&out2));

    fs::remove_dir_all(&base).ok();
}
