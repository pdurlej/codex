use super::*;
use codex_protocol::protocol::AdditionalContextEntry as CoreAdditionalContextEntry;
use codex_protocol::protocol::AdditionalContextKind as CoreAdditionalContextKind;

const MAX_CONTEXT_PINS_PER_THREAD: usize = 16;
const MAX_CONTEXT_PIN_TEXT_BYTES: usize = 16 * 1024;
const MAX_CONTEXT_PIN_TOTAL_TEXT_BYTES: usize = 64 * 1024;
const DEFAULT_CONTEXT_PIN_LIST_LIMIT: usize = MAX_CONTEXT_PINS_PER_THREAD;
const CONTEXT_PIN_ADDITIONAL_CONTEXT_KEY_PREFIX: &str = "codex_context_pin_";

#[derive(Clone)]
pub(crate) struct ThreadContextPinsRequestProcessor {
    thread_manager: Arc<ThreadManager>,
    config: Arc<Config>,
    state_db: Option<StateDbHandle>,
}

impl ThreadContextPinsRequestProcessor {
    pub(crate) fn new(
        thread_manager: Arc<ThreadManager>,
        config: Arc<Config>,
        state_db: Option<StateDbHandle>,
    ) -> Self {
        Self {
            thread_manager,
            config,
            state_db,
        }
    }

    pub(crate) async fn thread_context_pin_list(
        &self,
        params: ThreadContextPinListParams,
    ) -> Result<Option<ClientResponsePayload>, JSONRPCErrorError> {
        self.thread_context_pin_list_inner(params)
            .await
            .map(|response| Some(response.into()))
    }

    pub(crate) async fn thread_context_pin_create(
        &self,
        params: ThreadContextPinCreateParams,
    ) -> Result<Option<ClientResponsePayload>, JSONRPCErrorError> {
        self.thread_context_pin_create_inner(params)
            .await
            .map(|response| Some(response.into()))
    }

    pub(crate) async fn thread_context_pin_update(
        &self,
        params: ThreadContextPinUpdateParams,
    ) -> Result<Option<ClientResponsePayload>, JSONRPCErrorError> {
        self.thread_context_pin_update_inner(params)
            .await
            .map(|response| Some(response.into()))
    }

    pub(crate) async fn thread_context_pin_delete(
        &self,
        params: ThreadContextPinDeleteParams,
    ) -> Result<Option<ClientResponsePayload>, JSONRPCErrorError> {
        self.thread_context_pin_delete_inner(params)
            .await
            .map(|response| Some(response.into()))
    }

    async fn thread_context_pin_list_inner(
        &self,
        params: ThreadContextPinListParams,
    ) -> Result<ThreadContextPinListResponse, JSONRPCErrorError> {
        let thread_id = parse_thread_id_for_context_pin_request(params.thread_id.as_str())?;
        let state_db = self.state_db_for_materialized_thread(thread_id).await?;
        let pins = state_db
            .thread_context_pins()
            .list_thread_context_pins(thread_id)
            .await
            .map_err(|err| internal_error(format!("failed to list thread context pins: {err}")))?
            .into_iter()
            .map(api_thread_context_pin_from_state)
            .collect::<Vec<_>>();
        let (data, next_cursor) =
            paginate_context_pins(pins, params.cursor.as_deref(), params.limit)?;
        Ok(ThreadContextPinListResponse { data, next_cursor })
    }

    async fn thread_context_pin_create_inner(
        &self,
        params: ThreadContextPinCreateParams,
    ) -> Result<ThreadContextPinCreateResponse, JSONRPCErrorError> {
        validate_context_pin_text(&params.text)?;
        let thread_id = parse_thread_id_for_context_pin_request(params.thread_id.as_str())?;
        let state_db = self.state_db_for_materialized_thread(thread_id).await?;
        self.reconcile_context_pin_rollout(thread_id, &state_db)
            .await?;
        let existing = state_db
            .thread_context_pins()
            .list_thread_context_pins(thread_id)
            .await
            .map_err(|err| internal_error(format!("failed to list thread context pins: {err}")))?;
        validate_create_context_pin_limits(&existing, &params.text)?;

        let pin = state_db
            .thread_context_pins()
            .create_thread_context_pin(thread_id, params.text.as_str())
            .await
            .map_err(|err| internal_error(format!("failed to create thread context pin: {err}")))?;
        Ok(ThreadContextPinCreateResponse {
            pin: api_thread_context_pin_from_state(pin),
        })
    }

    async fn thread_context_pin_update_inner(
        &self,
        params: ThreadContextPinUpdateParams,
    ) -> Result<ThreadContextPinUpdateResponse, JSONRPCErrorError> {
        validate_context_pin_id(&params.pin_id)?;
        validate_context_pin_text(&params.text)?;
        let thread_id = parse_thread_id_for_context_pin_request(params.thread_id.as_str())?;
        let state_db = self.state_db_for_materialized_thread(thread_id).await?;
        let existing = state_db
            .thread_context_pins()
            .list_thread_context_pins(thread_id)
            .await
            .map_err(|err| internal_error(format!("failed to list thread context pins: {err}")))?;
        if !existing.iter().any(|pin| pin.pin_id == params.pin_id) {
            return Ok(ThreadContextPinUpdateResponse { pin: None });
        }
        validate_update_context_pin_limits(&existing, params.pin_id.as_str(), &params.text)?;

        let pin = state_db
            .thread_context_pins()
            .update_thread_context_pin(thread_id, params.pin_id.as_str(), params.text.as_str())
            .await
            .map_err(|err| internal_error(format!("failed to update thread context pin: {err}")))?
            .map(api_thread_context_pin_from_state);
        Ok(ThreadContextPinUpdateResponse { pin })
    }

    async fn thread_context_pin_delete_inner(
        &self,
        params: ThreadContextPinDeleteParams,
    ) -> Result<ThreadContextPinDeleteResponse, JSONRPCErrorError> {
        validate_context_pin_id(&params.pin_id)?;
        let thread_id = parse_thread_id_for_context_pin_request(params.thread_id.as_str())?;
        let state_db = self.state_db_for_materialized_thread(thread_id).await?;
        let deleted = state_db
            .thread_context_pins()
            .delete_thread_context_pin(thread_id, params.pin_id.as_str())
            .await
            .map_err(|err| internal_error(format!("failed to delete thread context pin: {err}")))?;
        Ok(ThreadContextPinDeleteResponse { deleted })
    }

    async fn state_db_for_materialized_thread(
        &self,
        thread_id: ThreadId,
    ) -> Result<StateDbHandle, JSONRPCErrorError> {
        if let Ok(thread) = self.thread_manager.get_thread(thread_id).await {
            if thread.rollout_path().is_none() {
                return Err(invalid_request(format!(
                    "ephemeral thread does not support context pins: {thread_id}"
                )));
            }
            if let Some(state_db) = thread.state_db() {
                return Ok(state_db);
            }
        } else {
            self.find_context_pin_rollout_path(thread_id).await?;
        }

        self.state_db
            .clone()
            .ok_or_else(|| internal_error("sqlite state db unavailable for thread context pins"))
    }

    async fn reconcile_context_pin_rollout(
        &self,
        thread_id: ThreadId,
        state_db: &StateDbHandle,
    ) -> Result<(), JSONRPCErrorError> {
        let running_thread = self.thread_manager.get_thread(thread_id).await.ok();
        let (rollout_path, archived_only) = match running_thread.as_ref() {
            Some(thread) => {
                let rollout_path = thread.rollout_path().ok_or_else(|| {
                    invalid_request(format!(
                        "ephemeral thread does not support context pins: {thread_id}"
                    ))
                })?;
                thread.ensure_rollout_materialized().await;
                thread.flush_rollout().await.map_err(|err| {
                    internal_error(format!("failed to flush thread {thread_id}: {err}"))
                })?;
                (rollout_path, None)
            }
            None => {
                let (rollout_path, archived) =
                    self.find_context_pin_rollout_path(thread_id).await?;
                (rollout_path, Some(archived))
            }
        };
        reconcile_rollout(
            Some(state_db),
            rollout_path.as_path(),
            self.config.model_provider_id.as_str(),
            /*builder*/ None,
            &[],
            archived_only,
            /*new_thread_memory_mode*/ None,
        )
        .await;
        Ok(())
    }

    async fn find_context_pin_rollout_path(
        &self,
        thread_id: ThreadId,
    ) -> Result<(PathBuf, bool), JSONRPCErrorError> {
        let thread_id_str = thread_id.to_string();
        let active_path = codex_rollout::find_thread_path_by_id_str(
            &self.config.codex_home,
            &thread_id_str,
            self.state_db.as_deref(),
        )
        .await
        .map_err(|err| internal_error(format!("failed to locate thread id {thread_id}: {err}")))?;

        if let Some(path) = active_path {
            return Ok((path, false));
        }

        codex_rollout::find_archived_thread_path_by_id_str(
            &self.config.codex_home,
            &thread_id_str,
            self.state_db.as_deref(),
        )
        .await
        .map_err(|err| {
            internal_error(format!(
                "failed to locate archived thread id {thread_id}: {err}"
            ))
        })?
        .map(|path| (path, true))
        .ok_or_else(|| invalid_request(format!("thread not found: {thread_id}")))
    }
}

pub(super) async fn pinned_context_additional_context(
    state_db: Option<&StateDbHandle>,
    thread_id: ThreadId,
) -> Result<BTreeMap<String, CoreAdditionalContextEntry>, JSONRPCErrorError> {
    let Some(state_db) = state_db else {
        return Ok(BTreeMap::new());
    };
    let pins = state_db
        .thread_context_pins()
        .list_thread_context_pins(thread_id)
        .await
        .map_err(|err| internal_error(format!("failed to list thread context pins: {err}")))?;

    Ok(pins
        .into_iter()
        .map(|pin| {
            (
                format!("{CONTEXT_PIN_ADDITIONAL_CONTEXT_KEY_PREFIX}{}", pin.pin_id),
                CoreAdditionalContextEntry {
                    value: pin.text,
                    kind: CoreAdditionalContextKind::Untrusted,
                },
            )
        })
        .collect())
}

fn validate_context_pin_id(pin_id: &str) -> Result<(), JSONRPCErrorError> {
    if pin_id.trim().is_empty() {
        return Err(invalid_request("pinId must not be empty"));
    }
    Ok(())
}

fn validate_context_pin_text(text: &str) -> Result<(), JSONRPCErrorError> {
    if text.trim().is_empty() {
        return Err(invalid_request("context pin text must not be empty"));
    }
    let bytes = text.len();
    if bytes > MAX_CONTEXT_PIN_TEXT_BYTES {
        return Err(invalid_request(format!(
            "context pin text must be at most {MAX_CONTEXT_PIN_TEXT_BYTES} bytes"
        )));
    }
    Ok(())
}

fn validate_create_context_pin_limits(
    existing: &[codex_state::ThreadContextPin],
    new_text: &str,
) -> Result<(), JSONRPCErrorError> {
    if existing.len() >= MAX_CONTEXT_PINS_PER_THREAD {
        return Err(invalid_request(format!(
            "threads can have at most {MAX_CONTEXT_PINS_PER_THREAD} context pins"
        )));
    }
    validate_total_context_pin_bytes(existing.iter().map(|pin| pin.text.as_str()), Some(new_text))
}

fn validate_update_context_pin_limits(
    existing: &[codex_state::ThreadContextPin],
    updated_pin_id: &str,
    updated_text: &str,
) -> Result<(), JSONRPCErrorError> {
    validate_total_context_pin_bytes(
        existing
            .iter()
            .filter(|pin| pin.pin_id != updated_pin_id)
            .map(|pin| pin.text.as_str()),
        Some(updated_text),
    )
}

fn validate_total_context_pin_bytes<'a>(
    existing_texts: impl Iterator<Item = &'a str>,
    new_text: Option<&str>,
) -> Result<(), JSONRPCErrorError> {
    let existing_bytes: usize = existing_texts.map(str::len).sum();
    let new_bytes = new_text.map(str::len).unwrap_or(0);
    let total_bytes = existing_bytes.saturating_add(new_bytes);
    if total_bytes > MAX_CONTEXT_PIN_TOTAL_TEXT_BYTES {
        return Err(invalid_request(format!(
            "thread context pins must total at most {MAX_CONTEXT_PIN_TOTAL_TEXT_BYTES} bytes"
        )));
    }
    Ok(())
}

fn paginate_context_pins(
    pins: Vec<ThreadContextPin>,
    cursor: Option<&str>,
    limit: Option<u32>,
) -> Result<(Vec<ThreadContextPin>, Option<String>), JSONRPCErrorError> {
    let total = pins.len();
    let start = match cursor {
        Some(cursor) => cursor
            .parse::<usize>()
            .map_err(|_| invalid_request(format!("invalid cursor: {cursor}")))?,
        None => 0,
    };

    if start > total {
        return Err(invalid_request(format!(
            "cursor {start} exceeds total context pins {total}"
        )));
    }

    let requested_limit = limit.map(|value| value as usize);
    if requested_limit == Some(0) {
        return Err(invalid_request("limit must be greater than 0"));
    }
    let effective_limit = requested_limit
        .unwrap_or(DEFAULT_CONTEXT_PIN_LIST_LIMIT)
        .min(MAX_CONTEXT_PINS_PER_THREAD);
    let end = start.saturating_add(effective_limit).min(total);
    let next_cursor = (end < total).then(|| end.to_string());
    Ok((pins[start..end].to_vec(), next_cursor))
}

fn parse_thread_id_for_context_pin_request(thread_id: &str) -> Result<ThreadId, JSONRPCErrorError> {
    ThreadId::from_string(thread_id)
        .map_err(|err| invalid_request(format!("invalid thread id: {err}")))
}

fn api_thread_context_pin_from_state(pin: codex_state::ThreadContextPin) -> ThreadContextPin {
    ThreadContextPin {
        thread_id: pin.thread_id.to_string(),
        pin_id: pin.pin_id,
        text: pin.text,
        created_at: pin.created_at.timestamp(),
        updated_at: pin.updated_at.timestamp(),
    }
}
