DELETE FROM chat_messages
WHERE id = ?1 AND deleted_at IS NOT NULL
RETURNING id
