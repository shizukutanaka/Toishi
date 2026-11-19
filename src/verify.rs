use std::fs;

use toishi::{cluster, collect, emit};

use super::{normalize_patterns, parse_options, Options};

pub(super) fn run(args: &[String]) -> u8 {
    let result = args
        .split_first()
        .filter(|(name, _)| emit::is_valid_skill_name(name))
        .ok_or_else(|| "verify requires a valid skill name".to_string())
        .and_then(|(name, rest)| parse_options(rest, "verify").map(|opts| (name, opts)));
    let (name, opts) = match result {
        Ok(parsed) => parsed,
        Err(err) => {
            eprintln!("error: {err}");
            return 2;
        }
    };
    match verify(name, &opts) {
        Ok(count) => {
            println!("verified {name}: reproduced SKILL.md from {count} source commits");
            0
        }
        Err(err) => {
            eprintln!("error: {err}");
            1
        }
    }
}

fn verify(name: &str, opts: &Options) -> Result<usize, String> {
    let dir = opts.out.join(name);
    let source_path = dir.join(emit::SOURCES_FILE);
    let source = fs::read_to_string(&source_path).map_err(|err| {
        let hint = if err.kind() == std::io::ErrorKind::NotFound {
            "; re-mine with a version of Toishi that records source commits"
        } else {
            ""
        };
        format!("cannot read {}: {err}{hint}", source_path.display())
    })?;
    let (full_source_ids, ids) = parse_sources(&source)?;
    let skill_path = dir.join("SKILL.md");
    let actual = fs::read(&skill_path)
        .map_err(|err| format!("cannot read {}: {err}", skill_path.display()))?;

    for repo in &opts.repos {
        collect::check_repository(repo).map_err(|err| format!("{}: {err}", repo.display()))?;
    }
    let mut commits = Vec::with_capacity(ids.len());
    for id in ids {
        let mut commit = None;
        for repo in &opts.repos {
            commit = collect::collect_source(repo, id)
                .map_err(|err| format!("{}: {err}", repo.display()))?;
            if commit.is_some() {
                break;
            }
        }
        commits.push(
            commit
                .ok_or_else(|| format!("source commit {id} not found in supplied repositories"))?,
        );
    }
    let (patterns, _) = normalize_patterns(&commits);
    let clusters = cluster::cluster(&patterns);
    let [matched] = clusters.as_slice() else {
        return Err("source commits do not form a single mined pattern".to_string());
    };
    if matched.support() != commits.len() {
        return Err("not every source commit contributes to the mined pattern".to_string());
    }
    let expected = emit::replay_skill(name, matched, &commits, &patterns, full_source_ids)?;
    if expected.as_bytes() != actual {
        return Err("SKILL.md does not match the replayed source commits".to_string());
    }
    Ok(commits.len())
}

fn parse_sources(source: &str) -> Result<(bool, Vec<&str>), String> {
    let mut lines = source.lines();
    let full_source_ids = match lines.next() {
        Some(emit::SOURCES_SHORT) => false,
        Some(emit::SOURCES_FULL) => true,
        _ => return Err("unsupported source manifest format".to_string()),
    };
    let ids: Vec<_> = lines.collect();
    if ids.is_empty()
        || ids.iter().any(|id| {
            !matches!(id.len(), 40 | 64)
                || !id
                    .bytes()
                    .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
        })
        || ids.windows(2).any(|pair| pair[0] >= pair[1])
    {
        return Err("invalid source manifest: expected sorted, unique full commit IDs".to_string());
    }
    Ok((full_source_ids, ids))
}
