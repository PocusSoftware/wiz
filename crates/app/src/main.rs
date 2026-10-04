use gpui::{App, Application, Bounds, Context, Window, WindowBounds, WindowOptions, div, prelude::*, px, rgb, size};

struct Hello;
impl Render for Hello {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div().size_full().bg(rgb(0x1e1e2e)).text_color(rgb(0xcdd6f4)).p_4().child("ssh-client")
    }
}

fn main() {
    Application::new().run(|cx: &mut App| {
        let bounds = Bounds::centered(None, size(px(900.), px(600.)), cx);
        cx.open_window(
            WindowOptions { window_bounds: Some(WindowBounds::Windowed(bounds)), ..Default::default() },
            |_, cx| cx.new(|_| Hello),
        )
        .unwrap();
        cx.activate(true);
    });
}
