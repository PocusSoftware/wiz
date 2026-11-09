//! Compiled-in assets.

use super::*;

#[test]
fn every_os_logo_is_compiled_in_and_used() {
    for slug in ssh_core::os::slugs() {
        let path = format!("icons/os/{slug}.svg");
        assert!(ICONS.iter().any(|(p, _)| *p == path), "no logo for {slug}");
    }
    for (path, _) in ICONS {
        if let Some(slug) = path.strip_prefix("icons/os/") {
            let slug = slug.trim_end_matches(".svg");
            assert!(ssh_core::os::is_known(slug), "{slug} is never shown");
        }
    }
}
