# Toishi (砥石) — v0.1 Specification

Status: v0.1, implemented. This spec incorporates the pre-implementation
design review (2026-06-10): the git environment pinning (§1), configurable
support with a zero-skill acceptance criterion (§2), trigger-bearing
descriptions (§2.3), the scoped confidence lifecycle (§3), the documented
security-gate limits (§4), distribution (§7), and the competitive
boundary against agent-session learning (§8).

Toishi mines **verified human git commits** — fix/feature commits are
problem→solution pairs a human already validated by committing them — and
distills recurring change patterns into Agent Skills (`SKILL.md`).

Design invariants:

- **Local-first**: no network I/O, ever. All input comes from the local
  repository's object database via the system `git` binary.
- **Deterministic**: the same repository state + the same configuration
  produces byte-identical output. Wall-clock time is never used in
  output; every date in the output derives from commit metadata.
- **Zero-dependency**: the Rust standard library is the entire supply
  chain. `Cargo.toml` declares no dependencies.

## 1. collect

History is read by invoking the system `git` binary as a subprocess
(`git log` for metadata, `git show -U0` per mineable commit for diffs).

Because porcelain/plumbing output varies with the user's gitconfig,
locale, and platform, every invocation pins its environment:

- environment: `GIT_CONFIG_NOSYSTEM=1`, `LC_ALL=C`
- flags: `-c core.quotePath=false -c diff.mnemonicPrefix=false
  -c core.autocrlf=false -c i18n.logOutputEncoding=UTF-8`

Commit metadata is `%H` (hash), `%ct` (committer time, Unix seconds), and
`%s` (subject), separated by control characters (`0x1E`/`0x1F`) that
cannot appear in a subject. Only fix/feat commits (see §2.1) have their
diffs collected.

Git output is decoded as UTF-8 lossily: bytes that are not valid UTF-8
(files in Latin-1, Shift_JIS, ...) become U+FFFD instead of aborting the
run. The mapping is deterministic, so §6 still holds.

## 2. Pipeline

### 2.1 classify

A commit is labeled **fix**, **feat**, or **other**:

1. A Conventional Commits type prefix (`fix:`, `feat(scope):`, `fix!:`)
   decides directly; any other conventional type is *other*.
2. Otherwise, keyword fallback on the subject — English (`fix`, `bug`,
   `patch`, `add`, `implement`, ...) and Japanese (修正, バグ, 追加,
   実装, ...) substrings. Fix keywords win over feat keywords.

Only fix and feat commits are mined. Known limit (review L-1): projects
that do not use Conventional Commits rely on the keyword fallback, which
is approximate.

### 2.2 normalize / fingerprint

Each added/removed diff line is normalized:

- string literals (`"..."`, `'...'`, `` `...` ``) → `⟨S⟩`
- numeric literals (integer, decimal, hex) → `⟨N⟩`
- whitespace runs collapse to one space; the line is trimmed

Digits that are part of an identifier (`utf8`, `base64`) are preserved.
The fingerprint of a line is the FNV-1a 64-bit hash of `sign + text`.
FNV-1a is an identity hash (not cryptographic); it is used because it is
small, platform-stable, and auditable.

### 2.3 cluster

Commits are processed in ascending hash order (independent of `git log`
traversal order). Each commit's fingerprint set is compared, by Jaccard
similarity, against the representative set (first member's set) of each
existing cluster; at similarity ≥ 0.5 the commit joins that cluster,
otherwise it starts a new one.

A cluster's **support** is its member count. Clusters with
`support >= n` become skill candidates, where `n` is configurable:
`--support <n>`, default **3**.

**Sparse-recurrence premise (review C-1).** The MSR literature shows
within-project fix patterns recur at only ~8% (size-3) to ~21% (size-2),
and mined-pattern yields on small single-author repositories average
~1.4 patterns per repo. Toishi's primary target — a solo developer's
single repository — sits exactly in this sparse region. Consequences,
accepted by design:

- **A zero-skill result is a first-class outcome, not a failure.**
  `mine` exits 0 and reports the reason: no fix/feat commits, no
  parseable diff lines, or the largest pattern's support vs. the
  threshold (with a hint to lower `--support`).
- The support threshold is user-configurable down to 1.
- Cross-repository mining is the structural mitigation. Each explicit
  `--repo` contributes its local HEAD history to a single corpus, deduplicated
  by full commit ID before clustering. Input order does not affect artifacts.
  An unreadable input aborts the entire run before writing output.

### 2.4 emit

Each surviving candidate produces `<out>/<name>/SKILL.md`, a sibling
`sources.txt` manifest, and one `registry.jsonl` record. The manifest contains
`toishi-sources-v1 short` or `toishi-sources-v1 full` on the first line
(the display mode for commit IDs in `SKILL.md`), then every full source commit
ID in ascending order, one per line.

- `name`: slug of the cluster's imperative phrase (see below); lowercase
  ASCII letters/digits/hyphens, ≤ 64 chars, identical to the directory
  name. Collisions get `-2`, `-3`, ... suffixes.
- `description`: ≤ 200 chars, third person, deliberately "pushy"
  (agents under-trigger skills). It is assembled from **trigger
  material**:
  - the *imperative phrase*: the most frequent sanitized commit subject
    (Conventional-Commits prefix stripped) in the cluster;
  - *identifiers*: the most frequent identifier tokens in the
    normalized diff;
  - a *path glob*: the most frequent directory among touched files,
    rendered as `dir/**`.
- body: ≤ 40 lines — pattern summary (support, last verified),
  representative normalized diff lines (most frequent first), trigger
  hints, and source commits (short hash for one repository, full hash for
  multiple repositories, plus sanitized subject; newest first).
  Raw diff is never copied verbatim beyond the normalized,
  redacted representative lines.
- `last_verified`: the newest **commit date inside the cluster** (UTC
  date of the maximum committer timestamp). Never the current time.

`registry.jsonl` holds one JSON object per line:

```json
{"skill":"...","confidence":0.600,"last_verified":"2026-01-04","stability":0.600,"usage_count":0,"success_rate":0.000}
```

Fields: `skill`, `confidence`, `last_verified`, `stability`,
`usage_count`, `success_rate`. Lines are sorted by skill name.

Candidates rejected by the security gate (§4) are recorded in
`rejected.jsonl` as `{"candidate": "...", "reason": "..."}` and also
printed in the run report.

## 3. Confidence lifecycle (scope for v0.1)

- **Initial value** (the only lifecycle stage v0.1 implements):
  `confidence = support / (support + 2)`, printed with three decimals.
  `stability` starts at the same value; `usage_count` and `success_rate`
  start at 0. The formula is deterministic and documented here; it is a
  frequency prior, nothing more.
- **Not in v0.1** (planned, v0.1.x/v0.2): time-based decay and
  event-driven invalidation. Source replay (`verify`) does not update
  confidence or `last_verified`.

Design note (review H-4): FSRS-style forgetting curves model human
memory, whose decay is continuous. Code-pattern staleness is instead
driven by discrete events (dependency major bumps, API deprecations,
framework migrations). When decay is added, it will therefore pair a
slow time component with **event-driven invalidation**: a major-version
bump in `Cargo.toml` / `package.json` diffs marks dependent skills
stale. v0.1 deliberately ships neither rather than shipping a
time-only model that would falsely stale stable patterns.

## 4. Security gate

Commit subjects and diff lines are untrusted: they are copied into
SKILL.md files that an agent will later read as instructions. The gate
applies, in order:

1. **Preprocessing** (a deliberate, documented subset of NFKC — std has
   no Unicode normalization): full-width ASCII (U+FF01–U+FF5E) folds to
   half-width; U+3000 becomes a space; zero-width characters
   (U+200B–U+200D, U+2060, U+FEFF) and bidirectional controls
   (U+202A–U+202E, U+2066–U+2069) are removed.
2. **PII removal**: email addresses and `@mentions` are stripped from
   subjects.
3. **Secret redaction** (gitleaks-style heuristic): a token of ≥ 12
   characters containing both ASCII letters and digits with Shannon
   entropy ≥ 3.5 is treated as a secret and replaced by `⟨R⟩` in
   subjects and diff lines, before normalization. Ordinary identifiers
   (no digits) and natural words (low entropy) are not flagged.
4. **Instruction-override blocklist** (EN/JA): if a phrase such as
   "ignore previous instructions" / 「以前の指示を無視」 appears — after
   preprocessing, so full-width and zero-width evasions are caught — in
   any member subject, representative line, or the generated skill text,
   the **whole candidate is rejected** and the reason recorded.

**Principled limits (review M-1/M-2).** A blocklist cannot be complete:
homoglyph substitution (Cyrillic look-alikes), base64-encoded payloads,
and novel phrasings bypass it. The entropy heuristic likewise both
misses short/low-entropy secrets and can flag innocent high-entropy
tokens (GUIDs, hashes — redacting those is harmless). The gate is a
risk-reduction layer, not a proof of safety; emitted skills are meant
to be reviewed by the repository owner before use, and exit code 3
(§5) surfaces total gate rejection explicitly instead of failing
silently.

## 5. CLI and exit codes

```
toishi mine --repo <path> [--repo <path> ...] [--out <dir>] [--support <n>]
toishi list [--out <dir>]
toishi verify <skill> --repo <path> [--repo <path> ...] [--out <dir>]
```

`--out` defaults to `toishi-out`. `list` reads `<out>/registry.jsonl`
and prints the recorded skills.

When mining multiple repository paths, source references in `SKILL.md`
use full commit IDs. Repository paths are not embedded in the artifacts.

`verify` resolves the manifest's IDs against the supplied local Git
object databases, not the current branch tip. It re-clusters only those
commits, requires every one to contribute to a single pattern, applies
the normal security gate, and compares the regenerated `SKILL.md`
byte-for-byte for the selected name. It does not run the skill or write
output. Missing manifests, missing commit objects and mismatches are
errors, not successful checks. Replay is a reproducibility check, not
proof of source trust or current applicability.

| Exit code | Meaning |
|---|---|
| 0 | Success — including a zero-skill result (reason is printed) |
| 1 | Runtime error (repository unreadable, git failure, I/O failure, or unsuccessful verification) |
| 2 | Usage error (unknown/malformed arguments) |
| 3 | Candidates existed but the security gate rejected **all** of them |

## 6. Determinism

For a fixed repository state and configuration, `mine` produces
byte-identical output:

- git invocations are environment-pinned (§1);
- commits are processed in ascending hash order; all rankings break
  ties lexicographically;
- no wall-clock time, randomness, or environment-dependent value
  reaches the output (dates come from commit timestamps);
- floats are printed with fixed precision (`{:.3}`).

The integration test suite verifies byte-identical output across two
runs on a fixture repository.

## 7. Distribution

v0.1 ships as a standalone CLI binary (build with `cargo build
--release`; no runtime dependencies beyond a `git` binary on `PATH`).
Planned for v0.1.x (review M-3): packaging as a Claude Code plugin with
PostToolUse/Stop hooks so mining can run automatically at session end,
matching how the 2026 ecosystem discovers tools.

## 8. Competitive boundary

Agent-session learning systems (e.g. Anthropic's "Dreaming" for
managed agents, announced 2026-05) extract patterns from **agent
session logs** and rewrite an agent's memory/playbook automatically.
Toishi is deliberately different on three axes:

| | Toishi | Session-learning systems |
|---|---|---|
| Source | **Verified human commits** (fix/feat) | Agent session logs |
| Trust model | Human-validated problem→solution pairs | Self-generated summaries (prompt-sensitive) |
| Output | Deterministic, reviewable `SKILL.md` + JSONL registry | Internal memory notes, auto-applied |

Self-generated skills are empirically fragile (SWE-Skills-Bench: most
sampled skills are prompt-sensitive); starting from commits a human
verified is Toishi's answer. Toishi never modifies anything on its own:
it emits files for the owner to review and adopt.

## 9. Roadmap (post-v0.1)

- v0.1.x: plugin/hook distribution; event-driven invalidation.
- v0.2: cross-repository mining and source replay with `verify` (implemented);
  tree-sitter AST diffs (line diffs break
  under identifier renames and code motion — review H-1); optional,
  explicitly-labeled local-LLM description refinement (the deterministic
  core stays the default).
