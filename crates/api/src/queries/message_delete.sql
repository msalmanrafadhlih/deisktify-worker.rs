UPDATE chat_messages
SET deleted_at = ?2
WHERE id = ?1 AND deleted_at IS NULL
RETURNING id
