UPDATE radio_stations
SET is_active = 0
WHERE id = ?1 AND stream_url = ?2 AND is_active = 1
