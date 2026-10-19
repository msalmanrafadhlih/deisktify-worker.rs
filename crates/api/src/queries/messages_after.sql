SELECT id, username, url, message, created_at
FROM chat_messages
WHERE id > ?1 AND deleted_at IS NULL
ORDER BY id ASC
LIMIT ?2
