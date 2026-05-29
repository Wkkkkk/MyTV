INSERT INTO channels (id, name, category, logo_url, type, sort_order, loop_anchor) VALUES
  (1, 'Live OK',       'test', NULL, 'live',     1, NULL),
  (2, 'All Down',      'test', NULL, 'live',     2, NULL),
  (3, 'Has Fallback',  'test', NULL, 'live',     3, NULL),
  (4, 'VOD Has Items', 'test', NULL, 'vod_loop', 4, '2020-01-01 00:00:00'),
  (5, 'VOD Empty',     'test', NULL, 'vod_loop', 5, '2020-01-01 00:00:00');

INSERT INTO sources (id, channel_id, kind, url, priority, is_active, consecutive_failures) VALUES
  (1, 1, 'hls', 'https://stream.example.com/live.m3u8',    1, 1, 0),
  (2, 2, 'hls', 'https://stream.example.com/down.m3u8',    1, 0, 3),
  (3, 3, 'hls', 'https://stream.example.com/primary.m3u8', 1, 0, 0),
  (4, 3, 'hls', 'https://stream.example.com/backup.m3u8',  2, 1, 0);

INSERT INTO playlist_items (channel_id, title, url, duration_secs, sort_order) VALUES
  (4, 'Episode 1', 'https://vod.example.com/ep1.mp4', 3600, 1),
  (4, 'Episode 2', 'https://vod.example.com/ep2.mp4', 3600, 2);
