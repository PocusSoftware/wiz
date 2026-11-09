//! Which operating system a server runs, for showing its logo. Read from `/etc/os-release`,
//! a few appliance markers and `uname -s`, and reduced to a slug from a fixed table, so
//! nothing the server prints is ever used as is.

#[cfg(test)]
mod tests;

use crate::session::Connection;

/// Appliances and flavors whose os-release just says Arch, Debian, Fedora, Ubuntu or FreeBSD,
/// then the distribution, then the kernel. The first `ID=` line wins.
const COMMAND: &str = "test -d /etc/pve && echo ID=proxmox; \
command -v midclt >/dev/null 2>&1 && echo ID=truenas; \
test -e /etc/unraid-version && echo ID=unraid; \
test -d /etc/openmediavault && echo ID=openmediavault; \
test -e /etc/synoinfo.conf && echo ID=synology; \
test -d /usr/local/opnsense && echo ID=opnsense; \
grep -qs pfSense /etc/platform && echo ID=pfsense; \
test -e /etc/mx-version && echo ID=mx; \
test -d ~/.local/share/omarchy && echo ID=omarchy; \
grep -qs ^TAILS_ /etc/os-release && echo ID=tails; \
grep -qs ^VARIANT_ID=coreos /etc/os-release && echo ID=fedora-coreos; \
sed -En 's/^Package: ([klx]ubuntu)-desktop$/ID=\\1/p' /var/lib/dpkg/status 2>/dev/null; \
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

/// A logo the app ships, one per file in its `icons/os` folder.
#[derive(Debug)]
pub struct Logo {
    pub slug: &'static str,
    /// What the host form lists it as.
    pub title: &'static str,
    /// os-release `ID`s (and lowercase `uname -s`) shown with it. Derivatives not listed
    /// reach their parent's through `ID_LIKE`.
    ids: &'static [&'static str],
}

pub const LOGOS: &[Logo] = &[
    Logo {
        slug: "almalinux",
        title: "AlmaLinux",
        ids: &["almalinux"],
    },
    Logo {
        slug: "alpinelinux",
        title: "Alpine Linux",
        ids: &["alpine"],
    },
    Logo {
        slug: "aosc",
        title: "AOSC OS",
        ids: &["aosc"],
    },
    Logo {
        slug: "archcraft",
        title: "Archcraft",
        ids: &["archcraft"],
    },
    Logo {
        slug: "archlabs",
        title: "ArchLabs",
        ids: &["archlabs"],
    },
    Logo {
        slug: "archlinux",
        title: "Arch Linux",
        ids: &["arch", "archarm"],
    },
    Logo {
        slug: "arcolinux",
        title: "ArcoLinux",
        ids: &["arcolinux"],
    },
    Logo {
        slug: "artixlinux",
        title: "Artix Linux",
        ids: &["artix"],
    },
    Logo {
        slug: "asahilinux",
        title: "Asahi Linux",
        ids: &["fedora-asahi-remix"],
    },
    Logo {
        slug: "biglinux",
        title: "BigLinux",
        ids: &["biglinux"],
    },
    Logo {
        slug: "cachyos",
        title: "CachyOS",
        ids: &["cachyos"],
    },
    Logo {
        slug: "centos",
        title: "CentOS",
        ids: &["centos"],
    },
    Logo {
        slug: "crystal",
        title: "Crystal Linux",
        ids: &["crystal"],
    },
    Logo {
        slug: "debian",
        title: "Debian",
        ids: &["debian"],
    },
    Logo {
        slug: "deepin",
        title: "deepin",
        ids: &["deepin"],
    },
    Logo {
        slug: "devuan",
        title: "Devuan",
        ids: &["devuan"],
    },
    Logo {
        slug: "elementary",
        title: "elementary OS",
        ids: &["elementary"],
    },
    Logo {
        slug: "endeavouros",
        title: "EndeavourOS",
        ids: &["endeavouros"],
    },
    Logo {
        slug: "fedora",
        title: "Fedora",
        ids: &["fedora"],
    },
    Logo {
        slug: "fedoracoreos",
        title: "Fedora CoreOS",
        ids: &["fedora-coreos"],
    },
    Logo {
        slug: "freebsd",
        title: "FreeBSD",
        ids: &["freebsd"],
    },
    Logo {
        slug: "garudalinux",
        title: "Garuda Linux",
        ids: &["garuda"],
    },
    Logo {
        slug: "gentoo",
        title: "Gentoo",
        ids: &["gentoo"],
    },
    Logo {
        slug: "gnome",
        title: "GNOME OS",
        ids: &["org.gnome.os"],
    },
    Logo {
        slug: "googlecontaineroptimizedos",
        title: "Container-Optimized OS",
        ids: &["cos"],
    },
    Logo {
        slug: "guix",
        title: "GNU Guix System",
        ids: &["guix"],
    },
    Logo {
        slug: "homeassistant",
        title: "Home Assistant OS",
        ids: &["haos"],
    },
    Logo {
        slug: "hyperbola",
        title: "Hyperbola",
        ids: &["hyperbola"],
    },
    Logo {
        slug: "illumos",
        title: "illumos",
        ids: &["illumos", "omnios", "openindiana", "smartos", "sunos"],
    },
    Logo {
        slug: "kalilinux",
        title: "Kali Linux",
        ids: &["kali"],
    },
    Logo {
        slug: "kdeneon",
        title: "KDE neon",
        ids: &["neon"],
    },
    Logo {
        slug: "kubuntu",
        title: "Kubuntu",
        ids: &["kubuntu"],
    },
    Logo {
        slug: "lcos",
        title: "LCOS",
        ids: &["lcos"],
    },
    Logo {
        slug: "linux",
        title: "Linux",
        ids: &["linux"],
    },
    Logo {
        slug: "linuxmint",
        title: "Linux Mint",
        ids: &["linuxmint"],
    },
    Logo {
        slug: "locos",
        title: "LOC-OS",
        ids: &["loc-os", "locos"],
    },
    Logo {
        slug: "lubuntu",
        title: "Lubuntu",
        ids: &["lubuntu"],
    },
    Logo {
        slug: "lxle",
        title: "LXLE",
        ids: &["lxle"],
    },
    Logo {
        slug: "macos",
        title: "macOS",
        ids: &["darwin"],
    },
    Logo {
        slug: "mageia",
        title: "Mageia",
        ids: &["mageia"],
    },
    Logo {
        slug: "manjaro",
        title: "Manjaro",
        ids: &["manjaro", "manjaro-arm"],
    },
    Logo {
        slug: "mxlinux",
        title: "MX Linux",
        ids: &["mx"],
    },
    Logo {
        slug: "netbsd",
        title: "NetBSD",
        ids: &["netbsd"],
    },
    Logo {
        slug: "nixos",
        title: "NixOS",
        ids: &["nixos"],
    },
    Logo {
        slug: "nobaralinux",
        title: "Nobara",
        ids: &["nobara"],
    },
    Logo {
        slug: "omarchy",
        title: "Omarchy",
        ids: &["omarchy"],
    },
    Logo {
        slug: "openbsd",
        title: "OpenBSD",
        ids: &["openbsd"],
    },
    Logo {
        slug: "openmandriva",
        title: "OpenMandriva",
        ids: &["openmandriva"],
    },
    Logo {
        slug: "openmediavault",
        title: "openmediavault",
        ids: &["openmediavault"],
    },
    Logo {
        slug: "opensuse",
        title: "openSUSE",
        ids: &[
            "opensuse",
            "opensuse-leap",
            "opensuse-microos",
            "opensuse-slowroll",
            "opensuse-tumbleweed",
        ],
    },
    Logo {
        slug: "openwrt",
        title: "OpenWrt",
        ids: &["openwrt"],
    },
    Logo {
        slug: "opnsense",
        title: "OPNsense",
        ids: &["opnsense"],
    },
    Logo {
        slug: "osmc",
        title: "OSMC",
        ids: &["osmc"],
    },
    Logo {
        slug: "parabola",
        title: "Parabola",
        ids: &["parabola"],
    },
    Logo {
        slug: "parrotsecurity",
        title: "Parrot OS",
        ids: &["parrot"],
    },
    Logo {
        slug: "pfsense",
        title: "pfSense",
        ids: &["pfsense"],
    },
    Logo {
        slug: "photon",
        title: "Photon OS",
        ids: &["photon"],
    },
    Logo {
        slug: "popos",
        title: "Pop!_OS",
        ids: &["pop"],
    },
    Logo {
        slug: "postmarketos",
        title: "postmarketOS",
        ids: &["postmarketos"],
    },
    Logo {
        slug: "proxmox",
        title: "Proxmox VE",
        ids: &["proxmox"],
    },
    Logo {
        slug: "puppy",
        title: "Puppy Linux",
        ids: &["puppy"],
    },
    Logo {
        slug: "qubesos",
        title: "Qubes OS",
        ids: &["qubes"],
    },
    Logo {
        slug: "rancher",
        title: "RancherOS",
        ids: &["rancheros"],
    },
    Logo {
        slug: "raspberrypi",
        title: "Raspberry Pi OS",
        ids: &["raspbian"],
    },
    Logo {
        slug: "redhat",
        title: "Red Hat Enterprise Linux",
        ids: &["rhel"],
    },
    Logo {
        slug: "rockylinux",
        title: "Rocky Linux",
        ids: &["rocky"],
    },
    Logo {
        slug: "sabayon",
        title: "Sabayon",
        ids: &["sabayon"],
    },
    Logo {
        slug: "slackware",
        title: "Slackware",
        ids: &["slackware"],
    },
    Logo {
        slug: "solus",
        title: "Solus",
        ids: &["solus"],
    },
    Logo {
        slug: "steamdeck",
        title: "SteamOS",
        ids: &["steamos"],
    },
    Logo {
        slug: "suse",
        title: "SUSE Linux Enterprise",
        ids: &["sled", "sles", "sles_sap", "suse"],
    },
    Logo {
        slug: "synology",
        title: "Synology DSM",
        ids: &["synology"],
    },
    Logo {
        slug: "tails",
        title: "Tails",
        ids: &["tails"],
    },
    Logo {
        slug: "talos",
        title: "Talos Linux",
        ids: &["talos"],
    },
    Logo {
        slug: "trisquel",
        title: "Trisquel",
        ids: &["trisquel"],
    },
    Logo {
        slug: "truenas",
        title: "TrueNAS",
        ids: &["truenas"],
    },
    Logo {
        slug: "tuxedocomputers",
        title: "TUXEDO OS",
        ids: &["tuxedo"],
    },
    Logo {
        slug: "ubuntu",
        title: "Ubuntu",
        ids: &["ubuntu"],
    },
    Logo {
        slug: "unraid",
        title: "Unraid",
        ids: &["unraid"],
    },
    Logo {
        slug: "vanillaos",
        title: "Vanilla OS",
        ids: &["vanilla"],
    },
    Logo {
        slug: "voidlinux",
        title: "Void Linux",
        ids: &["void"],
    },
    Logo {
        slug: "xerolinux",
        title: "XeroLinux",
        ids: &["xerolinux"],
    },
    Logo {
        slug: "xubuntu",
        title: "Xubuntu",
        ids: &["xubuntu"],
    },
    Logo {
        slug: "zorin",
        title: "Zorin OS",
        ids: &["zorin"],
    },
];

fn slug(id: &str) -> Option<&'static str> {
    LOGOS.iter().find(|l| l.ids.contains(&id)).map(|l| l.slug)
}

/// The logo with this slug, `None` for values from a hand-edited `hosts.toml`.
pub fn logo(slug: &str) -> Option<&'static Logo> {
    LOGOS.iter().find(|l| l.slug == slug)
}
