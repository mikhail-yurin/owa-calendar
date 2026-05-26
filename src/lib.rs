use chrono::{DateTime, Duration as ChronoDuration, Local, Utc};
use dioxus::prelude::spawn;
use notify_rust::Notification;
use reqwest::cookie::{CookieStore, Jar};
use reqwest::header::COOKIE;
use std::collections::HashSet;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use crate::types::{CalendarItem, CalendarResponse, NotificationType};

pub mod components;
pub mod config;
pub mod types;

static SCHEDULED: OnceLock<Mutex<HashSet<(String, DateTime<Utc>, u8)>>> = OnceLock::new();

fn url_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len() * 3);
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char);
            }
            _ => {
                out.push('%');
                out.push(
                    char::from_digit((b >> 4) as u32, 16)
                        .unwrap()
                        .to_ascii_uppercase(),
                );
                out.push(
                    char::from_digit((b & 0xf) as u32, 16)
                        .unwrap()
                        .to_ascii_uppercase(),
                );
            }
        }
    }
    out
}

fn generate_correlation_id() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    format!("{:032X}_{}", ts.as_nanos(), ts.as_millis())
}

fn scheduled_set() -> &'static Mutex<HashSet<(String, DateTime<Utc>, u8)>> {
    SCHEDULED.get_or_init(|| Mutex::new(HashSet::new()))
}

pub async fn authenticate_owa(
    host: &str,
    username: &str,
    password: &str,
) -> Result<Vec<(String, String)>, Box<dyn std::error::Error + Send + Sync>> {
    let jar = Arc::new(Jar::default());
    let client = reqwest::Client::builder()
        .cookie_provider(jar.clone())
        .user_agent("Mozilla/5.0 (X11; Linux x86_64) Chrome/120.0.0.0")
        .build()?;

    let base = host.trim_end_matches('/');
    let auth_url = format!("{}/owa/auth.owa", base);
    let destination = format!("{}/owa/", base);

    let params = [
        ("destination", destination.as_str()),
        ("flags", "4"),
        ("forcedownlevel", "0"),
        ("trusted", "0"),
        ("username", username),
        ("password", password),
        ("isUtf8", "1"),
    ];

    let response = client.post(&auth_url).form(&params).send().await?;
    let status = response.status();
    let final_url = response.url().clone();

    // Если редирект пошёл обратно на logon.aspx — значит неверные кредо
    if final_url.path().contains("logon.aspx") {
        return Err(format!(
            "OWA auth failed: redirected back to login page ({}). Check username/password.",
            final_url
        )
        .into());
    }

    if status.as_u16() == 401 || status.as_u16() == 403 {
        return Err(format!("OWA auth failed: HTTP {}", status).into());
    }

    let owa_url = destination.parse::<reqwest::Url>()?;
    let cookie_header = jar.cookies(&owa_url).ok_or(
        "No cookies received after OWA authentication — check host/username/password in config",
    )?;

    let cookie_str = cookie_header
        .to_str()
        .map_err(|e| format!("Cookie encoding error: {}", e))?;

    let cookies = cookie_str
        .split("; ")
        .filter_map(|pair: &str| {
            let mut parts = pair.splitn(2, '=');
            let name = parts.next()?.trim().to_string();
            let value = parts.next().unwrap_or("").trim().to_string();
            if name.is_empty() {
                None
            } else {
                Some((name, value))
            }
        })
        .collect();

    Ok(cookies)
}

async fn fetch_calendar_folder_id(
    client: &reqwest::Client,
    cookie_string: &str,
    canary: &str,
    config: &config::AppConfig,
) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
    let correlation_id = generate_correlation_id();
    let client_begin = Local::now().format("%Y-%m-%dT%H:%M:%S%.3f").to_string();
    let base = config.calendar.host.trim_end_matches('/');
    let action_id = config.calendar.action_calendar_folders;
    let url = format!(
        "{}/owa/service.svc?action=GetCalendarFolders&EP=1&ID={}&AC=1",
        base, action_id
    );

    let response = client
        .post(&url)
        .header(COOKIE, cookie_string)
        .header("Accept", "*/*")
        .header("Action", "GetCalendarFolders")
        .header("Cache-Control", "no-cache")
        .header("Connection", "keep-alive")
        .header("Content-Length", "0")
        .header("Content-Type", "application/json; charset=UTF-8")
        .header("Origin", base)
        .header("X-OWA-ActionId", action_id.to_string())
        .header("X-OWA-ActionName", "GetCalendarFoldersAction")
        .header("X-OWA-Attempt", "1")
        .header("X-OWA-CANARY", canary)
        .header("X-OWA-ClientBegin", client_begin)
        .header("X-OWA-ClientBuildVersion", &config.calendar.build_version)
        .header("X-OWA-CorrelationId", correlation_id.clone())
        .header("X-OWA-UrlPostData", "%7B%7D")
        .header("X-Requested-With", "XMLHttpRequest")
        .header("client-request-id", correlation_id)
        .send()
        .await?;

    let status = response.status();
    let body = response.text().await?;

    if !status.is_success() {
        return Err(format!("GetCalendarFolders: server returned {}", status).into());
    }

    let json: serde_json::Value = serde_json::from_str(&body)
        .map_err(|e| format!("GetCalendarFolders: failed to parse: {}", e))?;

    // Log full response structure to understand the shape on first run
    // Find the default calendar (IsDefaultCalendar=true), fall back to first entry
    let calendars = json["CalendarGroups"].as_array().and_then(|groups| {
        groups
            .iter()
            .flat_map(|g| g["Calendars"].as_array().into_iter().flatten())
            .find(|c| c["IsDefaultCalendar"].as_bool() == Some(true))
    });

    let folder_id = if let Some(cal) = calendars {
        cal["CalendarFolderId"]["Id"].as_str()
    } else {
        json["CalendarFolders"][0]["FolderId"]["Id"].as_str()
    }
    .ok_or("GetCalendarFolders: cannot find FolderId in response")?
    .to_string();

    Ok(folder_id)
}

async fn fetch_calendar_inner(
    client: &reqwest::Client,
    cookie_string: &str,
    canary: &str,
    config: &config::AppConfig,
    folder_id: &str,
) -> Result<Vec<CalendarItem>, Box<dyn std::error::Error + Send + Sync>> {
    let ten_days_before = (Local::now() - ChronoDuration::days(10))
        .format("%Y-%m-%dT%H:%M:%S%.3f")
        .to_string();
    let three_weeks_after = (Local::now() + ChronoDuration::weeks(3))
        .format("%Y-%m-%dT%H:%M:%S%.3f")
        .to_string();

    let folder_id_encoded = url_encode(folder_id);
    // Payload structure matches the original browser-captured format exactly.
    // folder_id and dates are the only dynamic parts.
    let url_post_data = format!(
        "%7B%22__type%22%3A%22GetCalendarViewJsonRequest%3A%23Exchange%22%2C%22Header%22%3A%7B%22__type%22%3A%22JsonRequestHeaders%3A%23Exchange%22%2C%22RequestServerVersion%22%3A%22V2017_08_18%22%2C%22TimeZoneContext%22%3A%7B%22__type%22%3A%22TimeZoneContext%3A%23Exchange%22%2C%22TimeZoneDefinition%22%3A%7B%22__type%22%3A%22TimeZoneDefinitionType%3A%23Exchange%22%2C%22Id%22%3A%22Russian%20Standard%20Time%22%7D%7D%7D%2C%22Body%22%3A%7B%22__type%22%3A%22GetCalendarViewRequest%3A%23Exchange%22%2C%22CalendarId%22%3A%7B%22__type%22%3A%22TargetFolderId%3A%23Exchange%22%2C%22BaseFolderId%22%3A%7B%22__type%22%3A%22FolderId%3A%23Exchange%22%2C%22Id%22%3A%22{}%22%7D%7D%2C%22RangeStart%22%3A%22{}%22%2C%22RangeEnd%22%3A%22{}%22%7D%7D",
        folder_id_encoded, ten_days_before, three_weeks_after
    );

    let correlation_id = generate_correlation_id();
    let client_begin = Local::now().format("%Y-%m-%dT%H:%M:%S%.3f").to_string();

    let base = config.calendar.host.trim_end_matches('/');
    let action_id = config.calendar.action_calendar_view;
    let calendar_url = format!(
        "{}/owa/service.svc?action=GetCalendarView&EP=1&ID={}&AC=1",
        base, action_id
    );

    let response = client
        .post(calendar_url)
        .header(COOKIE, cookie_string)
        .header("Accept", "*/*")
        .header("Accept-Language", "ru-RU,ru;q=0.9,en-US;q=0.8,en;q=0.7")
        .header("Action", "GetCalendarView")
        .header("Cache-Control", "no-cache")
        .header("Connection", "keep-alive")
        .header("Content-Length", "0")
        .header("Content-Type", "application/json; charset=UTF-8")
        .header("Origin", base)
        .header("Pragma", "no-cache")
        .header("Sec-Fetch-Dest", "empty")
        .header("Sec-Fetch-Mode", "cors")
        .header("Sec-Fetch-Site", "same-origin")
        .header("X-OWA-ActionId", action_id.to_string())
        .header("X-OWA-ActionName", "GetCalendarViewAction_PrefetchMonth")
        .header("X-OWA-Attempt", "1")
        .header("X-OWA-CANARY", canary)
        .header("X-OWA-ClientBegin", client_begin)
        .header("X-OWA-ClientBuildVersion", &config.calendar.build_version)
        .header("X-OWA-CorrelationId", correlation_id.clone())
        .header("X-OWA-UrlPostData", url_post_data)
        .header("X-Requested-With", "XMLHttpRequest")
        .header("client-request-id", correlation_id)
        .header(
            "sec-ch-ua",
            "\"Google Chrome\";v=\"141\", \"Not?A_Brand\";v=\"8\", \"Chromium\";v=\"141\"",
        )
        .header("sec-ch-ua-mobile", "?0")
        .header("sec-ch-ua-platform", "\"Linux\"")
        .send()
        .await?;

    let body = response.text().await?;

    match serde_json::from_str::<CalendarResponse>(&body) {
        Ok(calendar) => {
            if calendar.body.response_code != "NoError" {
                return Err(format!("OWA error: {}", calendar.body.response_code).into());
            }
            Ok(calendar.body.items)
        }
        Err(e) => Err(e.into()),
    }
}

async fn fetch_unread_count_inner(
    client: &reqwest::Client,
    cookie_string: &str,
    canary: &str,
    config: &config::AppConfig,
) -> Result<u32, Box<dyn std::error::Error + Send + Sync>> {
    let url_post_data = "%7B%22__type%22%3A%22GetFolderJsonRequest%3A%23Exchange%22%2C%22Header%22%3A%7B%22__type%22%3A%22JsonRequestHeaders%3A%23Exchange%22%2C%22RequestServerVersion%22%3A%22V2017_08_18%22%2C%22TimeZoneContext%22%3A%7B%22__type%22%3A%22TimeZoneContext%3A%23Exchange%22%2C%22TimeZoneDefinition%22%3A%7B%22__type%22%3A%22TimeZoneDefinitionType%3A%23Exchange%22%2C%22Id%22%3A%22Russian%20Standard%20Time%22%7D%7D%7D%2C%22Body%22%3A%7B%22__type%22%3A%22GetFolderRequest%3A%23Exchange%22%2C%22FolderShape%22%3A%7B%22__type%22%3A%22FolderResponseShape%3A%23Exchange%22%2C%22BaseShape%22%3A%22IdOnly%22%2C%22AdditionalProperties%22%3A%5B%7B%22__type%22%3A%22PropertyUri%3A%23Exchange%22%2C%22FieldURI%22%3A%22folder%3AUnreadCount%22%7D%5D%7D%2C%22FolderIds%22%3A%5B%7B%22__type%22%3A%22DistinguishedFolderId%3A%23Exchange%22%2C%22Id%22%3A%22inbox%22%7D%5D%7D%7D";

    let base = config.calendar.host.trim_end_matches('/');
    let action_id = config.calendar.action_get_folder;
    let service_url = format!(
        "{}/owa/service.svc?action=GetFolder&EP=1&ID={}&AC=1",
        base, action_id
    );

    let correlation_id = generate_correlation_id();
    let client_begin = Local::now().format("%Y-%m-%dT%H:%M:%S%.3f").to_string();

    let response = client
        .post(&service_url)
        .header(COOKIE, cookie_string)
        .header("Accept", "*/*")
        .header("Action", "GetFolder")
        .header("Cache-Control", "no-cache")
        .header("Connection", "keep-alive")
        .header("Content-Length", "0")
        .header("Content-Type", "application/json; charset=UTF-8")
        .header("X-OWA-ActionId", action_id.to_string())
        .header("X-OWA-ActionName", "GetFolderAction")
        .header("X-OWA-Attempt", "1")
        .header("X-OWA-CANARY", canary)
        .header("X-OWA-ClientBegin", client_begin)
        .header("X-OWA-ClientBuildVersion", &config.calendar.build_version)
        .header("X-OWA-CorrelationId", correlation_id.clone())
        .header("X-OWA-UrlPostData", url_post_data)
        .header("X-Requested-With", "XMLHttpRequest")
        .header("client-request-id", correlation_id)
        .send()
        .await?;

    let body = response.text().await?;

    let json: serde_json::Value = serde_json::from_str(&body)?;
    let unread = json["Body"]["ResponseMessages"]["Items"][0]["Folders"][0]["UnreadCount"]
        .as_u64()
        .unwrap_or(0) as u32;

    Ok(unread)
}

pub async fn fetch_all_data(
) -> Result<(Vec<CalendarItem>, u32), Box<dyn std::error::Error + Send + Sync>> {
    let config = config::AppConfig::load().map_err(|e| format!("Failed to load config: {}", e))?;

    let cookies = authenticate_owa(
        &config.calendar.host,
        &config.calendar.username,
        &config.calendar.password,
    )
    .await?;

    let client = reqwest::Client::builder()
        .user_agent("Mozilla/5.0 (X11; Linux x86_64) Chrome/120.0.0.0")
        .build()?;

    let cookie_string = cookies
        .iter()
        .map(|(name, value)| format!("{}={}", name, value))
        .collect::<Vec<_>>()
        .join("; ");

    let canary = cookies
        .iter()
        .find(|(name, _)| name == "X-OWA-CANARY")
        .map(|(_, value)| value.clone())
        .unwrap_or_default();

    let folder_id = fetch_calendar_folder_id(&client, &cookie_string, &canary, &config)
        .await
        .map_err(|e| format!("Failed to discover calendar folder: {}", e))?;
    let calendar_items =
        fetch_calendar_inner(&client, &cookie_string, &canary, &config, &folder_id).await?;
    let unread_count = fetch_unread_count_inner(&client, &cookie_string, &canary, &config).await?;

    Ok((calendar_items, unread_count))
}

pub fn schedule_notifications(events: Vec<CalendarItem>) {
    // Загружаем конфиг для получения времени уведомления
    let notify_minutes = match config::AppConfig::load() {
        Ok(cfg) => cfg.calendar.notify,
        Err(_) => 15, // fallback на 15 минут
    };

    let now = Local::now();

    for event in events {
        if event.is_cancelled.unwrap_or(false) {
            continue;
        }

        let local_start = event.start.with_timezone(&Local);

        // Уведомление за N минут до события
        let notify_min_timeout = local_start - ChronoDuration::minutes(notify_minutes);
        if notify_min_timeout > now {
            let key = (event.subject.clone(), event.start, 0u8);
            let mut set = scheduled_set().lock().unwrap();
            if !set.contains(&key) {
                set.insert(key.clone());
                drop(set);
                let event_clone = event.clone();
                let duration = (notify_min_timeout - now)
                    .to_std()
                    .unwrap_or(Duration::from_secs(0));
                spawn(async move {
                    tokio::time::sleep(duration).await;
                    send_notification(event_clone, NotificationType::FifteenMinutes).await;
                    scheduled_set().lock().unwrap().remove(&key);
                });
            }
        }

        // Уведомление в момент начала события
        if local_start > now {
            let key = (event.subject.clone(), event.start, 1u8);
            let mut set = scheduled_set().lock().unwrap();
            if !set.contains(&key) {
                set.insert(key.clone());
                drop(set);
                let event_clone = event.clone();
                let duration = (local_start - now)
                    .to_std()
                    .unwrap_or(Duration::from_secs(0));
                spawn(async move {
                    tokio::time::sleep(duration).await;
                    send_notification(event_clone, NotificationType::EventStart).await;
                    scheduled_set().lock().unwrap().remove(&key);
                });
            }
        }
    }
}

pub async fn send_notification(event: CalendarItem, notification_type: NotificationType) {
    let local_start = event.start.with_timezone(&Local);
    let location_link = event
        .location
        .as_ref()
        .map(|loc| loc.display_name.as_str())
        .unwrap_or("");
    let location_url = extract_url(location_link);

    let (summary, body) = match notification_type {
        NotificationType::FifteenMinutes => {
            let summary = format!("⏰ Через 15 минут: {}", event.subject);
            let body = format!(
                "<b>Начало:</b> {}\n<a href='{}'>{}</a>",
                local_start.format("%H:%M"),
                location_url,
                location_url
            );
            (summary, body)
        }
        NotificationType::EventStart => {
            let summary = format!("🔔 Началось: {}", event.subject);
            let body = format!(
                "<b>Время:</b> {}\n<a href='{}'>{}</a>",
                local_start.format("%H:%M"),
                location_url,
                location_url
            );
            (summary, body)
        }
    };

    #[cfg(target_os = "linux")]
    let urgency = match notification_type {
        NotificationType::FifteenMinutes => notify_rust::Urgency::Normal,
        NotificationType::EventStart => notify_rust::Urgency::Critical,
    };

    let mut n = Notification::new();
    n.appname("OWA Calendar")
        .summary(&summary)
        .body(&body)
        .auto_icon()
        .timeout(0);
    #[cfg(target_os = "linux")]
    n.urgency(urgency);
    let _ = n.show_async().await;
}

pub fn extract_url(s: &str) -> &str {
    s.split_whitespace()
        .find(|word| word.starts_with("http://") || word.starts_with("https://"))
        .unwrap_or("")
}
