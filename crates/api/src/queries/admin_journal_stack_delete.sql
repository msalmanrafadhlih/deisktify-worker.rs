DELETE FROM journal_stack
WHERE id = ?1
RETURNING name
