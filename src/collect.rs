//! collect stage: read commit history from a local repository by invoking
//! the system `git` binary as a subprocess.
//!
//! Determinism (spec §1 / §6): git's output depends on the user's gitconfig,
//! locale, and platform, so every invocation pins the environment:
//! `GIT_CONFIG_NOSYSTEM=1`, `LC_ALL=C`, and the `-c` flags
//! `core.quotePath=false`, `diff.mnemonicPrefix=false`,
//! `core.autocrlf=false`, `i18n.logOutputEncoding=UTF-8`. No network I/O is
//! performed: `git log` / `git show` operate purely on the local object
//! database.

use crate::{Commit, DiffLine};
use std::path::Path;
use std::process::Command;

/// The `-c` configuration pins applied to every git invocation.
const CONFIG_PINS: [(&str, &str); 4] = [
    ("core.quotePath", "false"),
    ("diff.mnemonicPrefix", "false"),
    ("core.autocrlf", "false"),
    ("i18n.logOutputEncoding", "UTF-8"),
];

/// Build a `git` command with the pinned environment for `repo`.
fn git_command(repo: &Path) -> Command {
    let mut cmd = Command::new("git");
    cmd.env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_ALLOW_PROTOCOL", "")
        .env("GIT_NO_LAZY_FETCH", "1")
        .env("GIT_NO_REPLACE_OBJECTS", "1")
        .env("LC_ALL", "C")
        .env("GIT_PAGER", "cat")
        .env("PAGER", "cat");
    for (key, value) in CONFIG_PINS {
        cmd.arg("-c").arg(format!("{key}={value}"));
    }
    cmd.arg("-C").arg(repo);
    cmd
}

/// Run git with `args` and return stdout as a String.
fn run_git(repo: &Path, args: &[&str]) -> Result<String, String> {
    let output = git_command(repo)
        .args(args)
        .output()
        .map_err(|e| format!("failed to execute git: {e}"))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!(
            "git {} failed: {}",
            args.first().copied().unwrap_or(""),
            stderr.trim()
        ));
    }
    // Diffs carry file content in whatever encoding the file used (Latin-1,
    // Shift_JIS, ...). Decode lossily (invalid bytes -> U+FFFD) so one legacy
    // file cannot abort mining the whole history; the mapping is deterministic.
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

/// Raw commit metadata before diff parsing.
struct CommitMeta {
    hash: String,
    timestamp: i64,
    subject: String,
}

/// List commit metadata, newest first (git log order).
fn list_commits(repo: &Path) -> Result<Vec<CommitMeta>, String> {
    // %x1e (record separator) starts each record; %x1f (unit separator)
    // delimits fields. Subjects cannot contain these control characters.
    let out = run_git(repo, &["log", "--format=%x1e%H%x1f%ct%x1f%s"])?;
    Ok(parse_metadata(&out))
}

fn parse_metadata(out: &str) -> Vec<CommitMeta> {
    let mut metas = Vec::new();
    for record in out.split('\u{1e}') {
        let record = record.trim_matches('\n');
        if record.is_empty() {
            continue;
        }
        let mut fields = record.splitn(3, '\u{1f}');
        let hash = fields.next().unwrap_or("").trim().to_string();
        let ts_str = fields.next().unwrap_or("0").trim();
        let subject = fields.next().unwrap_or("").to_string();
        if hash.is_empty() {
            continue;
        }
        metas.push(CommitMeta {
            hash,
            timestamp: ts_str.parse::<i64>().unwrap_or(0),
            subject,
        });
    }
    metas
}

/// Parse the unified diff of a single commit (`git show -U0`).
fn commit_diff(repo: &Path, hash: &str) -> Result<Vec<DiffLine>, String> {
    let out = run_git(
        repo,
        &[
            "show",
            "--format=",
            "-U0",
            "--no-color",
            "--no-ext-diff",
            "--no-textconv",
            hash,
            "--",
        ],
    )?;
    Ok(parse_diff(&out))
}

/// Parse unified-diff text into added/removed lines with their file paths.
pub fn parse_diff(diff: &str) -> Vec<DiffLine> {
    let mut lines = Vec::new();
    let mut current_path = String::new();
    for raw in diff.lines() {
        if let Some(rest) = raw.strip_prefix("+++ ") {
            current_path = strip_diff_prefix(rest);
            continue;
        }
        if raw.starts_with("--- ") || raw.starts_with("diff --git ") {
            continue;
        }
        if let Some(text) = raw.strip_prefix('+') {
            if !current_path.is_empty() {
                lines.push(DiffLine {
                    sign: '+',
                    path: current_path.clone(),
                    text: text.to_string(),
                });
            }
        } else if let Some(text) = raw.strip_prefix('-') {
            if !current_path.is_empty() {
                lines.push(DiffLine {
                    sign: '-',
                    path: current_path.clone(),
                    text: text.to_string(),
                });
            }
        }
    }
    lines
}

/// Strip the `a/` or `b/` prefix git puts on diff paths. `/dev/null` maps to
/// the empty string (the line is then dropped: a pure creation/deletion has
/// no counterpart path we can attribute patterns to reliably).
fn strip_diff_prefix(p: &str) -> String {
    let p = p.trim();
    if p == "/dev/null" {
        return String::new();
    }
    if let Some(rest) = p.strip_prefix("b/") {
        rest.to_string()
    } else if let Some(rest) = p.strip_prefix("a/") {
        rest.to_string()
    } else {
        p.to_string()
    }
}

/// Collect all commits of the repository at `repo`, each classified and with
/// its parsed diff.
pub fn collect(repo: &Path) -> Result<Vec<Commit>, String> {
    if !repo.is_dir() {
        return Err(format!(
            "repository path does not exist: {}",
            repo.display()
        ));
    }
    let metas = list_commits(repo)?;
    metas
        .into_iter()
        .map(|meta| read_commit(repo, meta))
        .collect()
}

fn read_commit(repo: &Path, meta: CommitMeta) -> Result<Commit, String> {
    let category = crate::classify::classify(&meta.subject);
    let lines = if category.is_mineable() {
        commit_diff(repo, &meta.hash)?
    } else {
        Vec::new()
    };
    Ok(Commit {
        hash: meta.hash,
        timestamp: meta.timestamp,
        subject: meta.subject,
        category,
        lines,
    })
}

pub fn check_repository(repo: &Path) -> Result<(), String> {
    run_git(repo, &["rev-parse", "--git-dir"]).map(|_| ())
}

pub fn collect_source(repo: &Path, id: &str) -> Result<Option<Commit>, String> {
    if !matches!(id.len(), 40 | 64) || !id.bytes().all(|c| c.is_ascii_hexdigit()) {
        return Err("invalid source commit ID".to_string());
    }
    let revision = format!("{id}^{{commit}}");
    let result = git_command(repo)
        .args([
            "rev-parse",
            "--verify",
            "--quiet",
            "--end-of-options",
            &revision,
        ])
        .output()
        .map_err(|e| format!("failed to execute git: {e}"))?;
    match result.status.code() {
        Some(1) => return Ok(None),
        Some(0) if result.stdout.strip_suffix(b"\n") == Some(id.as_bytes()) => {}
        Some(0) => return Ok(None),
        _ => return Err("git rev-parse failed while resolving a source commit".to_string()),
    }
    let output = run_git(
        repo,
        &["show", "-s", "--format=%x1e%H%x1f%ct%x1f%s", id, "--"],
    )?;
    let meta = parse_metadata(&output)
        .into_iter()
        .find(|meta| meta.hash == id)
        .ok_or("git returned no metadata for a source commit")?;
    read_commit(repo, meta).map(Some)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_simple_diff() {
        let diff = "\
diff --git a/src/main.rs b/src/main.rs
--- a/src/main.rs
+++ b/src/main.rs
@@ -1 +1 @@
-old line
+new line
";
        let lines = parse_diff(diff);
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0].sign, '-');
        assert_eq!(lines[0].text, "old line");
        assert_eq!(lines[0].path, "src/main.rs");
        assert_eq!(lines[1].sign, '+');
        assert_eq!(lines[1].text, "new line");
    }

    #[test]
    fn parse_diff_new_file() {
        let diff = "\
diff --git a/new.txt b/new.txt
--- /dev/null
+++ b/new.txt
@@ -0,0 +1 @@
+hello
";
        let lines = parse_diff(diff);
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0].path, "new.txt");
    }

    #[test]
    fn parse_diff_ignores_headers() {
        let diff = "\
diff --git a/a b/a
--- a/a
+++ b/a
@@ -1,1 +1,1 @@ fn header
 context line
";
        // Context lines (space prefix) and hunk headers are not collected.
        assert!(parse_diff(diff).is_empty());
    }
}
