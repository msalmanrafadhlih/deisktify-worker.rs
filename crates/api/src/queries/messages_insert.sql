INSERT INTO chat_messages (username, url, message, created_at)
VALUES (?1, ?2, ?3, ?4)
RETURNING id, username, url, avatar_url, message, attachment_url, created_at
