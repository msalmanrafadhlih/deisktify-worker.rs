SELECT id, username, url, message, created_at
FROM (
    SELECT id, username, url, message, created_at
    FROM chat_messages
    WHERE id < ?1 AND deleted_at IS NULL
    ORDER BY id DESC
    LIMIT ?2
)
ORDER BY id ASC
