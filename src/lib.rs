//! Toishi — mine verified human git commits, distill recurring change
//! patterns into Agent Skills (SKILL.md).
//!
//! Pipeline: collect -> classify -> sanitize -> normalize/fingerprint ->
//! cluster -> security gate -> emit. See `spec.md` for the normative
//! specification.
//!
//! Design invariants:
//! - local-first: no network I/O, ever.
//! - deterministic: the same repository + configuration produces
//!   byte-identical output. Wall-clock time is never used in output.
//! - zero-dependency: the Rust standard library is the entire supply chain.

pub mod classify;
pub mod cluster;
pub mod collect;
pub mod emit;
pub mod normalize;
pub mod sanitize;

use std::collections::BTreeSet;

/// A single line of a unified diff, after security redaction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiffLine {
    /// '+' for an added line, '-' for a removed line.
    pub sign: char,
    /// Repository-relative path of the file the line belongs to.
    pub path: String,
    /// Raw line content (without the leading sign character), with secret
    /// tokens already redacted by the sanitize stage.
    pub text: String,
}

/// A commit with its parsed diff.
#[derive(Debug, Clone)]
pub struct Commit {
    pub hash: String,
    /// Committer time, seconds since the Unix epoch (from `%ct`).
    pub timestamp: i64,
    /// Raw subject line (unsanitized; sanitization happens in the pipeline).
    pub subject: String,
    pub category: classify::Category,
    pub lines: Vec<DiffLine>,
}

/// A diff line after normalization, paired with its fingerprint.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NormLine {
    pub sign: char,
    pub path: String,
    /// Normalized text (no sign prefix).
    pub text: String,
    pub fingerprint: u64,
}

impl NormLine {
    /// The clustering key: sign + normalized text.
    pub fn key(&self) -> String {
        format!("{}{}", self.sign, self.text)
    }
}

/// A commit reduced to its normalized fingerprint set, ready for clustering.
#[derive(Debug, Clone)]
pub struct CommitPattern {
    pub commit_index: usize,
    pub fingerprints: BTreeSet<u64>,
    pub lines: Vec<NormLine>,
}

/// Convert a Unix timestamp (seconds) to a `YYYY-MM-DD` UTC date string.
/// Implemented from first principles (Howard Hinnant's civil-from-days
/// algorithm) so no date crate is needed.
pub fn date_from_unix(ts: i64) -> String {
    let days = ts.div_euclid(86_400);
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097); // [0, 146096]
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365; // [0, 399]
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // [0, 365]
    let mp = (5 * doy + 2) / 153; // [0, 11]
    let d = doy - (153 * mp + 2) / 5 + 1; // [1, 31]
    let m = if mp < 10 { mp + 3 } else { mp - 9 }; // [1, 12]
    let y = if m <= 2 { y + 1 } else { y };
    format!("{y:04}-{m:02}-{d:02}")
}

/// Escape a string for inclusion in a JSON string literal (RFC 8259 subset:
/// quotes, backslash, and control characters).
pub fn json_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn date_epoch() {
        assert_eq!(date_from_unix(0), "1970-01-01");
    }

    #[test]
    fn date_known_values() {
        assert_eq!(date_from_unix(1_700_000_000), "2023-11-14");
        assert_eq!(date_from_unix(1_767_225_600), "2026-01-01");
        // Leap day.
        assert_eq!(date_from_unix(1_709_164_800), "2024-02-29");
    }

    #[test]
    fn json_escape_works() {
        assert_eq!(json_escape("a\"b\\c\nd"), "a\\\"b\\\\c\\nd");
        assert_eq!(json_escape("plain"), "plain");
    }
}
