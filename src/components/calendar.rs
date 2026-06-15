use crate::types::CalendarItem;
use crate::{config::AppConfig, extract_url};
use chrono::{Datelike, Local, Weekday};
use dioxus::prelude::*;
use std::time::Duration;

use crate::{fetch_all_data, notify_unread_emails, schedule_notifications};

#[component]
pub fn calendar_list() -> Element {
    let mut items = use_signal(|| Vec::<CalendarItem>::new());
    let mut is_loading = use_signal(|| false);
    let mut error_msg = use_signal(|| String::new());
    let mut unread_count: Signal<Option<u32>> = use_signal(|| None);
    let mut prev_unread: Signal<Option<u32>> = use_signal(|| None);

    let mail_url = AppConfig::load().ok()
        .map(|c| format!("{}/owa/#path=/mail/inbox", c.calendar.host.trim_end_matches('/')))
        .unwrap_or_default();

    let mail_url_for_effect = mail_url.clone();
    let mail_url_for_link = mail_url.clone();

    let fetch_data = move |_| {
        let url = mail_url.clone();
        spawn(async move {
            is_loading.set(true);
            match fetch_all_data().await {
                Ok((calendar_items, count)) => {
                    schedule_notifications(calendar_items.clone());
                    is_loading.set(false);
                    items.set(calendar_items);
                    if Some(count) != prev_unread() && count > 0 {
                        spawn(async move { notify_unread_emails(count, &url).await });
                    }
                    prev_unread.set(Some(count));
                    unread_count.set(Some(count));
                }
                Err(e) => {
                    is_loading.set(false);
                    let msg = format!("{}", e);
                    eprintln!("Failed to fetch data: {}", msg);
                    error_msg.set(msg);
                }
            }
        });
    };

    // Auto update interval
    use_effect(move || {
        let mail_url = mail_url_for_effect.clone();
        spawn(async move {
            // Загружаем конфиг один раз при инициализации
            let config = match AppConfig::load() {
                Ok(c) => c,
                Err(e) => {
                    eprintln!("Failed to load config: {}", e);
                    return;
                }
            };

            loop {
                is_loading.set(true);
                match fetch_all_data().await {
                    Ok((calendar_items, count)) => {
                        schedule_notifications(calendar_items.clone());
                        is_loading.set(false);
                        items.set(calendar_items);
                        if Some(count) != prev_unread() && count > 0 {
                            let url = mail_url.clone();
                            spawn(async move { notify_unread_emails(count, &url).await });
                        }
                        prev_unread.set(Some(count));
                        unread_count.set(Some(count));
                    }
                    Err(e) => {
                        is_loading.set(false);
                        eprintln!("Failed to fetch data: {}", e);
                    }
                }

                // Wait before next update
                tokio::time::sleep(Duration::from_secs(config.calendar.fetch * 60)).await;
            }
        });
    });

    let filtered_items: Vec<_> = items
        .read()
        .iter()
        .filter(|event| !event.is_cancelled.unwrap_or(false))
        .cloned()
        .collect();

    // Group events by date
    use std::collections::HashMap;
    let mut events_by_date: HashMap<String, Vec<CalendarItem>> = HashMap::new();

    for event in filtered_items {
        let local_start = event.start.with_timezone(&Local);
        let date_key = local_start.format("%Y-%m-%d").to_string();
        events_by_date
            .entry(date_key)
            .or_insert_with(Vec::new)
            .push(event);
    }

    // Sort dates
    let mut dates: Vec<String> = events_by_date.keys().cloned().collect();
    dates.sort();

    let today = Local::now();
    let today_string = today.format("%Y-%m-%d").to_string();
    let today_title = today.format("%d.%m.%Y").to_string();

    rsx! {
        div {
            div { style: "position: fixed; display: flex; justify-content: space-between; align-items: center; width: calc(100% - 4 * 8px); padding: 12px 8px 8px; background-color: #f8f3ef",
                div {
                    button { onclick: fetch_data, disabled: is_loading(), "Refresh" }
                }
                div { style: "font-size: 22px; font-weight: 700; flex: 1; display: flex; align-items: center; justify-content: center; gap: 12px;",
                    if is_loading() {
                        span { "Calendar is loading..." }
                    } else {
                        span { "{&today_title}" }
                        {
                            let (label, label_style) = match unread_count() {
                                Some(n) if n > 0 => {
                                    (
                                        format!("непрочитанных писем: {}", n),
                                        "font-weight: 700; color: #cc2200;",
                                    )
                                }
                                _ => {
                                    (
                                        "Открыть почту в браузере".to_string(),
                                        "font-weight: 400; color: #555;",
                                    )
                                }
                            };
                            rsx! {
                                a {
                                    style: "font-size: 14px; cursor: pointer; text-decoration: underline; {label_style}",
                                    onclick: move |_| {
                                        AppConfig::open_url_in_default_browser(&mail_url_for_link);
                                    },
                                    "{label}"
                                }
                            }
                        }
                    }
                }
                div {
                    span { style: "margin-right: 8px; font-size: 12px; color: #888;",
                        "v{env!(\"CARGO_PKG_VERSION\")}"
                    }
                    button {
                        onclick: |_| {
                            let path = AppConfig::get_config_path();
                            AppConfig::open_file_in_default_app(&path);
                        },
                        "Settings"
                    }
                }
            }

            div {
                // onkeydown: on_keydown,
                tabindex: 0,
                style: "display: grid; grid-template-columns: repeat(5, 1fr); gap: 8px; padding: 8px; max-width: 100vw; outline: none; padding-top: 50px;",
                // todo: check no connection
                if dates.is_empty() && !is_loading() {
                    div { style: "grid-column: 1 / -1; text-align: center; padding: 40px; font-size: 24px;",
                        "No connection"
                        if !error_msg().is_empty() {
                            div { style: "margin-top: 12px; font-size: 14px; color: #ff6b6b; font-family: monospace;",
                                "{error_msg}"
                            }
                        }
                    }
                }
                {
                    dates
                        .iter()
                        .skip_while(|date| {
                            let events = &events_by_date[*date];
                            let local_start = events[0].start.with_timezone(&Local);
                            local_start.weekday() != Weekday::Mon
                        })
                        .map(|date| {
                            let events = &events_by_date[date];
                            let local_start = events[0].start.with_timezone(&Local);
                            let weekday = local_start.weekday();
                            let today_color = "#f1e8ddff";
                            let display_date = local_start.format("%d.%m.%Y").to_string();
                            let border_color = if date == &today_string {
                                today_color
                            } else {
                                "#f2ded7"
                            };
                            let border_width = if date == &today_string { "2px" } else { "1px" };
                            let background = if date == &today_string {
                                today_color
                            } else {
                                "unset"
                            };
                            rsx! {
                                div { style: "border: {border_width} solid {border_color}; padding: 8px; display: flex; flex-direction: column; background-color: {background};",
                                    div { style: "font-weight: bold; margin-bottom: 8px;", "{weekday}, {display_date}" }
                                    {
                                        events
                                            .iter()
                                            .map(|event| {
                                                let location = event
                                                    .location
                                                    .as_ref()
                                                    .map_or(String::new(), |loc| loc.display_name.clone());
                                                let location_url = extract_url(location.as_str());
                                                let local_start = event.start.with_timezone(&Local);
                                                let local_end = event.end.with_timezone(&Local);
                                                let start_time = local_start.format("%H:%M").to_string();
                                                let end_time = local_end.format("%H:%M").to_string();
                                                let now = Local::now();
                                                let is_current_event = now >= local_start && now <= local_end;
                                                let past_event_color = "#aaaaaaff";
                                                let subject_style = if is_current_event {
                                                    "font-weight: bold;".to_string()
                                                } else if now > local_end {
                                                    format!("text-decoration: line-through; color: {}", past_event_color)
                                                } else {
                                                    String::new()
                                                };

                                                rsx! {
                                                    div { style: "margin-top: 12px; font-size: 0.9em;",
                                                        div { style: "display: flex; justify-content: space-between;",
                                                            div { style: "{subject_style}", "{event.subject}" }
                                                            div { style: "white-space: nowrap;", "{start_time} - {end_time}" }
                                                        }
                                                        if now <= local_end {
                                                            if !location_url.is_empty() {
                                                                a { href: "{location_url}", style: "font-size: 0.8em;", "{location_url}" }
                                                            }
                                                        }
                                                    }
                                                }
                                            })
                                    }
                                }
                            }
                        })
                }
            }
        }
    }
}
