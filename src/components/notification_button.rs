use dioxus::prelude::*;
use notify_rust::Notification;

async fn msg() {
    let body = format!("<b>Start:</b> 10:00\n<a href='https://google.com'>Open</a>");

    let _ = Notification::new()
        .appname("OWA Calendar")
        .summary("🔔 Some event")
        .body(&body)
        .auto_icon()
        .timeout(0)
        // .urgency(notify_rust::Urgency::Critical) // Low, Normal, Critical
        .show_async()
        .await;
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
