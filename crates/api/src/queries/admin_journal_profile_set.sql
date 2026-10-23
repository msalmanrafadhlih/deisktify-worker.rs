INSERT INTO journal_profile (id, github_username)
VALUES (1, ?1)
ON CONFLICT (id) DO UPDATE SET github_username = excluded.github_username
RETURNING github_username
