UPDATE journal_pinned_repos
SET repo_name = ?2, topic = ?3
WHERE id = ?1
RETURNING repo_name
