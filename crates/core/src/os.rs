//! Which operating system a server runs, for showing its logo. Read from `/etc/os-release`,
//! a few appliance markers and `uname -s`, and reduced to a slug from a fixed table, so
//! nothing the server prints is ever used as is.

#[cfg(test)]
mod tests;

use crate::session::Connection;

/// Appliances whose os-release just says Debian or FreeBSD, then the distribution, then the
/// kernel. The first `ID=` line wins.
const COMMAND: &str = "test -d /etc/pve && echo ID=proxmox; \
command -v midclt >/dev/null 2>&1 && echo ID=truenas; \
test -e /etc/unraid-version && echo ID=unraid; \
test -d /etc/openmediavault && echo ID=openmediavault; \
test -e /etc/synoinfo.conf && echo ID=synology; \
test -d /usr/local/opnsense && echo ID=opnsense; \
grep -qs pfSense /etc/platform && echo ID=pfsense; \
cat /etc/os-release 2>/dev/null; uname -s";

/// The logo slug of the server behind `conn`, `None` when it can't be told (Windows, a
/// forced command, an unknown kernel).
pub async fn detect(conn: &Connection) -> Option<&'static str> {
    let out = conn.exec(COMMAND).await.ok()?;
    parse(&out.stdout_str())
}

/// Pick the slug from the output of [`COMMAND`]: the `ID`, then each `ID_LIKE` (a derivative
/// gets its parent's logo), then the kernel name.
pub fn parse(out: &str) -> Option<&'static str> {
    let mut ids = Vec::new();
    let mut like = Vec::new();
    let mut kernel = None;
    for line in out.lines().map(str::trim) {
        if let Some(v) = line.strip_prefix("ID=") {
            ids.push(unquote(v));
        } else if let Some(v) = line.strip_prefix("ID_LIKE=") {
            like.extend(unquote(v).split_whitespace());
        } else if !line.is_empty() && line.chars().all(|c| c.is_ascii_alphanumeric()) {
            kernel = Some(line);
        }
    }
    ids.into_iter()
        .chain(like)
        .chain(kernel)
        .find_map(|id| slug(&id.to_ascii_lowercase()))
}

fn unquote(v: &str) -> &str {
    v.trim_matches(|c| c == '"' || c == '\'')
}

/// os-release `ID`s (and lowercase `uname -s`) and their logos, one per file in the app's
/// `icons/os` folder. Derivatives not listed reach their parent's through `ID_LIKE`.
const LOGOS: &[(&str, &str)] = &[
    ("almalinux", "almalinux"),
    ("alpine", "alpinelinux"),
    ("arch", "archlinux"),
    ("archarm", "archlinux"),
    ("artix", "artixlinux"),
    ("centos", "centos"),
    ("cos", "googlecontaineroptimizedos"),
    ("darwin", "macos"),
    ("debian", "debian"),
    ("deepin", "deepin"),
    ("elementary", "elementary"),
    ("endeavouros", "endeavouros"),
    ("fedora", "fedora"),
    ("fedora-asahi-remix", "asahilinux"),
    ("freebsd", "freebsd"),
    ("garuda", "garudalinux"),
    ("gentoo", "gentoo"),
    ("haos", "homeassistant"),
    ("kali", "kalilinux"),
    ("linux", "linux"),
    ("linuxmint", "linuxmint"),
    ("manjaro", "manjaro"),
    ("manjaro-arm", "manjaro"),
    ("neon", "kde"),
    ("netbsd", "netbsd"),
    ("nixos", "nixos"),
    ("nobara", "nobaralinux"),
    ("openbsd", "openbsd"),
    ("openmediavault", "openmediavault"),
    ("opensuse", "opensuse"),
    ("opensuse-leap", "opensuse"),
    ("opensuse-microos", "opensuse"),
    ("opensuse-slowroll", "opensuse"),
    ("opensuse-tumbleweed", "opensuse"),
    ("openwrt", "openwrt"),
    ("opnsense", "opnsense"),
    ("osmc", "osmc"),
    ("parrot", "parrotsecurity"),
    ("pfsense", "pfsense"),
    ("photon", "photon"),
    ("pop", "popos"),
    ("proxmox", "proxmox"),
    ("rancheros", "rancher"),
    ("raspbian", "raspberrypi"),
    ("rhel", "redhat"),
    ("rocky", "rockylinux"),
    ("slackware", "slackware"),
    ("sled", "suse"),
    ("sles", "suse"),
    ("sles_sap", "suse"),
    ("solus", "solus"),
    ("steamos", "steamdeck"),
    ("suse", "suse"),
    ("synology", "synology"),
    ("talos", "talos"),
    ("truenas", "truenas"),
    ("ubuntu", "ubuntu"),
    ("unraid", "unraid"),
    ("void", "voidlinux"),
    ("zorin", "zorin"),
];

fn slug(id: &str) -> Option<&'static str> {
    LOGOS.iter().find(|(i, _)| *i == id).map(|(_, s)| *s)
}

/// Every slug [`parse`] can return, so the app can check it has a logo for each.
pub fn slugs() -> impl Iterator<Item = &'static str> {
    LOGOS.iter().map(|(_, s)| *s)
}

/// Whether `slug` is one [`parse`] returns, for values read back from `hosts.toml`.
pub fn is_known(slug: &str) -> bool {
    slugs().any(|s| s == slug)
}
