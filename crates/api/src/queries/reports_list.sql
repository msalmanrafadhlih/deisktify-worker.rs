SELECT m.id AS id, m.username AS username, m.url AS url, m.message AS message,
       m.created_at AS created_at,
       count(*) AS reports, max(r.created_at) AS last_reported_at
FROM message_reports r
JOIN chat_messages m ON m.id = r.message_id
WHERE m.deleted_at IS NULL
GROUP BY m.id
ORDER BY reports DESC, last_reported_at DESC, m.id DESC
LIMIT ?1
