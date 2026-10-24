DELETE FROM radio_checks
WHERE station_id NOT IN (SELECT id FROM radio_stations)
