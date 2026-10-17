SELECT id, username, url, avatar_url, message, attachment_url, created_at
FROM chat_messages
WHERE id > ?1
ORDER BY id ASC
LIMIT ?2
