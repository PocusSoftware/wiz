mod anim;
mod assets;
mod docker_view;
mod host_form;
mod input;
mod log_view;
mod prefs;
mod privacy;
mod root;
mod rsync;
mod runtime;
mod settings_view;
mod sftp_view;
mod terminal_view;
mod theme;
mod ui;
mod window_drag;

use gpui::{
    App, Application, Bounds, KeyBinding, Menu, MenuItem, TitlebarOptions, WindowBounds,
    WindowOptions, point, prelude::*, px, size,
};

use root::{CloseTab, NewHost, NextTab, OpenSettings, PrevTab, Quit, Root};

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

/// Show the castle in the Dock even when running unbundled (cargo run).
fn set_dock_icon() {
    use objc2::{AnyThread, MainThreadMarker};
    use objc2_app_kit::{NSApplication, NSImage};
    use objc2_foundation::NSData;
    let Some(mtm) = MainThreadMarker::new() else {
        return;
    };
    let data = NSData::with_bytes(include_bytes!("../assets/brand/icon-512.png"));
    if let Some(img) = NSImage::initWithData(NSImage::alloc(), &data) {
        // SAFETY: called on the main thread with a valid image.
        unsafe { NSApplication::sharedApplication(mtm).setApplicationIconImage(Some(&img)) };
    }
}

fn main() {
    START.get_or_init(std::time::Instant::now);
    mark("main");
    Application::new()
        .with_assets(assets::Assets)
        .run(|cx: &mut App| {
            mark("app run");
            mark("before fonts");
            let settings = ssh_core::config::Settings::load();
            prefs::init(settings.clone());
            let ts = cx.text_system().clone();
            // An unknown family resolves to the fallback font, so a candidate exists
            // exactly when it resolves to something else.
            let fallback = ts.resolve_font(&gpui::font("\u{1}no-such-font"));
            theme::init_fonts(
                &|name| ts.resolve_font(&gpui::font(name.to_string())) != fallback,
                &settings,
            );
            mark("after fonts");
            cx.on_action(|_: &Quit, cx| cx.quit());
            cx.bind_keys([
                KeyBinding::new("cmd-q", Quit, None),
                KeyBinding::new("cmd-w", CloseTab, None),
                KeyBinding::new("cmd-shift-]", NextTab, None),
                KeyBinding::new("cmd-shift-[", PrevTab, None),
                KeyBinding::new("ctrl-tab", NextTab, None),
                KeyBinding::new("ctrl-shift-tab", PrevTab, None),
                KeyBinding::new("cmd-n", NewHost, None),
                KeyBinding::new("cmd-,", OpenSettings, None),
            ]);
            cx.set_menus(vec![Menu {
                name: "Bastion".into(),
                items: vec![
                    MenuItem::action("Settings...", OpenSettings),
                    MenuItem::action("New Host", NewHost),
                    MenuItem::action("Close Tab", CloseTab),
                    MenuItem::separator(),
                    MenuItem::action("Quit", Quit),
                ],
            }]);
            cx.on_window_closed(|cx| {
                if cx.windows().is_empty() {
                    cx.quit();
                }
            })
            .detach();

            let bounds = Bounds::centered(None, size(px(1100.), px(700.)), cx);
            cx.open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(bounds)),
                    titlebar: Some(TitlebarOptions {
                        title: Some("Bastion".into()),
                        appears_transparent: true,
                        // Native buttons are parked off-screen; the title bar draws its own.
                        traffic_light_position: Some(point(px(-100.), px(-100.))),
                    }),
                    ..Default::default()
                },
                |_, cx| cx.new(Root::new),
            )
            .unwrap();
            mark("window opened");
            set_dock_icon();
            cx.activate(true);
        });
}
