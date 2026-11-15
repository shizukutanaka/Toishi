//! classify stage: label each commit as fix / feat / other.
//!
//! Primary signal: a Conventional Commits type prefix (`fix:`, `feat:`,
//! `fix(scope):`, ...). Fallback: English and Japanese keyword matching on
//! the subject (spec §2). Only fix and feat commits are mined in v0.1:
//! they are the verified problem->solution and capability->implementation
//! pairs Toishi distills.

/// Commit category.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Category {
    Fix,
    Feat,
    Other,
}

impl Category {
    /// Whether commits of this category feed the mining pipeline.
    pub fn is_mineable(self) -> bool {
        matches!(self, Category::Fix | Category::Feat)
    }

    /// Lowercase label used in generated output.
    pub fn label(self) -> &'static str {
        match self {
            Category::Fix => "fix",
            Category::Feat => "feat",
            Category::Other => "other",
        }
    }

    /// Third-person verb used at the start of a skill description.
    pub fn third_person_verb(self) -> &'static str {
        match self {
            Category::Fix => "Fixes",
            Category::Feat => "Implements",
            Category::Other => "Handles",
        }
    }
}

/// English fallback keywords (matched as substrings of the lowercased
/// subject). Fix keywords are checked before feat keywords so that
/// e.g. "add fix for ..." classifies as a fix.
const FIX_KEYWORDS_EN: [&str; 8] = [
    "fix", "bugfix", "hotfix", "patch", "repair", "resolve", "bug", "crash",
];
const FEAT_KEYWORDS_EN: [&str; 6] = [
    "add",
    "implement",
    "introduce",
    "feature",
    "support",
    "enable",
];

/// Japanese fallback keywords.
const FIX_KEYWORDS_JA: [&str; 6] = ["修正", "バグ", "不具合", "直す", "復旧", "対処"];
const FEAT_KEYWORDS_JA: [&str; 5] = ["追加", "実装", "導入", "新規", "機能"];

/// Extract the Conventional Commits type token, if the subject has one:
/// the run of ASCII letters before `:` or `(`, at the very start.
fn conventional_type(subject: &str) -> Option<String> {
    let head: String = subject
        .chars()
        .take_while(|c| c.is_ascii_alphabetic())
        .collect();
    if head.is_empty() {
        return None;
    }
    let rest = &subject[head.len()..];
    if rest.starts_with(':') || rest.starts_with('(') || rest.starts_with("!:") {
        Some(head.to_ascii_lowercase())
    } else {
        None
    }
}

/// Classify a commit subject.
pub fn classify(subject: &str) -> Category {
    if let Some(ty) = conventional_type(subject) {
        return match ty.as_str() {
            "fix" => Category::Fix,
            "feat" => Category::Feat,
            _ => Category::Other,
        };
    }
    let lower = subject.to_lowercase();
    if FIX_KEYWORDS_EN.iter().any(|k| lower.contains(k))
        || FIX_KEYWORDS_JA.iter().any(|k| subject.contains(k))
    {
        return Category::Fix;
    }
    if FEAT_KEYWORDS_EN.iter().any(|k| lower.contains(k))
        || FEAT_KEYWORDS_JA.iter().any(|k| subject.contains(k))
    {
        return Category::Feat;
    }
    Category::Other
}

/// Strip the Conventional Commits prefix (`fix(scope): ` etc.) from a
/// subject, returning the imperative core phrase. Subjects without a
/// conventional prefix are returned unchanged (trimmed).
pub fn strip_prefix(subject: &str) -> String {
    if conventional_type(subject).is_some() {
        if let Some(idx) = subject.find(':') {
            return subject[idx + 1..].trim().to_string();
        }
    }
    subject.trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn conventional_prefixes() {
        assert_eq!(classify("fix: null deref on empty input"), Category::Fix);
        assert_eq!(classify("feat(ui): add dark mode"), Category::Feat);
        assert_eq!(classify("fix!: breaking fix"), Category::Fix);
        assert_eq!(classify("docs: update readme"), Category::Other);
        assert_eq!(classify("chore(deps): bump serde"), Category::Other);
        assert_eq!(classify("refactor: split module"), Category::Other);
    }

    #[test]
    fn keyword_fallback_en() {
        assert_eq!(classify("Fix crash when file is missing"), Category::Fix);
        assert_eq!(classify("Add export to CSV"), Category::Feat);
        assert_eq!(classify("Update documentation"), Category::Other);
    }

    #[test]
    fn keyword_fallback_ja() {
        assert_eq!(classify("ログイン時の不具合を修正"), Category::Fix);
        assert_eq!(classify("CSV書き出し機能を追加"), Category::Feat);
        assert_eq!(classify("ドキュメントを更新"), Category::Other);
    }

    #[test]
    fn strip_prefix_works() {
        assert_eq!(
            strip_prefix("fix(auth): token refresh loop"),
            "token refresh loop"
        );
        assert_eq!(strip_prefix("feat: add cache"), "add cache");
        assert_eq!(strip_prefix("no prefix here"), "no prefix here");
    }
}
