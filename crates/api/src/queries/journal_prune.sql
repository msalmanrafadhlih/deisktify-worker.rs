DELETE FROM journal_snapshots
WHERE source = 'github'
  AND id NOT IN (
      SELECT id
      FROM journal_snapshots
      WHERE source = 'github'
      ORDER BY fetched_at DESC, id DESC
      LIMIT ?1
  )
