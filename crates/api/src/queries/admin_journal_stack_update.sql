UPDATE journal_stack
SET name = ?2
WHERE id = ?1
RETURNING name
