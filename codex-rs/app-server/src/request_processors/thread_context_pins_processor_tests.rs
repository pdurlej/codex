use super::*;
use crate::error_code::INVALID_REQUEST_ERROR_CODE;
use chrono::DateTime;
use chrono::Utc;
use codex_protocol::ThreadId;
use pretty_assertions::assert_eq;

fn make_pin(pin_id: &str, text: &str) -> codex_state::ThreadContextPin {
    let now = DateTime::<Utc>::from_timestamp(0, 0).expect("epoch is valid");
    codex_state::ThreadContextPin {
        thread_id: ThreadId::from_string("00000000-0000-0000-0000-000000000001")
            .expect("valid thread id"),
        pin_id: pin_id.to_string(),
        text: text.to_string(),
        created_at: now,
        updated_at: now,
    }
}

#[test]
fn validate_context_pin_id_rejects_empty_and_whitespace() {
    assert_eq!(
        validate_context_pin_id("").unwrap_err().code,
        INVALID_REQUEST_ERROR_CODE
    );
    assert_eq!(
        validate_context_pin_id("   \t").unwrap_err().code,
        INVALID_REQUEST_ERROR_CODE
    );
    validate_context_pin_id("pin-1").expect("non-empty id is valid");
}

#[test]
fn validate_context_pin_text_rejects_empty_and_whitespace() {
    assert_eq!(
        validate_context_pin_text("").unwrap_err().code,
        INVALID_REQUEST_ERROR_CODE
    );
    assert_eq!(
        validate_context_pin_text("  \n\t ").unwrap_err().code,
        INVALID_REQUEST_ERROR_CODE
    );
}

#[test]
fn validate_context_pin_text_enforces_byte_limit_boundary() {
    validate_context_pin_text(&"x".repeat(MAX_CONTEXT_PIN_TEXT_BYTES))
        .expect("exactly at the byte limit is valid");
    assert_eq!(
        validate_context_pin_text(&"x".repeat(MAX_CONTEXT_PIN_TEXT_BYTES + 1))
            .unwrap_err()
            .code,
        INVALID_REQUEST_ERROR_CODE
    );
}

#[test]
fn validate_create_pin_limits_enforce_count_boundary() {
    let fifteen: Vec<_> = (0..(MAX_CONTEXT_PINS_PER_THREAD - 1))
        .map(|index| make_pin(&format!("pin-{index}"), "a"))
        .collect();
    validate_create_context_pin_limits(&fifteen, "new")
        .expect("under the per-thread count limit is valid");

    let sixteen: Vec<_> = (0..MAX_CONTEXT_PINS_PER_THREAD)
        .map(|index| make_pin(&format!("pin-{index}"), "a"))
        .collect();
    assert_eq!(
        validate_create_context_pin_limits(&sixteen, "new")
            .unwrap_err()
            .code,
        INVALID_REQUEST_ERROR_CODE
    );
}

#[test]
fn validate_create_pin_limits_enforce_total_byte_boundary() {
    // Each pin stays under the per-pin limit, but together they approach the
    // thread total so the boundary is exercised through the combined budget.
    let existing: Vec<_> = (0..3)
        .map(|index| {
            make_pin(
                &format!("pin-{index}"),
                &"x".repeat(MAX_CONTEXT_PIN_TEXT_BYTES),
            )
        })
        .collect();
    // 3 * 16384 = 49152; remaining budget is exactly one per-pin-sized pin.
    let remaining = MAX_CONTEXT_PIN_TOTAL_TEXT_BYTES - 3 * MAX_CONTEXT_PIN_TEXT_BYTES;
    validate_create_context_pin_limits(&existing, &"x".repeat(remaining))
        .expect("total at limit is valid");

    assert_eq!(
        validate_create_context_pin_limits(&existing, &"x".repeat(remaining + 1))
            .unwrap_err()
            .code,
        INVALID_REQUEST_ERROR_CODE
    );
}

#[test]
fn validate_update_pin_limits_exclude_the_updated_pin_from_total() {
    let existing = vec![make_pin("pin-1", "first"), make_pin("pin-2", "second")];
    // Replacing pin-1 with a large value keeps the total bounded by excluding pin-1.
    let replacement = &"x".repeat(MAX_CONTEXT_PIN_TOTAL_TEXT_BYTES - "second".len());
    validate_update_context_pin_limits(&existing, "pin-1", replacement)
        .expect("update excluding the replaced pin keeps total valid");

    // A brand-new id is treated like an add, so the existing pin-1 text still counts.
    let too_large = &"x".repeat(MAX_CONTEXT_PIN_TOTAL_TEXT_BYTES - "second".len() + 1);
    assert_eq!(
        validate_update_context_pin_limits(&existing, "missing-pin", too_large)
            .unwrap_err()
            .code,
        INVALID_REQUEST_ERROR_CODE
    );
}

#[test]
fn paginate_context_pins_rejects_zero_limit_and_overflow_cursor() {
    let pins = vec![
        api_thread_context_pin_from_state(make_pin("pin-1", "a")),
        api_thread_context_pin_from_state(make_pin("pin-2", "b")),
    ];

    assert_eq!(
        paginate_context_pins(pins.clone(), None, Some(0))
            .unwrap_err()
            .code,
        INVALID_REQUEST_ERROR_CODE
    );
    assert_eq!(
        paginate_context_pins(pins.clone(), Some("3"), None)
            .unwrap_err()
            .code,
        INVALID_REQUEST_ERROR_CODE
    );
    assert_eq!(
        paginate_context_pins(pins, Some("not-a-number"), None)
            .unwrap_err()
            .code,
        INVALID_REQUEST_ERROR_CODE
    );
}

#[test]
fn paginate_context_pins_returns_pages_and_next_cursor() {
    let pins: Vec<_> = (0..3)
        .map(|index| api_thread_context_pin_from_state(make_pin(&format!("pin-{index}"), "a")))
        .collect();

    let (page, next_cursor) =
        paginate_context_pins(pins.clone(), None, Some(2)).expect("first page is valid");
    assert_eq!(
        page.iter()
            .map(|pin| pin.pin_id.as_str())
            .collect::<Vec<_>>(),
        vec!["pin-0", "pin-1"],
    );
    assert_eq!(next_cursor.as_deref(), Some("2"));

    let (page, next_cursor) =
        paginate_context_pins(pins, Some("2"), Some(2)).expect("second page is valid");
    assert_eq!(
        page.iter()
            .map(|pin| pin.pin_id.as_str())
            .collect::<Vec<_>>(),
        vec!["pin-2"],
    );
    assert_eq!(next_cursor, None);
}
