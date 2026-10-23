DELETE FROM journal_pinned_repos
WHERE id = ?1
RETURNING repo_name
