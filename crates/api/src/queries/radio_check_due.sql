SELECT s.id AS id,
       s.stream_url AS stream_url,
       c.checked_url AS checked_url,
       c.consecutive_failures AS failures,
       c.failing_since AS failing_since,
       c.last_ok_at AS last_ok_at,
       c.disabled_at AS disabled_at
FROM radio_stations AS s
LEFT JOIN radio_checks AS c ON c.station_id = s.id
WHERE s.is_active = 1
ORDER BY (c.last_checked_at IS NOT NULL), c.last_checked_at, s.id
LIMIT ?1
