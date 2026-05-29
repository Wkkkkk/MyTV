# Configuration

## Environment variables

| Variable | Default | Required | Notes |
|---|---|---|---|
| `DATABASE_URL` | `sqlite:mytv.db` | No | Path to the SQLite file. Use an absolute path in production. |
| `ADMIN_PASSWORD` | `admin` | **Yes** | Protects `/admin` via HTTP Basic Auth. Change this. |
| `PORT` | `3000` | No | TCP port the server listens on. |
| `YOUTUBE_API_KEY` | _(unset)_ | No | YouTube Data API v3 key. Without it, the YouTube tab in Discover shows a configuration message. |
| `RUST_LOG` | _(unset)_ | No | Log level filter, e.g. `info`, `mytv=debug`. |

---

## Getting a YouTube API key (optional)

YouTube search in the Discover tab requires a YouTube Data API v3 key.

1. Go to [Google Cloud Console](https://console.cloud.google.com)
2. Create or reuse a project
3. Enable **YouTube Data API v3**
4. Create an **API key** credential (no OAuth needed)
5. Set `YOUTUBE_API_KEY=<your-key>` in your environment

The free tier quota (10,000 units/day) is sufficient for personal use — a keyword search costs ~100 units.
