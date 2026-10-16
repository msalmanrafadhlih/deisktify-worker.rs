-- Seed konten situs: generations, bookmarks, radio, embience.
-- DIBANGKITKAN oleh scripts/gen_seed.mjs dari berkas JSON frontend. Jangan diedit tangan.
-- Aman dijalankan berulang kali (INSERT OR IGNORE dengan id eksplisit).

-- generations
INSERT OR IGNORE INTO generations (number, label, link, kernel, built_on) VALUES
    (1, 'GitHub Journal', '/profile', '6.9.12', '2026-01-10'),
    (2, 'Github', 'https://github.com/msalmanrafadhlih', '6.10.4', '2026-03-21'),
    (3, 'LinkedIn', 'https://linkedin.com/in/msalmanrafadhlih', '6.11.9', '2026-05-02'),
    (4, 'My Portofolio', 'https://msalmanrafadhlih.is-a.dev', '6.12.5', '2026-07-14'),
    (5, 'Go to Desktop', '/deisktify', '6.12.7', '2026-08-30');

-- bookmarks
INSERT OR IGNORE INTO bookmarks (id, label, url, sort_order) VALUES
    (1, 'Github', 'https://github.com/msalmanrafadhlih', 0),
    (2, 'windah basudara', 'https://www.youtube.com/@WindahBasudara?sub_confirmation=1', 1),
    (3, 'Motion IME', 'https://discord.com/invite/motionime', 2);

-- radio_stations
INSERT OR IGNORE INTO radio_stations (id, label, slogan, stream_url, official_link, image_url, sort_order, is_active) VALUES
    (1, 'Delta FM', 'Hits Terenak', 'https://n05-us.rcs.revma.com/k02rmq48kxcwv?rj-ttl=5&rj-tok=AAABnqUThoUAAqX6VpJNMeaCtg&age=18+', 'https://deltafm.net/', 'https://deltafm.net/img/coverArt.png', 0, 1),
    (2, 'Jak FM', 'All Great Things For Your Day', 'https://wz.mari.co.id:1936/noice_jakfm/jakfm/chunklist_w644295007.m3u8', 'https://jak101fm.com/', 'https://images.noiceid.cc/catalog/content-1614529857498.jpg', 1, 1),
    (3, 'Prambors FM', 'Indonesia No. 1 Hit Music Station', 'https://stream.rcs.revma.com/h77wwp48kxcwv?city=Bogor', 'https://prambors.live/', 'https://www.pramborsfm.com/logo.png', 2, 1),
    (4, 'Hard Rock FM (The Rockin Life)', 'Streaming Radio Online Indonesia', 'https://stream.radiojar.com/7csmg90fuqruv?1780884834=', 'https://therockinlife.com/', 'https://assets-static.therockinlife.com/wp-content/uploads/2024/12/TRL-MUSIC-STREAM-PLAYER-JKT.jpg', 3, 1),
    (5, 'Gen98 FM', 'Gen 987 FM! Radio No. 1 di Jakarta!', 'https://wz.mari.co.id:1936/noice_genfm/genfm/chunklist_w1051351755.m3u8', 'https://gen987fm.com/', 'https://images.noiceid.cc/catalog/content-1614529848190.jpg', 4, 1);

-- embiences
INSERT OR IGNORE INTO embiences (id, name, icon_url, stream_url, sort_order) VALUES
    (1, 'Rain', 'https://raw.githubusercontent.com/msalmanrafadhlih/tquilla.is-a.dev/refs/heads/main/packages/components/assets/logo-rain.svg', 'https://r2.earth.fm/wp-content/uploads/2022/08/512334__felix-blume__light-rain-in-a-field-of-bananas-trees.mp3', 0),
    (2, 'Storm', 'https://raw.githubusercontent.com/msalmanrafadhlih/tquilla.is-a.dev/refs/heads/main/packages/components/assets/logo-storm.svg', 'https://r2.earth.fm/wp-content/uploads/2022/11/Storm-in-conifer-forest.mp3', 1),
    (3, 'Wind', 'https://raw.githubusercontent.com/msalmanrafadhlih/tquilla.is-a.dev/refs/heads/main/packages/components/assets/logo-wind.svg', 'https://r2.earth.fm/wp-content/uploads/2026/03/02-Wind-in-Grass-Mountain-Meadow-Summer-Night.mp3', 2),
    (4, 'Wave', 'https://raw.githubusercontent.com/msalmanrafadhlih/tquilla.is-a.dev/refs/heads/main/packages/components/assets/logo-wave.svg', 'https://r2.earth.fm/wp-content/uploads/2024/03/earth-fm_Jacob-Bentz_Waves-of-Wind.mp3', 3),
    (5, 'Stream', 'https://raw.githubusercontent.com/msalmanrafadhlih/tquilla.is-a.dev/refs/heads/main/packages/components/assets/logo-stream.svg', 'https://r2.earth.fm/wp-content/uploads/2022/07/earth-fm_MelissaPons_Malcata-stream.mp3', 4),
    (6, 'Train', 'https://raw.githubusercontent.com/msalmanrafadhlih/tquilla.is-a.dev/refs/heads/main/packages/components/assets/logo-train.svg', 'https://cdn.pixabay.com/audio/2022/03/12/audio_a7f39b960a.mp3', 5),
    (7, 'Summer Night', 'https://raw.githubusercontent.com/msalmanrafadhlih/tquilla.is-a.dev/refs/heads/main/packages/components/assets/logo-summer_night.svg', 'https://cdn.pixabay.com/audio/2022/02/07/audio_cfb54c4a18.mp3', 6),
    (8, 'Birds', 'https://raw.githubusercontent.com/msalmanrafadhlih/tquilla.is-a.dev/refs/heads/main/packages/components/assets/logo-birds.svg', 'https://cdn.pixabay.com/audio/2022/02/10/audio_7a07ee0e79.mp3', 7),
    (9, 'City', 'https://raw.githubusercontent.com/msalmanrafadhlih/tquilla.is-a.dev/refs/heads/main/packages/components/assets/logo-city.svg', 'https://cdn.pixabay.com/audio/2023/12/11/audio_36ea838e09.mp3', 8),
    (10, 'Boat', 'https://raw.githubusercontent.com/msalmanrafadhlih/tquilla.is-a.dev/refs/heads/main/packages/components/assets/logo-boat.svg', 'https://cdn.pixabay.com/audio/2022/03/09/audio_1e84fd8637.mp3', 9),
    (11, 'Fireplace', 'https://raw.githubusercontent.com/msalmanrafadhlih/tquilla.is-a.dev/refs/heads/main/packages/components/assets/logo-fireplace.svg', 'https://cdn.pixabay.com/audio/2022/09/10/audio_113b247b69.mp3', 10),
    (12, 'Coffee Shop', 'https://raw.githubusercontent.com/msalmanrafadhlih/tquilla.is-a.dev/refs/heads/main/packages/components/assets/logo-coffee_shop.svg', 'https://cdn.pixabay.com/audio/2022/03/13/audio_c62782dbc9.mp3', 11);

-- embience_presets
INSERT OR IGNORE INTO embience_presets (id, label, sort_order) VALUES
    (1, 'Learning', 0),
    (2, 'Chill', 1),
    (3, 'Busy', 2);

-- embience_preset_items
INSERT OR IGNORE INTO embience_preset_items (preset_id, embience_id, position) VALUES
    (1, 6, 0),
    (1, 12, 1),
    (2, 7, 0),
    (2, 11, 1),
    (3, 9, 0),
    (3, 6, 1),
    (3, 12, 2);
