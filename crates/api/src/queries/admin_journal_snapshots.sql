SELECT id, fetched_at, source
FROM journal_snapshots
ORDER BY fetched_at DESC, id DESC
LIMIT ?1
