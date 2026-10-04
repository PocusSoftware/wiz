//! "Hide sensitive information": masks hostnames, IP addresses and usernames.
//!
//! Masking keeps the text length (every masked char becomes a bullet) so terminal columns,
//! selections and click positions never shift.

use std::borrow::Cow;
use std::ops::Range;
use std::sync::RwLock;
use std::sync::atomic::{AtomicBool, Ordering};

const MASK: char = '•';

static HIDE: AtomicBool = AtomicBool::new(false);
/// Strings to hide: configured hosts and usernames plus the local user.
static TOKENS: RwLock<Vec<Vec<char>>> = RwLock::new(Vec::new());

pub fn hide() -> bool {
    HIDE.load(Ordering::Relaxed)
}

pub fn set_hide(on: bool) {
    HIDE.store(on, Ordering::Relaxed);
}

/// Replace the list of known-sensitive strings. Very short ones are ignored: masking every
/// "a" or "io" would make the UI unreadable.
pub fn set_tokens(tokens: impl IntoIterator<Item = String>) {
    let mut v: Vec<Vec<char>> = tokens
        .into_iter()
        .filter(|t| t.chars().count() >= 3)
        .map(|t| t.chars().collect())
        .collect();
    v.sort();
    v.dedup();
    *TOKENS.write().unwrap() = v;
}

fn is_word(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

fn boundary_before(chars: &[char], i: usize) -> bool {
    i == 0 || !is_word(chars[i - 1])
}

fn boundary_after(chars: &[char], end: usize) -> bool {
    end >= chars.len() || !is_word(chars[end])
}

/// Length of an IPv4 address starting at `i`, if there is one.
fn ipv4_len(chars: &[char], i: usize) -> Option<usize> {
    let mut pos = i;
    for octet in 0..4 {
        let start = pos;
        while pos < chars.len() && chars[pos].is_ascii_digit() && pos - start < 3 {
            pos += 1;
        }
        if pos == start {
            return None;
        }
        let val: u32 = chars[start..pos].iter().collect::<String>().parse().ok()?;
        if val > 255 {
            return None;
        }
        if octet < 3 {
            if chars.get(pos) != Some(&'.') {
                return None;
            }
            pos += 1;
        }
    }
    Some(pos - i)
}

/// True when the candidate is really part of a longer dotted number (1.2.3.4.5).
fn continues_number(chars: &[char], start: usize, end: usize) -> bool {
    let after =
        chars.get(end) == Some(&'.') && chars.get(end + 1).is_some_and(|c| c.is_ascii_digit());
    let before = start >= 2 && chars[start - 1] == '.' && chars[start - 2].is_ascii_digit();
    after || before
}

/// Char ranges of `chars` that must be masked.
pub fn ranges(chars: &[char]) -> Vec<Range<usize>> {
    let tokens = TOKENS.read().unwrap();
    let mut out: Vec<Range<usize>> = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        let mut hit = 0;
        if boundary_before(chars, i) {
            if let Some(n) = ipv4_len(chars, i)
                && boundary_after(chars, i + n)
                && !continues_number(chars, i, i + n)
            {
                hit = n;
            }
            for t in tokens.iter() {
                let end = i + t.len();
                if end <= chars.len()
                    && t.len() > hit
                    && chars[i..end]
                        .iter()
                        .zip(t)
                        .all(|(a, b)| a.to_lowercase().eq(b.to_lowercase()))
                    && boundary_after(chars, end)
                {
                    hit = t.len();
                }
            }
        }
        if hit > 0 {
            out.push(i..i + hit);
            i += hit;
        } else {
            i += 1;
        }
    }
    out
}

/// `s` with sensitive parts masked, or `s` itself when hiding is off.
pub fn redact(s: &str) -> Cow<'_, str> {
    if !hide() {
        return Cow::Borrowed(s);
    }
    let mut chars: Vec<char> = s.chars().collect();
    let found = ranges(&chars);
    if found.is_empty() {
        return Cow::Borrowed(s);
    }
    for r in found {
        for c in &mut chars[r] {
            *c = MASK;
        }
    }
    Cow::Owned(chars.into_iter().collect())
}

pub fn mask_char() -> char {
    MASK
}

#[cfg(test)]
mod tests {
    use super::*;

    // The statics are shared, so every case runs inside one test.
    #[test]
    fn masks_tokens_ips_and_keeps_length() {
        set_tokens([
            "tester".to_string(),
            "my-server.example.com".to_string(),
            "ab".to_string(),
        ]);
        set_hide(false);
        assert_eq!(redact("tester@10.0.0.5"), "tester@10.0.0.5");

        set_hide(true);
        assert_eq!(redact("tester@10.0.0.5:22"), "••••••@••••••••:22");
        // Case-insensitive, boundaries respected: "testers" and "retest" stay readable.
        assert_eq!(redact("TESTER testers retest"), "•••••• testers retest");
        assert_eq!(redact("/home/tester/file"), "/home/••••••/file");
        assert_eq!(
            redact("my-server.example.com ok"),
            "••••••••••••••••••••• ok"
        );
        // Not an address: octet out of range, or part of a longer number.
        assert_eq!(redact("999.1.1.1"), "999.1.1.1");
        assert_eq!(redact("1.2.3.4.5"), "1.2.3.4.5");
        // Too-short tokens are ignored.
        assert_eq!(redact("ab ab"), "ab ab");
        // Length is always preserved.
        for s in ["tester@1.2.3.4", "x 192.168.1.1 y", "naïve tester ✓"] {
            assert_eq!(redact(s).chars().count(), s.chars().count());
        }
        set_hide(false);
    }
}
