SELECT id, username, url, message, created_at
FROM (
    SELECT id, username, url, message, created_at
    FROM chat_messages
    WHERE deleted_at IS NULL
    ORDER BY id DESC
    LIMIT ?1
)
ORDER BY id ASC
