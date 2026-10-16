SELECT id, label, slogan, stream_url, official_link, image_url
FROM radio_stations
WHERE is_active = 1
ORDER BY sort_order, id
LIMIT 100
