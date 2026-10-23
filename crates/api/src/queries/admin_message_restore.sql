UPDATE chat_messages
SET deleted_at = NULL
WHERE id = ?1 AND deleted_at IS NOT NULL
RETURNING id
