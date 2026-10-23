SELECT m.id AS id, m.username AS username, m.url AS url, m.message AS message,
       m.created_at AS created_at, m.deleted_at AS deleted_at,
       (SELECT count(*) FROM message_reports r WHERE r.message_id = m.id) AS reports,
       (SELECT max(r.created_at) FROM message_reports r WHERE r.message_id = m.id) AS last_reported_at
FROM chat_messages m
WHERE (?1 IS NULL OR m.id < ?1)
  AND CASE ?2
        WHEN 'hidden' THEN m.deleted_at IS NOT NULL
        WHEN 'reported' THEN m.deleted_at IS NULL
                             AND EXISTS (SELECT 1 FROM message_reports r WHERE r.message_id = m.id)
        ELSE 1
      END
ORDER BY m.id DESC
LIMIT ?3
