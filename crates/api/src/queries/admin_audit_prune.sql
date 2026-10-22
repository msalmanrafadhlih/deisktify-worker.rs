DELETE FROM admin_audit
WHERE id <= (SELECT max(id) FROM admin_audit) - ?1
