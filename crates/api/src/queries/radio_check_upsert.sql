INSERT INTO radio_checks (
    station_id, checked_url, last_checked_at, last_ok_at, failing_since,
    consecutive_failures, last_result, last_status, disabled_at
)
VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
ON CONFLICT (station_id) DO UPDATE SET
    checked_url = excluded.checked_url,
    last_checked_at = excluded.last_checked_at,
    last_ok_at = excluded.last_ok_at,
    failing_since = excluded.failing_since,
    consecutive_failures = excluded.consecutive_failures,
    last_result = excluded.last_result,
    last_status = excluded.last_status,
    disabled_at = excluded.disabled_at
