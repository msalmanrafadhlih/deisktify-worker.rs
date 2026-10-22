SELECT id, at, action, target, who
FROM admin_audit
WHERE (?1 IS NULL OR id < ?1)
ORDER BY id DESC
LIMIT ?2
