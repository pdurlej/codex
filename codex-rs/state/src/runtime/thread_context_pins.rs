use super::*;
use crate::model::ThreadContextPinRow;
use uuid::Uuid;

#[derive(Clone)]
pub struct ThreadContextPinStore {
    pool: Arc<SqlitePool>,
}

impl ThreadContextPinStore {
    pub(crate) fn new(pool: Arc<SqlitePool>) -> Self {
        Self { pool }
    }

    pub async fn list_thread_context_pins(
        &self,
        thread_id: ThreadId,
    ) -> anyhow::Result<Vec<crate::ThreadContextPin>> {
        let rows = sqlx::query(
            r#"
SELECT
    thread_id,
    pin_id,
    text,
    created_at_ms,
    updated_at_ms
FROM thread_context_pins
WHERE thread_id = ?
ORDER BY created_at_ms ASC, pin_id ASC
            "#,
        )
        .bind(thread_id.to_string())
        .fetch_all(self.pool.as_ref())
        .await?;

        rows.into_iter()
            .map(|row| ThreadContextPinRow::try_from_row(&row).and_then(TryInto::try_into))
            .collect()
    }

    pub async fn create_thread_context_pin(
        &self,
        thread_id: ThreadId,
        text: &str,
    ) -> anyhow::Result<crate::ThreadContextPin> {
        let pin_id = Uuid::new_v4().to_string();
        let now_ms = datetime_to_epoch_millis(Utc::now());
        let row = sqlx::query(
            r#"
INSERT INTO thread_context_pins (
    thread_id,
    pin_id,
    text,
    created_at_ms,
    updated_at_ms
) VALUES (?, ?, ?, ?, ?)
RETURNING
    thread_id,
    pin_id,
    text,
    created_at_ms,
    updated_at_ms
            "#,
        )
        .bind(thread_id.to_string())
        .bind(pin_id)
        .bind(text)
        .bind(now_ms)
        .bind(now_ms)
        .fetch_one(self.pool.as_ref())
        .await?;

        ThreadContextPinRow::try_from_row(&row).and_then(TryInto::try_into)
    }

    pub async fn update_thread_context_pin(
        &self,
        thread_id: ThreadId,
        pin_id: &str,
        text: &str,
    ) -> anyhow::Result<Option<crate::ThreadContextPin>> {
        let now_ms = datetime_to_epoch_millis(Utc::now());
        let row = sqlx::query(
            r#"
UPDATE thread_context_pins
SET
    text = ?,
    updated_at_ms = ?
WHERE thread_id = ? AND pin_id = ?
RETURNING
    thread_id,
    pin_id,
    text,
    created_at_ms,
    updated_at_ms
            "#,
        )
        .bind(text)
        .bind(now_ms)
        .bind(thread_id.to_string())
        .bind(pin_id)
        .fetch_optional(self.pool.as_ref())
        .await?;

        row.map(|row| ThreadContextPinRow::try_from_row(&row).and_then(TryInto::try_into))
            .transpose()
    }

    pub async fn delete_thread_context_pin(
        &self,
        thread_id: ThreadId,
        pin_id: &str,
    ) -> anyhow::Result<bool> {
        let result = sqlx::query(
            r#"
DELETE FROM thread_context_pins
WHERE thread_id = ? AND pin_id = ?
            "#,
        )
        .bind(thread_id.to_string())
        .bind(pin_id)
        .execute(self.pool.as_ref())
        .await?;
        Ok(result.rows_affected() > 0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runtime::test_support::test_thread_metadata;
    use crate::runtime::test_support::unique_temp_dir;
    use pretty_assertions::assert_eq;

    #[tokio::test]
    async fn create_list_update_and_delete_thread_context_pin() {
        let codex_home = unique_temp_dir();
        let runtime = StateRuntime::init(codex_home.clone(), "test-provider".to_string())
            .await
            .expect("state db should initialize");
        let thread_id =
            ThreadId::from_string("00000000-0000-0000-0000-000000000123").expect("valid thread id");
        let metadata = test_thread_metadata(&codex_home, thread_id, codex_home.clone());
        runtime
            .upsert_thread(&metadata)
            .await
            .expect("thread should be inserted before pin");

        let created = runtime
            .thread_context_pins()
            .create_thread_context_pin(thread_id, "Remember this")
            .await
            .expect("pin should be created");
        assert_eq!(created.thread_id, thread_id);
        assert_eq!(created.text, "Remember this");

        let listed = runtime
            .thread_context_pins()
            .list_thread_context_pins(thread_id)
            .await
            .expect("pins should be listed");
        assert_eq!(listed, vec![created.clone()]);

        let updated = runtime
            .thread_context_pins()
            .update_thread_context_pin(thread_id, created.pin_id.as_str(), "Remember this instead")
            .await
            .expect("pin should update")
            .expect("pin should exist");
        assert_eq!(updated.pin_id, created.pin_id);
        assert_eq!(updated.text, "Remember this instead");

        let deleted = runtime
            .thread_context_pins()
            .delete_thread_context_pin(thread_id, created.pin_id.as_str())
            .await
            .expect("pin should delete");
        assert!(deleted);

        let listed = runtime
            .thread_context_pins()
            .list_thread_context_pins(thread_id)
            .await
            .expect("pins should be listed after delete");
        assert_eq!(listed, Vec::new());
    }
}
