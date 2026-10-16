SELECT p.id AS preset_id, p.label AS label, e.id AS embience_id
FROM embience_presets p
LEFT JOIN embience_preset_items i ON i.preset_id = p.id
LEFT JOIN embiences e ON e.id = i.embience_id
ORDER BY p.sort_order, p.id, i.position, e.id
LIMIT 1000
