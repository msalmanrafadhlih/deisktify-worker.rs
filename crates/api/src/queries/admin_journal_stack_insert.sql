INSERT INTO journal_stack (position, name)
SELECT p, ?1
FROM (SELECT coalesce(max(position), 0) + 1 AS p FROM journal_stack)
WHERE (SELECT count(*) FROM journal_stack) < ?2
RETURNING id
