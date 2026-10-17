SELECT id, username, url, avatar_url, message, attachment_url, created_at
FROM (
    SELECT id, username, url, avatar_url, message, attachment_url, created_at
    FROM chat_messages
    ORDER BY id DESC
    LIMIT ?1
)
ORDER BY id ASC
