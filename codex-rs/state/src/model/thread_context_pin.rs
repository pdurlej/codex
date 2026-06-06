use anyhow::Result;
use chrono::DateTime;
use chrono::Utc;
use codex_protocol::ThreadId;
use sqlx::Row;
use sqlx::sqlite::SqliteRow;

use super::epoch_millis_to_datetime;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ThreadContextPin {
    pub thread_id: ThreadId,
    pub pin_id: String,
    pub text: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

pub(crate) struct ThreadContextPinRow {
    pub thread_id: String,
    pub pin_id: String,
    pub text: String,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
}

impl ThreadContextPinRow {
    pub(crate) fn try_from_row(row: &SqliteRow) -> Result<Self> {
        Ok(Self {
            thread_id: row.try_get("thread_id")?,
            pin_id: row.try_get("pin_id")?,
            text: row.try_get("text")?,
            created_at_ms: row.try_get("created_at_ms")?,
            updated_at_ms: row.try_get("updated_at_ms")?,
        })
    }
}

impl TryFrom<ThreadContextPinRow> for ThreadContextPin {
    type Error = anyhow::Error;

    fn try_from(row: ThreadContextPinRow) -> Result<Self> {
        Ok(Self {
            thread_id: ThreadId::try_from(row.thread_id)?,
            pin_id: row.pin_id,
            text: row.text,
            created_at: epoch_millis_to_datetime(row.created_at_ms)?,
            updated_at: epoch_millis_to_datetime(row.updated_at_ms)?,
        })
    }
}
