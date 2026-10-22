INSERT INTO admin_audit (at, action, target, who)
VALUES (?1, ?2, ?3, ?4)
RETURNING id
