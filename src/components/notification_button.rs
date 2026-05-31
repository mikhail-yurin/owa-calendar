use dioxus::prelude::*;
use notify_rust::Notification;

async fn msg() {
    let body = format!("<b>Start:</b> 10:00\n<a href='https://google.com'>Open</a>");

    let mut n = Notification::new();
    n.appname("OWA Calendar")
        .summary("🔔 Some event")
        .body(&body)
        .auto_icon()
        .timeout(0);
    #[cfg(target_os = "linux")]
    let _ = n.show_async().await;
    #[cfg(not(target_os = "linux"))]
    let _ = n.show();
}

#[component]
pub fn notification_button() -> Element {
    rsx! {
        div {
            id: "form",
            button {
                onclick: move |_| msg(),
                "Test notification"
            }
        }
    }
}
