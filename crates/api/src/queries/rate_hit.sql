INSERT INTO chat_rate_limits (scope, key, window, hits)
VALUES (?1, ?2, ?3, 1)
ON CONFLICT (scope, key, window) DO UPDATE SET hits = hits + 1
RETURNING hits
