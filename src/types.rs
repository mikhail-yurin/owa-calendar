use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy)]
pub enum NotificationType {
    FifteenMinutes,
    EventStart,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct CalendarResponse {
    #[serde(rename = "Body")]
    pub body: ResponseBody,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct ResponseBody {
    #[serde(rename = "ResponseCode")]
    pub response_code: String,
    #[serde(rename = "Items", default)]
    pub items: Vec<CalendarItem>,
}

#[derive(Debug, Deserialize, Serialize, Clone, PartialEq)]
pub struct CalendarItem {
    #[serde(rename = "Subject")]
    pub subject: String,
    #[serde(rename = "Start")]
    pub start: DateTime<Utc>,
    #[serde(rename = "End")]
    pub end: DateTime<Utc>,
    #[serde(rename = "Location")]
    pub location: Option<Location>,
    #[serde(rename = "IsAllDayEvent")]
    pub is_all_day_event: bool,
    #[serde(rename = "Organizer")]
    pub organizer: Option<Organizer>,
    #[serde(rename = "IsCancelled")]
    pub is_cancelled: Option<bool>,
}

#[derive(Debug, Deserialize, Serialize, Clone, PartialEq)]
pub struct Location {
    #[serde(rename = "DisplayName")]
    pub display_name: String,
}

#[derive(Debug, Deserialize, Serialize, Clone, PartialEq)]
pub struct Organizer {
    #[serde(rename = "Mailbox")]
    pub mailbox: Mailbox,
}

#[derive(Debug, Deserialize, Serialize, Clone, PartialEq)]
pub struct Mailbox {
    #[serde(rename = "Name")]
    pub name: String,
    #[serde(rename = "EmailAddress")]
    pub email_address: String,
}
