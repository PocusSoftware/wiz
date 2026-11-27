//! "Hide sensitive information": masks hostnames, usernames, IP addresses, ports and secret values,
//! and hides secret files from file lists. Every category is configurable (see `Settings`).
//!
//! Every masked span becomes the same fixed run of bullets, so the mask never reveals how long
//! the hidden value is. The terminal is the exception: it masks char by char (see `mask_char`).

use std::borrow::Cow;
use std::ops::Range;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{PoisonError, RwLock};

use crate::config::Settings;

const MASK: char = '•';
/// Bullets per masked span, whatever the length of the hidden text.
const MASK_LEN: usize = 6;

static HIDE: AtomicBool = AtomicBool::new(false);
static CONFIG: RwLock<Config> = RwLock::new(Config::new());

struct Config {
    ipv4: bool,
    ipv6: bool,
    ports: bool,
    secrets: bool,
    containers: bool,
    hosts: bool,
    users: bool,
    names: bool,
    files: bool,
    host_tokens: Vec<Vec<char>>,
    user_tokens: Vec<Vec<char>>,
    name_tokens: Vec<Vec<char>>,
    container_tokens: Vec<Vec<char>>,
    extra: Vec<Vec<char>>,
    patterns: Vec<String>,
}

impl Config {
    const fn new() -> Self {
        Self {
            ipv4: true,
            ipv6: true,
            ports: true,
            secrets: true,
            containers: true,
            hosts: true,
            users: true,
            names: false,
            files: true,
            host_tokens: Vec::new(),
            user_tokens: Vec::new(),
            name_tokens: Vec::new(),
            container_tokens: Vec::new(),
            extra: Vec::new(),
            patterns: Vec::new(),
        }
    }
}

pub fn hide() -> bool {
    HIDE.load(Ordering::Relaxed)
}

/// Apply the privacy-related settings.
pub fn configure(s: &Settings) {
    HIDE.store(s.hide_sensitive(), Ordering::Relaxed);
    let mut c = CONFIG.write().unwrap_or_else(PoisonError::into_inner);
    c.ipv4 = s.mask_ipv4();
    c.ipv6 = s.mask_ipv6();
    c.ports = s.mask_ports();
    c.secrets = s.mask_secrets();
    c.containers = s.mask_containers();
    c.hosts = s.mask_hosts();
    c.users = s.mask_users();
    c.names = s.mask_names();
    c.files = s.hide_secret_files();
    c.extra = tokens(s.extra_mask());
    c.patterns = s.secret_file_patterns();
}

fn tokens(items: impl IntoIterator<Item = String>) -> Vec<Vec<char>> {
    // Very short strings are ignored: masking every "a" or "io" would make the UI unreadable.
    let mut v: Vec<Vec<char>> = items
        .into_iter()
        .map(|t| t.trim().to_string())
        .filter(|t| t.chars().count() >= 3)
        .map(|t| t.chars().collect())
        .collect();
    v.sort();
    v.dedup();
    v
}

/// Hostnames, usernames and names of the saved hosts (plus the local user) to mask.
pub fn set_known(hosts: Vec<String>, users: Vec<String>, names: Vec<String>) {
    let mut c = CONFIG.write().unwrap_or_else(PoisonError::into_inner);
    c.host_tokens = tokens(hosts);
    c.user_tokens = tokens(users);
    c.name_tokens = tokens(names);
}

/// Names and IDs of containers seen in a Docker tab, masked whole from then on. Kept for
/// the rest of the run, so a container that was removed stays masked in old output.
pub fn add_containers(items: impl IntoIterator<Item = String>) {
    let mut c = CONFIG.write().unwrap_or_else(PoisonError::into_inner);
    let mut all = std::mem::take(&mut c.container_tokens);
    all.extend(tokens(items));
    all.sort();
    all.dedup();
    c.container_tokens = all;
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

// ---------- IPv4 ----------

fn parse_ipv4(chars: &[char], i: usize) -> Option<usize> {
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
        chars.get(end) == Some(&'.') && chars.get(end + 1).is_some_and(char::is_ascii_digit);
    let before = start >= 2 && chars[start - 1] == '.' && chars[start - 2].is_ascii_digit();
    after || before
}

fn ipv4_len(chars: &[char], i: usize) -> Option<usize> {
    let n = parse_ipv4(chars, i)?;
    (boundary_after(chars, i + n) && !continues_number(chars, i, i + n)).then_some(n)
}

// ---------- IPv6 ----------

fn is_ipv4_str(s: &str) -> bool {
    let chars: Vec<char> = s.chars().collect();
    parse_ipv4(&chars, 0) == Some(chars.len())
}

/// Count 16-bit groups in one side of an address ("1:2:3" or "1:2:1.2.3.4").
fn count_groups(part: &str) -> Option<usize> {
    if part.is_empty() {
        return Some(0);
    }
    let groups: Vec<&str> = part.split(':').collect();
    let mut n = 0;
    for (k, g) in groups.iter().enumerate() {
        if g.contains('.') {
            if k != groups.len() - 1 || !is_ipv4_str(g) {
                return None;
            }
            n += 2;
        } else {
            if g.is_empty() || g.len() > 4 || !g.chars().all(|c| c.is_ascii_hexdigit()) {
                return None;
            }
            n += 1;
        }
    }
    Some(n)
}

/// A real IPv6 address needs either `::` or all eight groups. That keeps times (10:45:30)
/// and MAC addresses (aa:bb:cc:dd:ee:ff) out.
fn valid_ipv6(s: &str) -> bool {
    if s.contains(":::") || s.matches("::").count() > 1 || !s.chars().any(|c| c.is_ascii_hexdigit())
    {
        return false;
    }
    match s.split_once("::") {
        // Compressed forms need a longer group (fe80, 2001, db8...) so scope-like text such
        // as `a::b` is left alone. `::1` is the one common exception.
        Some((head, tail)) => match (count_groups(head), count_groups(tail)) {
            (Some(h), Some(t)) => {
                h + t <= 7
                    && (s == "::1"
                        || s.split(':').any(|g| g.len() >= 3 && !g.contains('.'))
                        || s.contains('.'))
            }
            _ => false,
        },
        None => count_groups(s) == Some(8),
    }
}

fn ipv6_len(chars: &[char], i: usize) -> Option<usize> {
    let mut end = i;
    while end < chars.len()
        && (chars[end].is_ascii_hexdigit() || chars[end] == ':' || chars[end] == '.')
    {
        end += 1;
    }
    // Sentence punctuation after the address is not part of it.
    while end > i && chars[end - 1] == '.' {
        end -= 1;
    }
    if end > i + 1 && chars[end - 1] == ':' && chars[end - 2] != ':' {
        end -= 1;
    }
    if end - i < 2 {
        return None;
    }
    let cand: String = chars[i..end].iter().collect();
    if !valid_ipv6(&cand) {
        return None;
    }
    // Optional zone id, e.g. fe80::1%en0
    let mut stop = end;
    if chars.get(stop) == Some(&'%') {
        stop += 1;
        while stop < chars.len() && (chars[stop].is_alphanumeric() || "._-".contains(chars[stop])) {
            stop += 1;
        }
    }
    boundary_after(chars, stop).then_some(stop - i)
}

// ---------- ports ----------

/// Length of a port number (1-65535) at `i`, ending on a word boundary.
fn port_len(chars: &[char], i: usize) -> Option<usize> {
    let mut end = i;
    while end < chars.len() && chars[end].is_ascii_digit() && end - i < 5 {
        end += 1;
    }
    let val: u32 = chars[i..end].iter().collect::<String>().parse().ok()?;
    ((1..=65535).contains(&val) && boundary_after(chars, end)).then_some(end - i)
}

/// The port in `addr:port` / `[v6]:port`, where the address ends at `end`.
fn port_after(chars: &[char], mut end: usize) -> Option<Range<usize>> {
    if chars.get(end) == Some(&']') {
        end += 1;
    }
    if chars.get(end) != Some(&':') {
        return None;
    }
    port_len(chars, end + 1).map(|n| end + 1..end + 1 + n)
}

/// Ports given by name: `Port 22`, `PORT=3000`, `db_port: 5432`, `--port 8080`, and the
/// `-p`/`-P` flag on lines that run ssh, scp or rsync.
fn keyed_port_ranges(chars: &[char]) -> Vec<Range<usize>> {
    let line: String = chars.iter().collect::<String>().to_lowercase();
    let ssh_line = ["ssh", "scp", "rsync"].iter().any(|w| line.contains(w));
    let mut out = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        if !key_char(chars[i]) || (i > 0 && key_char(chars[i - 1])) {
            i += 1;
            continue;
        }
        let mut k_end = i;
        while k_end < chars.len() && key_char(chars[k_end]) {
            k_end += 1;
        }
        let key: String = chars[i..k_end].iter().collect::<String>().to_lowercase();
        let flag = key == "-p" && ssh_line && (i == 0 || chars[i - 1].is_whitespace());
        let named = key == "port" || ["_port", "-port", ".port"].iter().any(|w| key.ends_with(w));
        if flag || named {
            // Separator: closing quote, spaces, one optional `=` or `:`, opening quote.
            let mut v = k_end;
            while v < chars.len() && (chars[v] == '"' || chars[v] == '\'') {
                v += 1;
            }
            while v < chars.len() && chars[v] == ' ' {
                v += 1;
            }
            if !flag && matches!(chars.get(v), Some('=' | ':')) {
                v += 1;
            }
            while v < chars.len() && (chars[v] == ' ' || chars[v] == '"' || chars[v] == '\'') {
                v += 1;
            }
            if v > k_end
                && let Some(n) = port_len(chars, v)
            {
                out.push(v..v + n);
            }
        }
        i = k_end;
    }
    out
}

// ---------- secrets ----------

const SECRET_KEYS: &[&str] = &[
    "password",
    "passwd",
    "passphrase",
    "secret",
    "token",
    "apikey",
    "api_key",
    "api-key",
    "private_key",
    "privatekey",
    "access_key",
    "accesskey",
    "credential",
];

fn key_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || "_.-".contains(c)
}

/// Values of `PASSWORD=hunter2`, `token: abc`, `"api_key": "xyz"` style assignments.
fn secret_ranges(chars: &[char]) -> Vec<Range<usize>> {
    let mut out = Vec::new();
    for i in 0..chars.len() {
        if chars[i] != '=' && chars[i] != ':' {
            continue;
        }
        // Key before the separator, allowing a closing quote or spaces ("password" : ...).
        let mut k_end = i;
        while k_end > 0
            && (chars[k_end - 1] == '"' || chars[k_end - 1] == '\'' || chars[k_end - 1] == ' ')
        {
            k_end -= 1;
        }
        let mut k_start = k_end;
        while k_start > 0 && key_char(chars[k_start - 1]) {
            k_start -= 1;
        }
        if k_start == k_end {
            continue;
        }
        let key: String = chars[k_start..k_end]
            .iter()
            .collect::<String>()
            .to_lowercase();
        if !SECRET_KEYS.iter().any(|w| key.contains(w)) {
            continue;
        }
        // Value after the separator.
        let mut v = i + 1;
        while v < chars.len() && chars[v] == ' ' {
            v += 1;
        }
        let quote = chars.get(v).copied().filter(|c| *c == '"' || *c == '\'');
        if quote.is_some() {
            v += 1;
        }
        let mut e = v;
        while e < chars.len() {
            let ch = chars[e];
            let stop = match quote {
                Some(q) => ch == q,
                None => ch.is_whitespace() || ch == ',' || ch == ';' || ch == '"' || ch == '\'',
            };
            if stop {
                break;
            }
            e += 1;
        }
        if e > v {
            out.push(v..e);
        }
    }
    out
}

// ---------- public API ----------

/// Char ranges of `chars` that must be masked.
pub fn ranges(chars: &[char]) -> Vec<Range<usize>> {
    let cfg = CONFIG.read().unwrap_or_else(PoisonError::into_inner);
    let mut out: Vec<Range<usize>> = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        let mut hit = 0;
        if boundary_before(chars, i) {
            // Longest address here, masked or not; a port after it is masked on its own.
            let mut addr = 0;
            if let Some(n) = ipv6_len(chars, i) {
                addr = n;
                if cfg.ipv6 {
                    hit = n;
                }
            }
            if let Some(n) = ipv4_len(chars, i) {
                addr = addr.max(n);
                if cfg.ipv4 {
                    hit = hit.max(n);
                }
            }
            let lists = [
                (cfg.hosts, true, &cfg.host_tokens),
                (cfg.users, false, &cfg.user_tokens),
                (cfg.names, false, &cfg.name_tokens),
                (cfg.containers, false, &cfg.container_tokens),
                (true, false, &cfg.extra),
            ];
            for (on, is_addr, list) in lists {
                if !on && !is_addr {
                    continue;
                }
                for t in list {
                    let end = i + t.len();
                    if end <= chars.len()
                        && chars[i..end]
                            .iter()
                            .zip(t)
                            .all(|(a, b)| a.to_lowercase().eq(b.to_lowercase()))
                        && boundary_after(chars, end)
                    {
                        if is_addr {
                            addr = addr.max(t.len());
                        }
                        if on {
                            hit = hit.max(t.len());
                        }
                    }
                }
            }
            if cfg.ports
                && addr > 0
                && let Some(r) = port_after(chars, i + addr)
            {
                out.push(r);
            }
        }
        if hit > 0 {
            out.push(i..i + hit);
            i += hit;
        } else {
            i += 1;
        }
    }
    if cfg.ports {
        out.extend(keyed_port_ranges(chars));
    }
    if cfg.secrets {
        out.extend(secret_ranges(chars));
    }
    out.sort_by_key(|r| r.start);
    // Merge overlaps so each char is masked once.
    let mut merged: Vec<Range<usize>> = Vec::new();
    for r in out {
        match merged.last_mut() {
            Some(last) if r.start <= last.end => last.end = last.end.max(r.end),
            _ => merged.push(r),
        }
    }
    merged
}

/// The fixed placeholder that stands in for every masked span.
pub fn mask() -> String {
    std::iter::repeat_n(MASK, MASK_LEN).collect()
}

/// `s` with sensitive parts masked, or `s` itself when hiding is off.
pub fn redact(s: &str) -> Cow<'_, str> {
    if !hide() {
        return Cow::Borrowed(s);
    }
    let chars: Vec<char> = s.chars().collect();
    let found = ranges(&chars);
    if found.is_empty() {
        return Cow::Borrowed(s);
    }
    let mut out = String::with_capacity(s.len());
    let mut i = 0;
    for r in found {
        out.extend(&chars[i..r.start]);
        out.push_str(&mask());
        i = r.end;
    }
    out.extend(&chars[i..]);
    Cow::Owned(out)
}

/// The bullet the terminal paints over each masked char. The terminal masks in place (same
/// length) so its columns, selections and click positions never shift.
pub fn mask_char() -> char {
    MASK
}

/// True when port numbers should be shown masked.
pub fn hides_ports() -> bool {
    hide() && CONFIG.read().unwrap_or_else(PoisonError::into_inner).ports
}

/// `text` (a port, or a list/range of them) with each number masked when ports are hidden.
pub fn port(text: impl ToString) -> String {
    let text = text.to_string();
    if !hides_ports() {
        return text;
    }
    let mut out = String::new();
    let mut digits = false;
    for c in text.chars() {
        if c.is_ascii_digit() {
            if !digits {
                out.push_str(&mask());
            }
            digits = true;
        } else {
            out.push(c);
            digits = false;
        }
    }
    out
}

// ---------- secret files ----------

/// Case-insensitive match with `*` (any run) and `?` (one char).
fn glob(pattern: &str, name: &str) -> bool {
    let p: Vec<char> = pattern.to_lowercase().chars().collect();
    let n: Vec<char> = name.to_lowercase().chars().collect();
    let (mut pi, mut ni, mut star, mut mark) = (0, 0, None, 0);
    while ni < n.len() {
        if pi < p.len() && (p[pi] == '?' || p[pi] == n[ni]) {
            pi += 1;
            ni += 1;
        } else if pi < p.len() && p[pi] == '*' {
            star = Some(pi);
            mark = ni;
            pi += 1;
        } else if let Some(s) = star {
            pi = s + 1;
            mark += 1;
            ni = mark;
        } else {
            return false;
        }
    }
    while pi < p.len() && p[pi] == '*' {
        pi += 1;
    }
    pi == p.len()
}

/// True when a file list should not show `name` (hiding on, and it matches a secret pattern).
pub fn file_hidden(name: &str) -> bool {
    if !hide() {
        return false;
    }
    let cfg = CONFIG.read().unwrap_or_else(PoisonError::into_inner);
    cfg.files && cfg.patterns.iter().any(|p| glob(p.trim(), name))
}

#[cfg(test)]
mod tests {
    //! Tests for `privacy`.

    use super::*;

    /// The mask that stands in for any span. The argument is the hidden text's length,
    /// kept for readability only: the mask never depends on it.
    fn b(_n: usize) -> String {
        mask()
    }

    fn setup(f: impl FnOnce(&mut Settings)) {
        let mut s = Settings {
            hide_sensitive: Some(true),
            ..Default::default()
        };
        f(&mut s);
        configure(&s);
        set_known(
            vec!["my-server.example.com".into(), "ab".into()],
            vec!["tester".into()],
            vec!["rekade".into()],
        );
    }

    // The statics are shared, so everything runs in one test.
    #[test]
    fn privacy_engine() {
        // Off: nothing changes.
        configure(&Settings::default());
        assert_eq!(redact("tester@10.0.0.5"), "tester@10.0.0.5");
        assert!(!file_hidden(".env"));

        setup(|_| {});
        // Names and IPv4, keeping length.
        assert_eq!(
            redact("tester@10.0.0.5:22"),
            format!("{}@{}:{}", b(6), b(8), b(2))
        );
        assert_eq!(
            redact("TESTER testers retest"),
            format!("{} testers retest", b(6))
        );
        assert_eq!(redact("/home/tester/file"), format!("/home/{}/file", b(6)));
        assert_eq!(redact("my-server.example.com ok"), format!("{} ok", b(21)));
        assert_eq!(redact("999.1.1.1 1.2.3.4.5"), "999.1.1.1 1.2.3.4.5");
        assert_eq!(redact("ab ab"), "ab ab");

        // IPv6: full, compressed, loopback, bracketed with port, mapped IPv4, zone id.
        assert_eq!(redact("2001:db8::1"), b(11));
        assert_eq!(redact("fe80::1%en0 up"), format!("{} up", b(11)));
        assert_eq!(redact("[::1]:22"), format!("[{}]:{}", b(3), b(2)));
        assert_eq!(
            redact("2001:0db8:0000:0000:0000:ff00:0042:8329."),
            format!("{}.", b(39))
        );
        assert_eq!(redact("::ffff:192.168.1.1"), b(18));
        // Not addresses: times, MACs, C++ scopes, plain words.
        assert_eq!(
            redact("at 10:45:30 mac aa:bb:cc:dd:ee:ff"),
            "at 10:45:30 mac aa:bb:cc:dd:ee:ff"
        );
        assert_eq!(redact("std::string a::b"), "std::string a::b");

        // Secret-looking assignments.
        assert_eq!(
            redact("PASSWORD=hunter2 ok"),
            format!("PASSWORD={} ok", b(7))
        );
        assert_eq!(
            redact("api_key: \"abc def\" x"),
            format!("api_key: \"{}\" x", b(7))
        );
        assert_eq!(
            redact("{\"token\": \"xyz\"}"),
            format!("{{\"token\": \"{}\"}}", b(3))
        );
        assert_eq!(
            redact("PATH=/usr/bin user=alice"),
            "PATH=/usr/bin user=alice"
        );

        // Ports: after addresses and saved hosts, by name, and the ssh -p flag.
        assert_eq!(
            redact("my-server.example.com:2222"),
            format!("{}:{}", b(21), b(4))
        );
        assert_eq!(redact("ssh -p 2222 x"), format!("ssh -p {} x", b(4)));
        assert_eq!(redact("Port 22"), format!("Port {}", b(2)));
        assert_eq!(redact("PORT=3000"), format!("PORT={}", b(4)));
        assert_eq!(redact("db_port: 5432"), format!("db_port: {}", b(4)));
        assert_eq!(redact("--port=8080"), format!("--port={}", b(4)));
        assert_eq!(
            redact("report: 5 ports 80 ps -p 42 x:99"),
            "report: 5 ports 80 ps -p 42 x:99"
        );
        assert_eq!(redact("port 99999"), "port 99999");
        assert_eq!(port(2222), b(4));
        assert_eq!(port("80-90"), format!("{}-{}", b(2), b(2)));

        // Files.
        assert!(file_hidden(".env"));
        assert!(file_hidden(".ENV.production"));
        assert!(file_hidden("prod.env"));
        assert!(file_hidden("server.pem"));
        assert!(file_hidden("id_ed25519"));
        assert!(!file_hidden("id_ed25519.pub"));
        assert!(!file_hidden("environment.txt"));
        assert!(!file_hidden("readme.md"));

        // Every category can be switched off on its own.
        setup(|s| s.mask_ipv6 = Some(false));
        assert_eq!(redact("2001:db8::1"), "2001:db8::1");
        setup(|s| s.mask_ipv4 = Some(false));
        assert_eq!(redact("10.0.0.5"), "10.0.0.5");
        setup(|s| s.mask_secrets = Some(false));
        assert_eq!(redact("PASSWORD=hunter2"), "PASSWORD=hunter2");
        setup(|s| s.mask_users = Some(false));
        assert_eq!(redact("tester"), "tester");
        // Host names are off by default.
        assert_eq!(redact("rekade"), "rekade");
        setup(|s| s.mask_names = Some(true));
        assert_eq!(redact("rekade"), b(6));
        setup(|s| s.mask_hosts = Some(false));
        assert_eq!(redact("my-server.example.com"), "my-server.example.com");
        assert_eq!(
            redact("my-server.example.com:2222"),
            format!("my-server.example.com:{}", b(4))
        );
        setup(|s| s.mask_ipv4 = Some(false));
        assert_eq!(redact("10.0.0.5:22"), format!("10.0.0.5:{}", b(2)));
        setup(|s| s.mask_ports = Some(false));
        assert_eq!(
            redact("tester@10.0.0.5:22"),
            format!("{}@{}:22", b(6), b(8))
        );
        assert_eq!(redact("ssh -p 2222 PORT=1"), "ssh -p 2222 PORT=1");
        assert_eq!(port(22), "22");
        setup(|s| s.hide_secret_files = Some(false));
        assert!(!file_hidden(".env"));
        setup(|s| s.secret_file_patterns = Some(vec!["*.sql".into()]));
        assert!(file_hidden("dump.SQL"));
        assert!(!file_hidden(".env"));
        setup(|s| s.extra_mask = Some(vec!["Project-Falcon".into()]));
        assert_eq!(
            redact("deploy project-falcon now"),
            format!("deploy {} now", b(14))
        );

        setup(|_| {});
        let task = "a_media_automod_discord.1.apzpmaybcuijjz4oceqvlcnbf";
        assert_eq!(redact(task), task);
        add_containers([task.to_string(), "4a066c8b481f".to_string()]);
        assert_eq!(
            redact(&format!("{task} 4a066c8b481f Exited (1)")),
            format!("{} {} Exited (1)", b(48), b(12))
        );
        assert_eq!(
            redact("a_media_automod_discord.2"),
            "a_media_automod_discord.2"
        );
        setup(|s| s.mask_containers = Some(false));
        assert_eq!(redact(task), task);

        // The mask never reveals the hidden length.
        setup(|_| {});
        assert_eq!(
            redact("PASSWORD=a"),
            redact("PASSWORD=a-much-longer-secret")
        );
        assert_eq!(redact("1.2.3.4"), redact("192.168.100.200"));
        assert_eq!(port(22), port(65535));

        configure(&Settings::default());
    }
}
