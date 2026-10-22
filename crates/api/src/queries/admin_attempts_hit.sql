INSERT INTO admin_attempts (key, window, hits)
VALUES (?1, ?2, 1)
ON CONFLICT (key, window) DO UPDATE SET hits = hits + 1
RETURNING hits
