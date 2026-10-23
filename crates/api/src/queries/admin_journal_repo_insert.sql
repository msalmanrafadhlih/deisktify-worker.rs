INSERT INTO journal_pinned_repos (position, repo_name, topic)
SELECT p, ?1, ?2
FROM (SELECT coalesce(max(position), 0) + 1 AS p FROM journal_pinned_repos)
WHERE (SELECT count(*) FROM journal_pinned_repos) < ?3
RETURNING id
