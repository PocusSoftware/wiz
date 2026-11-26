//! Icons, brand images and fonts compiled into the binary. Icons are served to gpui by
//! path, fonts are added to the text system at startup.

#[cfg(test)]
mod tests;

use std::borrow::Cow;

use gpui::{AssetSource, Result, SharedString};

macro_rules! icons {
    ($($name:literal),* $(,)?) => {
        const ICONS: &[(&str, &[u8])] = &[
            $((concat!("icons/", $name, ".svg"), include_bytes!(concat!("../assets/icons/", $name, ".svg")))),*
        ];
    };
}

icons!(
    "plus",
    "x",
    "pencil",
    "copy",
    "trash",
    "refresh",
    "play",
    "stop",
    "terminal",
    "keyboard",
    "logs",
    "container",
    "server",
    "key",
    "folder",
    "minus",
    "maximize",
    "file",
    "arrow-left",
    "arrow-right",
    "arrow-up",
    "home",
    "eye",
    "eye-off",
    "folder-plus",
    "upload",
    "chevron-up",
    "chevron-down",
    "check",
    "arrow-down",
    "hat",
    "sliders",
    "pin",
    "chevron-right",
    "columns",
    "pause",
    "external-link",
    "os/almalinux",
    "os/alpinelinux",
    "os/aosc",
    "os/archcraft",
    "os/archlabs",
    "os/archlinux",
    "os/arcolinux",
    "os/artixlinux",
    "os/asahilinux",
    "os/biglinux",
    "os/cachyos",
    "os/centos",
    "os/crystal",
    "os/debian",
    "os/deepin",
    "os/devuan",
    "os/elementary",
    "os/endeavouros",
    "os/fedora",
    "os/fedoracoreos",
    "os/freebsd",
    "os/garudalinux",
    "os/gentoo",
    "os/gnome",
    "os/googlecontaineroptimizedos",
    "os/guix",
    "os/homeassistant",
    "os/hyperbola",
    "os/illumos",
    "os/kalilinux",
    "os/kdeneon",
    "os/kubuntu",
    "os/lcos",
    "os/linux",
    "os/linuxmint",
    "os/locos",
    "os/lubuntu",
    "os/lxle",
    "os/macos",
    "os/mageia",
    "os/manjaro",
    "os/mxlinux",
    "os/netbsd",
    "os/nixos",
    "os/nobaralinux",
    "os/omarchy",
    "os/openbsd",
    "os/openmandriva",
    "os/openmediavault",
    "os/opensuse",
    "os/openwrt",
    "os/opnsense",
    "os/osmc",
    "os/parabola",
    "os/parrotsecurity",
    "os/pfsense",
    "os/photon",
    "os/popos",
    "os/postmarketos",
    "os/proxmox",
    "os/puppy",
    "os/qubesos",
    "os/rancher",
    "os/raspberrypi",
    "os/redhat",
    "os/rockylinux",
    "os/sabayon",
    "os/slackware",
    "os/solus",
    "os/steamdeck",
    "os/suse",
    "os/synology",
    "os/tails",
    "os/talos",
    "os/trisquel",
    "os/truenas",
    "os/tuxedocomputers",
    "os/ubuntu",
    "os/unraid",
    "os/vanillaos",
    "os/voidlinux",
    "os/xerolinux",
    "os/xubuntu",
    "os/zorin",
);

/// Default UI and terminal fonts (SIL Open Font License, texts next to the files), so
/// every platform looks the same without installing anything.
pub const FONTS: &[&[u8]] = &[
    include_bytes!("../assets/fonts/IBMPlexSans-Regular.ttf"),
    include_bytes!("../assets/fonts/IBMPlexSans-Medium.ttf"),
    include_bytes!("../assets/fonts/IBMPlexSans-SemiBold.ttf"),
    include_bytes!("../assets/fonts/JetBrainsMono-Regular.ttf"),
    include_bytes!("../assets/fonts/JetBrainsMono-Bold.ttf"),
    include_bytes!("../assets/fonts/JetBrainsMono-Italic.ttf"),
    include_bytes!("../assets/fonts/JetBrainsMono-BoldItalic.ttf"),
];

/// Icons are compiled into the binary (Lucide, ISC license; server OS logos in `os/` from
/// Simple Icons, CC0, and font-logos, public domain, texts next to the files; `lcos` drawn
/// for Wiz after the LCOS badge, lettering in IBM Plex Sans).
pub struct Assets;

impl AssetSource for Assets {
    fn load(&self, path: &str) -> Result<Option<Cow<'static, [u8]>>> {
        Ok(ICONS
            .iter()
            .find(|(p, _)| *p == path)
            .map(|(_, b)| Cow::Borrowed(*b)))
    }

    fn list(&self, path: &str) -> Result<Vec<SharedString>> {
        Ok(ICONS
            .iter()
            .filter(|(p, _)| p.starts_with(path))
            .map(|(p, _)| (*p).into())
            .collect())
    }
}
