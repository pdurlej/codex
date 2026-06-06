CREATE TABLE thread_context_pins (
    thread_id TEXT NOT NULL REFERENCES threads(id) ON DELETE CASCADE,
    pin_id TEXT NOT NULL,
    text TEXT NOT NULL,
    created_at_ms INTEGER NOT NULL,
    updated_at_ms INTEGER NOT NULL,
    PRIMARY KEY(thread_id, pin_id)
);

CREATE INDEX idx_thread_context_pins_thread_created
ON thread_context_pins(thread_id, created_at_ms ASC, pin_id);
