//! normalize / fingerprint stage (spec §2).
//!
//! A diff line is normalized so that the *shape* of a change survives while
//! its incidental literals do not:
//! - string literals (`"..."`, `'...'`, `` `...` ``) become `⟨S⟩`,
//! - numeric literals (decimal, float, hex) become `⟨N⟩`,
//! - whitespace runs collapse to a single space and the line is trimmed.
//!
//! The fingerprint of a normalized line is its FNV-1a 64-bit hash. FNV-1a
//! is tiny, deterministic across platforms, and easy to audit — the right
//! trade for a zero-dependency tool (it is an identity hash here, not a
//! cryptographic one).

/// Placeholder for a string literal.
pub const STRING_PLACEHOLDER: &str = "⟨S⟩";
/// Placeholder for a numeric literal.
pub const NUMBER_PLACEHOLDER: &str = "⟨N⟩";

/// FNV-1a 64-bit hash.
pub fn fnv1a_64(data: &[u8]) -> u64 {
    const OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
    const PRIME: u64 = 0x0000_0100_0000_01b3;
    let mut hash = OFFSET;
    for &byte in data {
        hash ^= byte as u64;
        hash = hash.wrapping_mul(PRIME);
    }
    hash
}

/// Replace string literals in `line` with [`STRING_PLACEHOLDER`].
/// A backslash escape inside a literal does not terminate it. An
/// unterminated literal consumes the rest of the line.
fn replace_strings(line: &str) -> String {
    let chars: Vec<char> = line.chars().collect();
    let mut out = String::with_capacity(line.len());
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c == '"' || c == '\'' || c == '`' {
            out.push_str(STRING_PLACEHOLDER);
            i += 1;
            while i < chars.len() {
                if chars[i] == '\\' {
                    i += 2; // skip the escaped character
                    continue;
                }
                if chars[i] == c {
                    i += 1;
                    break;
                }
                i += 1;
            }
        } else {
            out.push(c);
            i += 1;
        }
    }
    out
}

/// Replace numeric literals in `line` with [`NUMBER_PLACEHOLDER`].
/// Handles integers, decimals (`1.5`), and hex (`0xFF`). A digit run that
/// is part of an identifier (preceded by a letter, `_`, or `.` in a name
/// like `utf8`) is left alone — with the exception that a leading digit
/// always starts a literal, matching how lexers tokenize.
fn replace_numbers(line: &str) -> String {
    let chars: Vec<char> = line.chars().collect();
    let mut out = String::with_capacity(line.len());
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        let prev_is_ident = i > 0 && (chars[i - 1].is_alphanumeric() || chars[i - 1] == '_');
        if c.is_ascii_digit() && !prev_is_ident {
            // Hex literal: 0x...
            if c == '0'
                && i + 2 < chars.len()
                && (chars[i + 1] == 'x' || chars[i + 1] == 'X')
                && chars[i + 2].is_ascii_hexdigit()
            {
                i += 2;
                while i < chars.len() && chars[i].is_ascii_hexdigit() {
                    i += 1;
                }
                out.push_str(NUMBER_PLACEHOLDER);
                continue;
            }
            while i < chars.len() && chars[i].is_ascii_digit() {
                i += 1;
            }
            // Fractional part: '.' followed by a digit.
            if i + 1 < chars.len() && chars[i] == '.' && chars[i + 1].is_ascii_digit() {
                i += 1;
                while i < chars.len() && chars[i].is_ascii_digit() {
                    i += 1;
                }
            }
            out.push_str(NUMBER_PLACEHOLDER);
        } else {
            out.push(c);
            i += 1;
        }
    }
    out
}

/// Normalize one diff line's content (without its sign). Returns `None`
/// when nothing but whitespace remains.
pub fn normalize_line(text: &str) -> Option<String> {
    let no_strings = replace_strings(text);
    let no_numbers = replace_numbers(&no_strings);
    let collapsed = no_numbers.split_whitespace().collect::<Vec<_>>().join(" ");
    if collapsed.is_empty() {
        None
    } else {
        Some(collapsed)
    }
}

/// Fingerprint of a normalized line key (sign + text).
pub fn fingerprint(key: &str) -> u64 {
    fnv1a_64(key.as_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fnv1a_known_values() {
        assert_eq!(fnv1a_64(b""), 0xcbf2_9ce4_8422_2325);
        assert_eq!(fnv1a_64(b"a"), 0xaf63_dc4c_8601_ec8c);
        assert_eq!(fnv1a_64(b"foobar"), 0x8594_4171_f739_67e8);
    }

    #[test]
    fn strings_become_placeholder() {
        assert_eq!(
            normalize_line("name = \"hello world\"").unwrap(),
            "name = ⟨S⟩"
        );
        assert_eq!(
            normalize_line("path = '/tmp/x' + `tpl`").unwrap(),
            "path = ⟨S⟩ + ⟨S⟩"
        );
        // Escaped quote does not end the literal.
        assert_eq!(normalize_line("s = \"a\\\"b\" + c").unwrap(), "s = ⟨S⟩ + c");
    }

    #[test]
    fn numbers_become_placeholder() {
        assert_eq!(normalize_line("timeout = 30").unwrap(), "timeout = ⟨N⟩");
        assert_eq!(normalize_line("ratio = 1.5").unwrap(), "ratio = ⟨N⟩");
        assert_eq!(normalize_line("mask = 0xFF").unwrap(), "mask = ⟨N⟩");
        // Digits inside identifiers survive.
        assert_eq!(normalize_line("utf8_encode(x)").unwrap(), "utf8_encode(x)");
        assert_eq!(normalize_line("base64 = on").unwrap(), "base64 = on");
    }

    #[test]
    fn whitespace_collapses() {
        assert_eq!(
            normalize_line("   let   x\t=\t  42 ; ").unwrap(),
            "let x = ⟨N⟩ ;"
        );
        assert_eq!(normalize_line("   \t  "), None);
    }

    #[test]
    fn combined_normalization() {
        assert_eq!(normalize_line("log(\"retry 3 of 5\")").unwrap(), "log(⟨S⟩)");
    }
}
