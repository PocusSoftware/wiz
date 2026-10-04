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
    "trash",
    "refresh",
    "play",
    "stop",
    "terminal",
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
    "download",
    "chevron-up",
    "chevron-down",
    "check",
    "arrow-down",
    "castle"
);

/// Icons are compiled into the binary (Lucide, ISC license).
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
