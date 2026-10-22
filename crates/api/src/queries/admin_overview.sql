SELECT
    (SELECT count(*) FROM generations)           AS generations,
    (SELECT count(*) FROM bookmarks)             AS bookmarks,
    (SELECT count(*) FROM radio_stations)        AS radio_stations,
    (SELECT count(*) FROM embiences)             AS embiences,
    (SELECT count(*) FROM embience_presets)      AS embience_presets,
    (SELECT count(*) FROM embience_preset_items) AS embience_preset_items,
    (SELECT count(*) FROM chat_messages)         AS chat_messages,
    (SELECT count(*) FROM message_reports)       AS message_reports,
    (SELECT count(*) FROM journal_snapshots)     AS journal_snapshots,
    (SELECT count(*) FROM journal_pinned_repos)  AS journal_pinned_repos,
    (SELECT count(*) FROM journal_stack)         AS journal_stack,
    (SELECT count(*) FROM admin_audit)           AS admin_audit,
    (SELECT count(*) FROM chat_messages WHERE deleted_at IS NOT NULL) AS messages_hidden,
    (SELECT count(DISTINCT r.message_id)
       FROM message_reports r
       JOIN chat_messages m ON m.id = r.message_id
      WHERE m.deleted_at IS NULL)                AS messages_reported,
    (SELECT fetched_at FROM journal_snapshots ORDER BY fetched_at DESC, id DESC LIMIT 1) AS journal_fetched_at,
    (SELECT source FROM journal_snapshots ORDER BY fetched_at DESC, id DESC LIMIT 1)     AS journal_source,
    (SELECT count(*) FROM _migrations)           AS migrations_applied,
    (SELECT max(name) FROM _migrations)          AS migrations_latest
