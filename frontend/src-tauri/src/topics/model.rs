//! Topic types, serialized camelCase like the action-item types.

use serde::{Deserialize, Serialize};
use sqlx::FromRow;

#[derive(Debug, Clone, FromRow, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Topic {
    pub id: String,
    pub name: String,
    pub description: String,
    pub status: String,
    pub notes_markdown: Option<String>,
    pub notes_json: Option<String>,
    pub created_at: String,
    pub updated_at: String,
    pub closed_at: Option<String>,
}

#[derive(Debug, Clone, FromRow, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct TopicSummary {
    pub id: String,
    pub name: String,
    pub description: String,
    pub status: String,
    pub last_activity: Option<String>,
    pub pending_count: i64,
    pub meeting_count: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Bullet {
    pub text: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub t: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Decision {
    pub id: String,
    pub text: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub t: Option<f64>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct EntryBody {
    #[serde(default)]
    pub bullets: Vec<Bullet>,
    #[serde(default)]
    pub decisions: Vec<Decision>,
}
