mod actions;
mod app;
mod assets;
mod data;
mod html;
mod search;
mod theme;
mod ui;

use gpui::{
    App, AppContext as _, Bounds, Menu, MenuItem, TitlebarOptions, WindowBackgroundAppearance,
    WindowBounds, WindowOptions, point, px, size,
};

use crate::{actions::Quit, app::PigeonApp, data::MailData, theme::Theme};

fn main() {
    let data = MailData::load().expect("bundled mailbox.json should parse");

    gpui::application()
        .with_assets(assets::Assets)
        .run(move |cx: &mut App| {
            gpui::init(cx);
            actions::bind_keys(cx);
            cx.on_action(|_: &Quit, cx| cx.quit());
            cx.set_menus(vec![
                Menu::new("Pigeon").items([MenuItem::action("Quit Pigeon", Quit)]),
            ]);
            cx.on_window_closed(|cx, _| {
                if cx.windows().is_empty() {
                    cx.quit();
                }
            })
            .detach();

            let bounds = Bounds::centered(None, size(px(1360.), px(860.)), cx);
            let options = WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                window_min_size: Some(size(px(900.), px(560.))),
                titlebar: Some(TitlebarOptions {
                    title: Some("Pigeon".into()),
                    appears_transparent: true,
                    traffic_light_position: Some(point(px(16.), px(18.))),
                }),
                // Panes move the window themselves (see `ui::window_drag_area`).
                app_owns_titlebar_drag: true,
                window_background: WindowBackgroundAppearance::Blurred,
                ..Default::default()
            };

            gpui::open_window(options, cx, |window, cx| {
                Theme::sync(window, cx);
                cx.new(|cx| PigeonApp::new(data, window, cx))
            })
            .expect("failed to open the main window");
            cx.activate(true);
        });
}
