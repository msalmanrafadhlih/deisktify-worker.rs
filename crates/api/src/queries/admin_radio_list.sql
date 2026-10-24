SELECT s.id AS id,
       s.label AS label,
       s.stream_url AS stream_url,
       s.is_active AS is_active,
       c.checked_url AS checked_url,
       c.last_checked_at AS last_checked_at,
       c.last_ok_at AS last_ok_at,
       c.failing_since AS failing_since,
       c.consecutive_failures AS failures,
       c.last_result AS last_result,
       c.last_status AS last_status,
       c.disabled_at AS disabled_at
FROM radio_stations AS s
LEFT JOIN radio_checks AS c ON c.station_id = s.id
ORDER BY s.sort_order, s.id
LIMIT 200
