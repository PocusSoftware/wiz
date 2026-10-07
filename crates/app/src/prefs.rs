//! Process-wide settings: loaded once, changed through `update`, which applies the effects
//! (privacy engine, secret store) and saves `settings.toml`.

use std::sync::{Arc, RwLock};

use ssh_core::config::Settings;

use crate::{privacy, theme};

static PREFS: RwLock<Option<Arc<Settings>>> = RwLock::new(None);

fn apply(s: &Settings) {
    privacy::configure(s);
    theme::set_accent(theme::parse_hex(&s.accent_color()).unwrap_or(theme::DEFAULT_ACCENT));
    ssh_core::secrets::set_store(ssh_core::secrets::Store::from_setting(&s.secret_store()));
}

pub fn init(s: Settings) {
    apply(&s);
    *PREFS.write().unwrap() = Some(Arc::new(s));
}

/// Cheap snapshot of the current settings.
pub fn get() -> Arc<Settings> {
    PREFS.read().unwrap().clone().unwrap_or_default()
}

/// Change settings, apply them and write the file. The in-memory change sticks even if the
/// write fails (the error is returned so the UI can say so).
pub fn update(f: impl FnOnce(&mut Settings)) -> ssh_core::Result<()> {
    let mut s = (*get()).clone();
    f(&mut s);
    apply(&s);
    *PREFS.write().unwrap() = Some(Arc::new(s.clone()));
    s.save()
}
