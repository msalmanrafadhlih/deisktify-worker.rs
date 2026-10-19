SELECT payload, source, fetched_at
FROM journal_snapshots
ORDER BY fetched_at DESC, id DESC
LIMIT 1
