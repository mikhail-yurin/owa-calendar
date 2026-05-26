use dioxus::{
    desktop::{use_window, Config, WindowBuilder},
    prelude::*,
};
use owa_calendar::components::calendar::calendar_list;

const FAVICON: Asset = asset!("/assets/favicon.ico");
const MAIN_CSS: Asset = asset!("/assets/main.css");

fn main() {
    // Load icon from ICO file
    let icon = include_bytes!("../assets/favicon.ico");
    let (icon_rgba, icon_width, icon_height) = {
        let image = image::load_from_memory(icon)
            .expect("Failed to load icon")
            .into_rgba8();
        let (width, height) = image.dimensions();
        let rgba = image.into_raw();
        (rgba, width, height)
    };

    dioxus::LaunchBuilder::new()
        .with_cfg(
            Config::default().with_menu(None).with_window(
                WindowBuilder::new()
                    .with_maximized(true)
                    .with_decorations(true)
                    .with_resizable(false)
                    .with_window_icon(Some(
                        dioxus::desktop::tao::window::Icon::from_rgba(
                            icon_rgba,
                            icon_width,
                            icon_height,
                        )
                        .expect("Failed to create icon"),
                    ))
                    .with_title("OWA Calendar"),
            ),
        )
        .launch(App);
}

#[component]
fn App() -> Element {
    // Adjust window size on first render
    use_effect(move || {
        let window = use_window();

        if let Some(monitor) = window.current_monitor() {
            let work_area = monitor.size();
            let scale_factor = window.scale_factor();

            // Leave margin for taskbar/decorations
            let width = ((work_area.width as f64 / scale_factor - 70.0).max(800.0)) as u32;
            let height = ((work_area.height as f64 / scale_factor - 40.0).max(600.0)) as u32;

            window.set_inner_size(dioxus::desktop::tao::dpi::LogicalSize::new(width, height));
        }
    });

    rsx! {
        document::Link { rel: "icon", href: FAVICON }
        document::Link { rel: "stylesheet", href: MAIN_CSS }
        MainPage {}
    }
}

#[component]
pub fn MainPage() -> Element {
    rsx! {
        div {
            id: "Component_Main_Page",
            calendar_list {}
        }
    }
}
