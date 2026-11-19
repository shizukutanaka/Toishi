//! Toishi CLI.
//!
//! Subcommands:
//! - `toishi mine --repo <path> [--repo <path> ...] [--out <dir>] [--support <n>]`
//! - `toishi list [--out <dir>]`
//!
//! Exit codes (spec §5):
//! - 0: success — including a zero-skill result, which is a first-class
//!   outcome reported with its reason (corpus size / threshold).
//! - 1: runtime error (repository unreadable, git failure, I/O failure).
//! - 2: usage error (unknown or malformed arguments).
//! - 3: candidates existed but the security gate rejected all of them.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use toishi::classify::Category;
use toishi::cluster::{self, DEFAULT_SUPPORT};
use toishi::{collect, emit, normalize, sanitize, Commit, CommitPattern, NormLine};

const DEFAULT_OUT: &str = "toishi-out";

const USAGE: &str = "\
Toishi — mine verified human git commits into Agent Skills.

USAGE:
    toishi mine --repo <path> [--repo <path> ...] [--out <dir>] [--support <n>]
    toishi list [--out <dir>]
    toishi --help

COMMANDS:
    mine    Mine local repositories and emit SKILL.md files + registry.jsonl.
    list    Show the skills recorded in <out>/registry.jsonl.

OPTIONS:
    --repo <path>    Local repository to mine (required for `mine`, repeatable).
    --out <dir>      Output directory (default: toishi-out).
    --support <n>    Minimum commits per pattern (default: 3).

EXIT CODES:
    0  success (a zero-skill result is success, with the reason reported)
    1  runtime error
    2  usage error
    3  candidates existed but the security gate rejected all of them
";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let code = run(&args);
    ExitCode::from(code)
}

fn run(args: &[String]) -> u8 {
    if args.is_empty() {
        eprintln!("{USAGE}");
        return 2;
    }
    match args[0].as_str() {
        "--help" | "-h" | "help" => {
            println!("{USAGE}");
            0
        }
        "mine" => cmd_mine(&args[1..]),
        "list" => cmd_list(&args[1..]),
        other => {
            eprintln!("error: unknown command '{other}'\n\n{USAGE}");
            2
        }
    }
}

/// Parsed options shared by subcommands.
struct Options {
    repos: BTreeSet<PathBuf>,
    out: PathBuf,
    support: usize,
}

fn parse_options(args: &[String], need_repo: bool) -> Result<Options, String> {
    let mut opts = Options {
        repos: BTreeSet::new(),
        out: PathBuf::from(DEFAULT_OUT),
        support: DEFAULT_SUPPORT,
    };
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--repo" => {
                i += 1;
                let value = args.get(i).ok_or("--repo requires a value")?;
                opts.repos.insert(PathBuf::from(value));
            }
            "--out" => {
                i += 1;
                let value = args.get(i).ok_or("--out requires a value")?;
                opts.out = PathBuf::from(value);
            }
            "--support" => {
                i += 1;
                let value = args.get(i).ok_or("--support requires a value")?;
                let n: usize = value
                    .parse()
                    .map_err(|_| format!("--support must be a positive integer, got '{value}'"))?;
                if n == 0 {
                    return Err("--support must be at least 1".to_string());
                }
                opts.support = n;
            }
            other => return Err(format!("unknown option '{other}'")),
        }
        i += 1;
    }
    if need_repo && opts.repos.is_empty() {
        return Err("--repo is required for `mine`".to_string());
    }
    Ok(opts)
}

fn cmd_mine(args: &[String]) -> u8 {
    let opts = match parse_options(args, true) {
        Ok(o) => o,
        Err(e) => {
            eprintln!("error: {e}\n\n{USAGE}");
            return 2;
        }
    };
    match mine(&opts.repos, &opts.out, opts.support) {
        Ok(code) => code,
        Err(e) => {
            eprintln!("error: {e}");
            1
        }
    }
}

/// The mining pipeline. Returns the process exit code.
fn mine(repos: &BTreeSet<PathBuf>, out: &Path, support: usize) -> Result<u8, String> {
    let mut unique = BTreeMap::<String, Commit>::new();
    for repo in repos {
        let history = collect::collect(repo).map_err(|err| format!("{}: {err}", repo.display()))?;
        for commit in history {
            unique.entry(commit.hash.clone()).or_insert(commit);
        }
    }
    let commits: Vec<Commit> = unique.into_values().collect();
    let total = commits.len();
    let mineable: Vec<usize> = commits
        .iter()
        .enumerate()
        .filter(|(_, c)| c.category.is_mineable())
        .map(|(i, _)| i)
        .collect();

    // sanitize -> normalize -> fingerprint, per mineable commit.
    // Commits are processed in ascending hash order so that clustering is
    // independent of git log's traversal order.
    let mut order: Vec<usize> = mineable.clone();
    order.sort_by(|&a, &b| commits[a].hash.cmp(&commits[b].hash));
    let mut patterns: Vec<CommitPattern> = Vec::new();
    let mut redacted_lines = 0usize;
    for &idx in &order {
        let commit = &commits[idx];
        let mut lines: Vec<NormLine> = Vec::new();
        let mut fps = BTreeSet::new();
        for dl in &commit.lines {
            let (clean, was_redacted) = sanitize::sanitize_line(&dl.text);
            if was_redacted {
                redacted_lines += 1;
            }
            if let Some(normalized) = normalize::normalize_line(&clean) {
                let key = format!("{}{}", dl.sign, normalized);
                let fp = normalize::fingerprint(&key);
                fps.insert(fp);
                lines.push(NormLine {
                    sign: dl.sign,
                    path: dl.path.clone(),
                    text: normalized,
                    fingerprint: fp,
                });
            }
        }
        patterns.push(CommitPattern {
            commit_index: idx,
            fingerprints: fps,
            lines,
        });
    }

    let clusters = cluster::cluster(&patterns);
    let cands = cluster::candidates(&clusters, support);
    let (skills, rejections) = emit::build_skills(&cands, &commits, &patterns, repos.len() > 1);
    emit::write_outputs(out, &skills, &rejections)?;

    // Report. Everything printed is derived from the repository and the
    // configuration only — no wall clock, no randomness.
    let fix_count = commits
        .iter()
        .filter(|c| c.category == Category::Fix)
        .count();
    let feat_count = commits
        .iter()
        .filter(|c| c.category == Category::Feat)
        .count();
    if repos.len() > 1 {
        println!(
            "mined a combined history of {} local repositories",
            repos.len()
        );
    }
    println!(
        "scanned {total} commits ({fix_count} fix, {feat_count} feat, {} other)",
        total - fix_count - feat_count
    );
    println!(
        "found {} distinct change patterns; {} met support >= {support}",
        clusters.len(),
        cands.len()
    );
    if redacted_lines > 0 {
        println!("redacted secret-like tokens in {redacted_lines} diff lines");
    }
    for rejection in &rejections {
        println!(
            "gate rejected '{}': {}",
            rejection.candidate, rejection.reason
        );
    }

    if skills.is_empty() {
        if !cands.is_empty() {
            // Candidates existed; the gate rejected every one of them.
            println!(
                "0 skills emitted: all {} candidate(s) were rejected by the security gate \
                 (see rejected.jsonl)",
                cands.len()
            );
            return Ok(3);
        }
        // Zero skills is a first-class result: explain why.
        let largest = clusters.iter().map(|c| c.support()).max().unwrap_or(0);
        println!("0 skills emitted (not an error):");
        if mineable.is_empty() {
            println!(
                "  reason: no fix/feat commits were found in {total} commits; \
                 Toishi mines verified fix/feature changes only"
            );
        } else if largest == 0 {
            println!(
                "  reason: {} fix/feat commits had no parseable diff lines",
                mineable.len()
            );
        } else {
            println!(
                "  reason: the largest recurring pattern has support {largest}, \
                 below the threshold {support}; sparse recurrence is expected on \
                 small repositories — try --support {} or a larger history",
                support.saturating_sub(1).max(1)
            );
        }
        return Ok(0);
    }

    println!("emitted {} skill(s) to {}", skills.len(), out.display());
    let mut names: Vec<&str> = skills.iter().map(|s| s.name.as_str()).collect();
    names.sort_unstable();
    for name in names {
        println!("  - {name}");
    }
    Ok(0)
}

fn cmd_list(args: &[String]) -> u8 {
    let opts = match parse_options(args, false) {
        Ok(o) => o,
        Err(e) => {
            eprintln!("error: {e}\n\n{USAGE}");
            return 2;
        }
    };
    let registry_path = opts.out.join("registry.jsonl");
    let content = match std::fs::read_to_string(&registry_path) {
        Ok(c) => c,
        Err(e) => {
            eprintln!(
                "error: cannot read {}: {e}\n(hint: run `toishi mine` first)",
                registry_path.display()
            );
            return 1;
        }
    };
    let entries = emit::parse_registry(&content);
    if entries.is_empty() {
        println!("no skills recorded in {}", registry_path.display());
        return 0;
    }
    println!(
        "{:<40} {:>10} {:>12} {:>9} {:>6} {:>8}",
        "SKILL", "CONFIDENCE", "LAST VERIFIED", "STABILITY", "USED", "SUCCESS"
    );
    for entry in &entries {
        println!(
            "{:<40} {:>10} {:>12} {:>9} {:>6} {:>8}",
            entry.skill,
            entry.confidence,
            entry.last_verified,
            entry.stability,
            entry.usage_count,
            entry.success_rate
        );
    }
    println!("{} skill(s)", entries.len());
    0
}
