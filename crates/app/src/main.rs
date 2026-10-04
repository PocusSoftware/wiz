mod docker_view;
mod host_form;
mod input;
mod root;
mod runtime;
mod terminal_view;
mod theme;
mod ui;

use gpui::{
    App, Application, Bounds, KeyBinding, Menu, MenuItem, TitlebarOptions, WindowBounds,
    WindowOptions, prelude::*, px, size,
};

use root::{CloseTab, NewHost, NextTab, PrevTab, Quit, Root};

fn main() {
    Application::new().run(|cx: &mut App| {
        cx.on_action(|_: &Quit, cx| cx.quit());
        cx.bind_keys([
            KeyBinding::new("cmd-q", Quit, None),
            KeyBinding::new("cmd-w", CloseTab, None),
            KeyBinding::new("cmd-shift-]", NextTab, None),
            KeyBinding::new("cmd-shift-[", PrevTab, None),
            KeyBinding::new("ctrl-tab", NextTab, None),
            KeyBinding::new("ctrl-shift-tab", PrevTab, None),
            KeyBinding::new("cmd-n", NewHost, None),
        ]);
        cx.set_menus(vec![Menu {
            name: "ssh-client".into(),
            items: vec![
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
                    title: Some("ssh-client".into()),
                    ..Default::default()
                }),
                ..Default::default()
            },
            |_, cx| cx.new(Root::new),
        )
        .unwrap();
        cx.activate(true);
    });
}
