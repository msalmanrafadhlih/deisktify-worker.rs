DELETE FROM message_reports
WHERE message_id = ?1
RETURNING message_id
