SELECT count(*) AS n
FROM message_reports
WHERE reporter = ?1 AND created_at >= ?2
