DELETE FROM message_reports
WHERE message_id = ?1
  AND NOT EXISTS (SELECT 1 FROM chat_messages WHERE id = ?1)
