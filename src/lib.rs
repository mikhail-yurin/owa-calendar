use base64::Engine;
use chrono::{DateTime, Duration as ChronoDuration, Local, Utc};
use dioxus::prelude::spawn;
use notify_rust::Notification;
use reqwest::cookie::{CookieStore, Jar};
use reqwest::header::{AUTHORIZATION, WWW_AUTHENTICATE};
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

/// Everything needed to call OWA's service.svc after a successful login.
/// Session cookies (incl. canary) live in the shared cookie jar and get
/// attached to every request automatically by the client that performed
/// the login — `basic_auth_header` is the one exception, since Basic auth
/// isn't remembered by the server and has to be re-sent on every call.
pub struct OwaSession {
    pub canary: String,
    pub basic_auth_header: Option<String>,
}

/// `DOMAIN\username` -> `("DOMAIN", "username")`. NTLM needs the domain as
/// a separate field; other formats (UPN, bare username) are passed through
/// with an empty domain.
fn split_domain_username(username: &str) -> (String, String) {
    match username.split_once('\\') {
        Some((domain, user)) => (domain.to_string(), user.to_string()),
        None => (String::new(), username.to_string()),
    }
}

fn canary_from_jar(jar: &Jar, url: &reqwest::Url) -> String {
    let Some(header) = jar.cookies(url) else {
        return String::new();
    };
    let Ok(cookie_str) = header.to_str() else {
        return String::new();
    };
    cookie_str
        .split("; ")
        .filter_map(|pair| pair.split_once('='))
        .find(|(name, _)| *name == "X-OWA-CANARY")
        .map(|(_, value)| value.trim().to_string())
        .unwrap_or_default()
}

/// Legacy OWA "forms" login: POST credentials to auth.owa and collect the
/// resulting session cookies. Still used by OWA deployments that show an
/// in-page login form rather than a login popup.
///
/// Returns `Err((wants_challenge_auth, message))`, where the flag signals
/// that the server answered with an HTTP auth challenge (401/403) instead
/// of processing the form — the caller should fall back to Basic/NTLM.
async fn authenticate_owa_forms(
    client: &reqwest::Client,
    jar: &Jar,
    host: &str,
    username: &str,
    password: &str,
) -> Result<OwaSession, (bool, String)> {
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

    let response = client
        .post(&auth_url)
        .form(&params)
        .send()
        .await
        .map_err(|e| (false, e.to_string()))?;
    let status = response.status();
    let final_url = response.url().clone();
    // Drain the body so the connection is returned to the pool before the
    // next request goes out — otherwise reqwest may open a second
    // connection instead of reusing this one.
    let _ = response.bytes().await;

    // Если редирект пошёл обратно на logon.aspx — значит неверные кредо
    // (старая форма ещё жива и она их отвергла — пробовать Basic/NTLM смысла нет)
    if final_url.path().contains("logon.aspx") {
        return Err((
            false,
            format!(
                "OWA auth failed: redirected back to login page ({}). Check username/password.",
                final_url
            ),
        ));
    }

    // 401/403 без редиректа на logon.aspx означает, что auth.owa либо не
    // существует, либо блокируется гейтом перед ней — сигнализируем
    // вызывающему коду попробовать Basic/NTLM вместо старой формы.
    if status.as_u16() == 401 || status.as_u16() == 403 {
        return Err((true, format!("OWA auth failed: HTTP {}", status)));
    }

    let owa_url = destination
        .parse::<reqwest::Url>()
        .map_err(|e| (false, e.to_string()))?;
    Ok(OwaSession {
        canary: canary_from_jar(jar, &owa_url),
        basic_auth_header: None,
    })
}

/// OWA login via HTTP Basic auth.
async fn authenticate_owa_basic(
    client: &reqwest::Client,
    jar: &Jar,
    owa_url: &str,
    username: &str,
    password: &str,
) -> Result<OwaSession, Box<dyn std::error::Error + Send + Sync>> {
    let auth_header = format!(
        "Basic {}",
        base64::engine::general_purpose::STANDARD.encode(format!("{}:{}", username, password))
    );

    let response = client
        .get(owa_url)
        .header(AUTHORIZATION, &auth_header)
        .send()
        .await?;
    let status = response.status();
    let _ = response.bytes().await;

    if status.as_u16() == 401 || status.as_u16() == 403 {
        return Err(format!(
            "OWA Basic auth failed: HTTP {}. Check username/password.",
            status
        )
        .into());
    }

    let url = owa_url.parse::<reqwest::Url>()?;
    Ok(OwaSession {
        canary: canary_from_jar(jar, &url),
        // Basic doesn't ride along with the session cookie, so it has to
        // be re-attached to every subsequent request.
        basic_auth_header: Some(auth_header),
    })
}

/// OWA login via NTLM. Unlike Forms/Basic, NTLM authenticates the
/// underlying TCP connection rather than individual requests, so the
/// handshake and every subsequent OWA call must share the same
/// `reqwest::Client` (and, in practice, its pooled connection).
async fn authenticate_owa_ntlm(
    client: &reqwest::Client,
    jar: &Jar,
    owa_url: &str,
    username: &str,
    password: &str,
) -> Result<OwaSession, Box<dyn std::error::Error + Send + Sync>> {
    let (domain, user) = split_domain_username(username);
    let creds = ntlmclient::Credentials {
        username: user,
        password: password.to_string(),
        domain,
    };
    const WORKSTATION: &str = "owa-calendar";

    let negotiate_msg = ntlmclient::Message::Negotiate(ntlmclient::NegotiateMessage {
        flags: ntlmclient::Flags::NEGOTIATE_UNICODE
            | ntlmclient::Flags::REQUEST_TARGET
            | ntlmclient::Flags::NEGOTIATE_NTLM
            | ntlmclient::Flags::NEGOTIATE_WORKSTATION_SUPPLIED,
        supplied_domain: String::new(),
        supplied_workstation: WORKSTATION.to_string(),
        os_version: Default::default(),
    });
    let negotiate_b64 = base64::engine::general_purpose::STANDARD.encode(negotiate_msg.to_bytes()?);

    let challenge_response = client
        .get(owa_url)
        .header(AUTHORIZATION, format!("NTLM {}", negotiate_b64))
        .send()
        .await?;

    let challenge_header = challenge_response
        .headers()
        .get(WWW_AUTHENTICATE)
        .ok_or("OWA NTLM auth failed: server didn't send a Type2 challenge")?
        .to_str()
        .map_err(|e| format!("OWA NTLM auth failed: unreadable challenge header: {}", e))?
        .to_string();
    let auth_url = challenge_response.url().clone();
    // Drain the body before the next request, same reasoning as above.
    let _ = challenge_response.bytes().await;

    let challenge_b64 = challenge_header
        .split(' ')
        .nth(1)
        .ok_or("OWA NTLM auth failed: malformed WWW-Authenticate header")?;
    let challenge_bytes = base64::engine::general_purpose::STANDARD.decode(challenge_b64)?;
    let challenge = match ntlmclient::Message::try_from(challenge_bytes.as_slice())? {
        ntlmclient::Message::Challenge(c) => c,
        _ => return Err("OWA NTLM auth failed: server didn't send a Type2 challenge".into()),
    };
    let target_info_bytes: Vec<u8> = challenge
        .target_information
        .iter()
        .flat_map(|entry| entry.to_bytes())
        .collect();

    let response = ntlmclient::respond_challenge_ntlm_v2(
        challenge.challenge,
        &target_info_bytes,
        ntlmclient::get_ntlm_time(),
        &creds,
    );
    let auth_flags = ntlmclient::Flags::NEGOTIATE_UNICODE | ntlmclient::Flags::NEGOTIATE_NTLM;
    let authenticate_msg = response.to_message(&creds, WORKSTATION, auth_flags);
    let authenticate_b64 =
        base64::engine::general_purpose::STANDARD.encode(authenticate_msg.to_bytes()?);

    let final_response = client
        .get(auth_url.clone())
        .header(AUTHORIZATION, format!("NTLM {}", authenticate_b64))
        .send()
        .await?;
    let status = final_response.status();
    // Drain the body before returning — the caller immediately issues more
    // requests on this same client and needs the connection back in the pool.
    let _ = final_response.bytes().await;
    if status.as_u16() == 401 || status.as_u16() == 403 {
        return Err(format!(
            "OWA NTLM auth failed: HTTP {}. Check username/password/domain.",
            status
        )
        .into());
    }

    Ok(OwaSession {
        canary: canary_from_jar(jar, &auth_url),
        basic_auth_header: None,
    })
}

/// Falls back to whichever HTTP auth scheme the server actually challenges
/// with (found by probing `{host}/owa/` anonymously), used when the legacy
/// forms login is no longer accepted.
async fn authenticate_owa_challenge(
    client: &reqwest::Client,
    jar: &Jar,
    host: &str,
    username: &str,
    password: &str,
) -> Result<OwaSession, Box<dyn std::error::Error + Send + Sync>> {
    let base = host.trim_end_matches('/');
    let owa_url = format!("{}/owa/", base);

    let probe = client.get(&owa_url).send().await?;
    let schemes: Vec<String> = probe
        .headers()
        .get_all(WWW_AUTHENTICATE)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .map(|v| v.to_ascii_lowercase())
        .collect();
    // Drain the body before the next request, same reasoning as above.
    let _ = probe.bytes().await;

    if schemes.iter().any(|s| s.contains("ntlm")) {
        authenticate_owa_ntlm(client, jar, &owa_url, username, password).await
    } else if schemes.iter().any(|s| s.contains("basic")) {
        authenticate_owa_basic(client, jar, &owa_url, username, password).await
    } else {
        Err(format!(
            "OWA auth failed: server challenge at {} didn't offer a supported scheme (got: {})",
            owa_url,
            schemes.join(", ")
        )
        .into())
    }
}

pub async fn authenticate_owa(
    client: &reqwest::Client,
    jar: &Jar,
    host: &str,
    username: &str,
    password: &str,
) -> Result<OwaSession, Box<dyn std::error::Error + Send + Sync>> {
    match authenticate_owa_forms(client, jar, host, username, password).await {
        Ok(session) => Ok(session),
        Err((wants_challenge_auth, forms_err)) if wants_challenge_auth => {
            authenticate_owa_challenge(client, jar, host, username, password)
                .await
                .map_err(|challenge_err| {
                    format!(
                        "Forms auth failed ({}); Basic/NTLM fallback also failed: {}",
                        forms_err, challenge_err
                    )
                    .into()
                })
        }
        Err((_, forms_err)) => Err(forms_err.into()),
    }
}

fn apply_auth(builder: reqwest::RequestBuilder, session: &OwaSession) -> reqwest::RequestBuilder {
    match &session.basic_auth_header {
        Some(header) => builder.header(AUTHORIZATION, header),
        None => builder,
    }
}

async fn fetch_calendar_folder_id(
    client: &reqwest::Client,
    session: &OwaSession,
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

    let response = apply_auth(client.post(&url), session)
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
        .header("X-OWA-CANARY", &session.canary)
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
    session: &OwaSession,
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

    let response = apply_auth(client.post(calendar_url), session)
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
        .header("X-OWA-CANARY", &session.canary)
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
    session: &OwaSession,
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

    let response = apply_auth(client.post(&service_url), session)
        .header("Accept", "*/*")
        .header("Action", "GetFolder")
        .header("Cache-Control", "no-cache")
        .header("Connection", "keep-alive")
        .header("Content-Length", "0")
        .header("Content-Type", "application/json; charset=UTF-8")
        .header("X-OWA-ActionId", action_id.to_string())
        .header("X-OWA-ActionName", "GetFolderAction")
        .header("X-OWA-Attempt", "1")
        .header("X-OWA-CANARY", &session.canary)
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

    // Cookies (incl. canary) and, for NTLM, the authenticated TCP connection
    // itself must be shared between the login and every subsequent OWA
    // call, so both use this one client/jar for the whole session.
    let jar = Arc::new(Jar::default());
    let client = reqwest::Client::builder()
        .cookie_provider(jar.clone())
        .user_agent("Mozilla/5.0 (X11; Linux x86_64) Chrome/120.0.0.0")
        .build()?;

    let session = authenticate_owa(
        &client,
        &jar,
        &config.calendar.host,
        &config.calendar.username,
        &config.calendar.password,
    )
    .await?;

    let folder_id = fetch_calendar_folder_id(&client, &session, &config)
        .await
        .map_err(|e| format!("Failed to discover calendar folder: {}", e))?;
    let calendar_items = fetch_calendar_inner(&client, &session, &config, &folder_id).await?;
    let unread_count = fetch_unread_count_inner(&client, &session, &config).await?;

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
    #[cfg(target_os = "linux")]
    let _ = n.show_async().await;
    #[cfg(not(target_os = "linux"))]
    let _ = n.show();
}

pub async fn notify_unread_emails(count: u32, mail_url: &str) {
    let summary = format!("📬 У вас {} непрочитанных писем", count);
    let body = format!("<a href='{}'>{}</a>", mail_url, mail_url);

    let mut n = Notification::new();
    n.appname("OWA Calendar")
        .summary(&summary)
        .body(&body)
        .auto_icon()
        .timeout(0);
    #[cfg(target_os = "linux")]
    let _ = n.show_async().await;
    #[cfg(not(target_os = "linux"))]
    let _ = n.show();
}

pub fn extract_url(s: &str) -> &str {
    s.split_whitespace()
        .find(|word| word.starts_with("http://") || word.starts_with("https://"))
        .unwrap_or("")
}
