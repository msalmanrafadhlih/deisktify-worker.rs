INSERT OR IGNORE INTO message_reports (message_id, reporter, created_at)
SELECT id, ?2, ?3
FROM chat_messages
WHERE id = ?1 AND deleted_at IS NULL
RETURNING message_id
