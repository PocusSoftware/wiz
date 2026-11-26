//! Wiz: app startup, key bindings, menus and the main window.
// Release builds on Windows are a GUI app: no console window next to it.
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod assets;
mod dialogs;
mod platform;
mod prefs;
mod runtime;
mod shortcuts;
#[cfg(test)]
mod testing;
mod theme;
mod views;
mod widgets;
mod workspace;

use gpui::{App, Bounds, Menu, MenuItem, WindowBounds, WindowOptions, prelude::*, px, size};
use ssh_core::{privacy, rsync};

use workspace::{
    CloseTab, NewHost, OpenSettings, QuickConnect, Quit, Snippets, SplitPane, Workspace,
};

/// "0.1.0 (abc1234)": the workspace version plus the commit it was built from, when known.
pub fn version() -> String {
    let v = env!("CARGO_PKG_VERSION");
    match env!("WIZ_COMMIT") {
        "" => v.to_string(),
        c => format!("{v} ({c})"),
    }
}

/// Print elapsed time since start when SSHC_TIMING is set.
pub fn mark(what: &str) {
    if std::env::var_os("SSHC_TIMING").is_some()
        && let Some(t) = START.get()
    {
        eprintln!("{what}: {:?}", t.elapsed());
    }
}

/// Set at process start; read by `mark`.
static START: std::sync::OnceLock<std::time::Instant> = std::sync::OnceLock::new();

fn main() {
    #[cfg(unix)]
    if std::env::args().nth(1).as_deref() == Some(rsync::runner::BRIDGE_ARG) {
        rsync::runner::bridge_main();
    }
    START.get_or_init(std::time::Instant::now);
    mark("main");
    gpui_platform::application()
        .with_assets(assets::Assets)
        .run(|cx: &mut App| {
            mark("app run");
            mark("before fonts");
            let settings = ssh_core::config::Settings::load();
            prefs::init(settings.clone());
            let ts = cx.text_system().clone();
            // Before any lookup: the text system caches what a family resolves to.
            if let Err(e) = ts.add_fonts(
                assets::FONTS
                    .iter()
                    .map(|f| std::borrow::Cow::Borrowed(*f))
                    .collect(),
            ) {
                eprintln!("Could not load the bundled fonts: {e}");
            }
            theme::init_fonts(&platform::font_exists(&ts), &settings);
            mark("after fonts");
            // Quit goes through Workspace, which refuses while sessions are open (if set).
            cx.on_action(|_: &Quit, cx| {
                let root = cx
                    .windows()
                    .into_iter()
                    .find_map(|w| w.downcast::<Workspace>());
                match root {
                    Some(r) if r.update(cx, Workspace::request_quit).is_ok() => {}
                    _ => cx.quit(),
                }
            });
            shortcuts::apply(cx);
            cx.set_menus(vec![Menu {
                name: "Wiz".into(),
                items: vec![
                    MenuItem::action("Settings...", OpenSettings),
                    MenuItem::action("New Host", NewHost),
                    MenuItem::action("Quick Connect...", QuickConnect),
                    MenuItem::action("Snippets...", Snippets),
                    MenuItem::action("Split Terminal", SplitPane),
                    MenuItem::action("Close Tab", CloseTab),
                    MenuItem::separator(),
                    MenuItem::action("Quit", Quit),
                ],
                disabled: false,
            }]);
            cx.on_window_closed(|cx, _| {
                if cx.windows().is_empty() {
                    cx.quit();
                }
            })
            .detach();

            let bounds = Bounds::centered(None, size(px(1100.), px(700.)), cx);
            cx.open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(bounds)),
                    ..platform::window_options()
                },
                |window, cx| {
                    let workspace = cx.new(|cx| Workspace::new(window, cx));
                    platform::on_window_open(window, &workspace, cx);
                    workspace
                },
            )
            .unwrap();
            mark("window opened");
            platform::on_launch();
            cx.activate(true);
        });
}
