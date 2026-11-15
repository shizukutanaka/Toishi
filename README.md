# Toishi (砥石)

**Mine your verified human git history. Distill recurring change patterns
into Agent Skills.**

Toishi is a local-first Rust CLI that reads the fix/feature commits of a
git repository — changes a human already verified by committing them —
finds the change patterns that recur, and writes them out as
[Agent Skills](https://agentskills.io) (`SKILL.md`) plus a JSONL skill
registry with confidence scores.

```console
$ toishi mine --repo ~/src/myproject
scanned 370 commits (32 fix, 243 feat, 95 other)
found 272 distinct change patterns; 1 met support >= 2
emitted 1 skill(s) to toishi-out
  - add-aria-labels-and-keyboard-accessibility-to-node-components
```

## Why "verified human commits"?

Agent-session learning (e.g. Anthropic's *Dreaming* for managed agents)
extracts patterns from **agent session logs** and applies them
automatically. That has a known weakness: self-generated skills are
prompt-sensitive — small wording changes flip whether they help.

Toishi starts from the opposite end: **your git history**. A `fix:`
commit is a problem→solution pair that a human diagnosed, implemented,
and accepted. Patterns that recur across such commits are procedures
worth keeping — and Toishi distills them into deterministic,
reviewable skill files instead of silently rewriting an agent's memory.

| | Toishi | Session-learning systems |
|---|---|---|
| Source | Verified human commits | Agent session logs |
| Trust | Human-validated changes | Self-generated summaries |
| Output | `SKILL.md` + `registry.jsonl`, byte-identical per input | Internal notes, auto-applied |
| Network | **None** | Cloud service |

## Install

Requires a Rust toolchain and a `git` binary on `PATH`.

```console
$ cargo build --release
$ cp target/release/toishi ~/.local/bin/
```

There are no dependencies — `Cargo.toml` is empty below the package
metadata, and the binary talks only to your local repositories.

## Usage

```console
# Mine a repository (default: patterns need >= 3 commits)
toishi mine --repo <path> [--out <dir>] [--support <n>]

# Show what was mined
toishi list [--out <dir>]
```

Output layout:

```
toishi-out/
├── registry.jsonl            # one record per skill
├── rejected.jsonl            # gate rejections with reasons (if any)
└── <skill-name>/
    └── SKILL.md              # frontmatter (name, description) + pattern body
```

Each `SKILL.md` carries a trigger-rich description (built from the
commits' imperative subjects, identifiers in the diff, and a path
glob), a normalized representative diff — string/number literals become
`⟨S⟩`/`⟨N⟩`, secret-looking tokens become `⟨R⟩` — and its source
commits, so every skill is auditable back to the history it came from.

### Zero skills is a valid result

Small, single-author repositories have sparse pattern recurrence (the
mining literature measures ~8–21% within-project recurrence for fixes).
If nothing reaches the support threshold, Toishi says so, explains why,
and exits 0:

```console
$ toishi mine --repo ~/src/small-repo
scanned 41 commits (9 fix, 12 feat, 20 other)
found 18 distinct change patterns; 0 met support >= 3
0 skills emitted (not an error):
  reason: the largest recurring pattern has support 2, below the
  threshold 3; ... try --support 2 or a larger history
```

### Exit codes

| Code | Meaning |
|---|---|
| 0 | Success (including zero-skill results) |
| 1 | Runtime error (bad repository, git failure, I/O) |
| 2 | Usage error |
| 3 | Candidates existed but the security gate rejected all of them |

## Guarantees

- **Deterministic**: same repository + same configuration →
  byte-identical output. Git is invoked with a pinned environment
  (`GIT_CONFIG_NOSYSTEM=1`, `LC_ALL=C`, fixed `-c` flags), processing
  order is hash-sorted, and output dates come from commit timestamps —
  never from the wall clock. Verified by the integration test suite.
- **Local-first**: no network I/O. Your history never leaves the
  machine.
- **Zero-dependency**: the Rust standard library is the whole supply
  chain.
- **Safety gate**: commit text is untrusted (it lands inside files an
  agent will read as instructions), so subjects are stripped of emails
  and @mentions, secret-looking tokens (Shannon entropy ≥ 3.5) are
  redacted, and instruction-override phrases (EN/JA, with full-width
  and zero-width evasion handling) reject the whole candidate — with
  the reason recorded in `rejected.jsonl`. A blocklist has principled
  limits (homoglyphs, encodings); see `spec.md` §4. Review emitted
  skills before adopting them.

## Limitations

- **Sparse histories yield few or zero skills.** This is expected on
  small repositories; lower `--support` or wait for history to accrue.
  Cross-repository mining is planned (v0.2).
- **Line diffs, not ASTs.** Renamed identifiers and moved code can
  split a pattern. tree-sitter AST diffing is planned (v0.2).
- **Classification is heuristic** outside Conventional Commits repos
  (EN/JA keyword fallback).
- **Confidence is a frequency prior** (`support / (support + 2)`) in
  v0.1; decay/re-verification arrive in later versions.

## Roadmap

- **v0.1.x** — Claude Code plugin + PostToolUse/Stop hook distribution;
  event-driven staleness (dependency major bumps).
- **v0.2** — cross-repository mining; `verify` subcommand; tree-sitter
  AST diffs; optional, explicitly-labeled local-LLM description
  refinement (deterministic core remains the default).

The full specification lives in [`spec.md`](spec.md).

## License

MIT — see [`LICENSE`](LICENSE).
