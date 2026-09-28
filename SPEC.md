# moodbeat — Specification

> Music player that builds a custom playlist from a mood you describe.

Status: draft · Scope: v1 (MVP) · Implementation: not started

---

## 1. Overview

moodbeat is a desktop app built with **Rust + Tauri 2**. The user types a mood or vibe
(e.g. *"90s rock"*, *"rainy day in New York"*, *"Polish summertime"*), and the app:

1. Asks an LLM (OpenAI API) for a list of **10–15 songs** that match the mood.
2. Finds each song on **YouTube**.
3. Downloads the audio and converts it into a playable **music file**.
4. Plays the playlist in order with **play/pause, previous, next** controls. When a
   song ends the next one starts automatically; when the last song ends, playback stops.

All app data lives in **`~/.moodbeat/`**. The UI is minimalistic and follows
[`DESIGN.md`](./DESIGN.md).

### Goals

- One screen, one input, one list, one set of controls — nothing else.
- Time from pressing *Generate* to hearing the first song: as short as possible
  (playback starts as soon as the first track is ready, not when all are ready).
- No accounts, no server — the only external services are OpenAI and YouTube.

### Non-goals (v1)

- Streaming without downloading, Spotify/Apple Music integration.
- Editing playlists (reordering, adding, removing songs), shuffle, repeat.
- Browsing or re-playing past playlists (history is stored, but has no UI in v1 — see §12).
- Mobile builds.

---

## 2. Target platforms

| Platform | Priority | Notes |
|----------|----------|-------|
| macOS (Apple Silicon + Intel) | Primary | Developed and tested here first |
| Linux (x86_64) | Secondary | WebKitGTK needs GStreamer plugins for MP3 playback |
| Windows 10/11 | Secondary | WebView2 |

---

## 3. User flows

### 3.1 First launch

1. App creates `~/.moodbeat/` and its subdirectories (§8).
2. App checks for the external tools `yt-dlp` and `ffmpeg` (§6.1). If missing, it
   downloads them into `~/.moodbeat/bin/` and shows progress. If that fails, it shows a
   clear error with manual install instructions.
3. No OpenAI API key is configured → the Settings panel opens automatically with a short
   message: *"Add your OpenAI API key to start."*

### 3.2 Setting the API key

1. User opens Settings (gear icon in the header).
2. User pastes the key into a password-type field (masked, with a show/hide toggle).
3. User clicks **Save**. The app validates the key with a cheap API call
   (`GET /v1/models`). Valid → saved to config, panel closes.
   Invalid → inline error in Negative Red, key is not saved.
4. The key can later be replaced or removed (**Remove key** button).

### 3.3 Generating a playlist

1. User types a mood into the input (max 200 characters) and presses **Enter** or the
   **Generate** button. Empty or whitespace-only input keeps the button disabled.
2. The input and Generate button are disabled; a loading state is shown
   (*"Finding songs for 'rainy day in New York'…"*).
3. The backend calls OpenAI (§5) and gets 10–15 songs.
4. The list shows all songs right away (artist + title), each with a status of *pending*.
5. The backend resolves and downloads tracks in the background (§6). Each row updates its
   status live: `pending → searching → downloading (n%) → ready` or `failed`.
6. **As soon as track #1 is ready, it starts playing automatically.**
7. If the user starts a new generation while a playlist is playing, the current playback
   stops, pending downloads of the old playlist are cancelled, and the new playlist
   replaces the old one.

### 3.4 Playback

| Action | Behavior |
|--------|----------|
| **Play/Pause** | Toggles playback of the current track. If playback has finished (after the last song), Play starts again from track #1. |
| **Next** | Jumps to the next *playable* track. On the last track: stops playback (same as the end of the playlist). |
| **Previous** | If the current position is > 3 s → restart the current track. Otherwise → go to the previous *playable* track. On the first track: restart it. |
| **Click on a row** | Plays that track (if it is ready). |
| **Track ends** | The next track starts automatically. |
| **Last track ends** | Playback stops, the player shows the *finished* state, the current track pointer resets to #1 (not playing). |

"Playable" = status `ready`. Rules for tracks that are not ready:

- Next track still `downloading`/`searching` when the current one ends → show a
  *buffering* state in the player bar and start that track as soon as it is `ready`.
- Next track `failed` → skip it silently (the row stays visible with a failed marker).
- All remaining tracks failed → behave as the end of the playlist.

The player bar shows: current track title + artist, elapsed / total time and a
progress bar. The progress bar can be clicked/dragged to seek (nice to have, but cheap
with an HTML `<audio>` element — include it in v1).

### 3.5 Keyboard shortcuts

| Key | Action |
|-----|--------|
| `Enter` (in mood input) | Generate |
| `Space` (focus outside input) | Play/Pause |
| `→` / `←` (focus outside input) | Next / Previous |
| `↑` / `↓` (focus outside input) | Volume up / down (5%) |
| `M` (focus outside input) | Mute / unmute |
| Media keys (Play/Pause, Next, Prev) | Same as buttons — nice to have, via OS media session if available |

---

## 4. Architecture

```
┌──────────────────────── Tauri window ────────────────────────┐
│  Frontend (TypeScript + Vite, no framework)                  │
│  - Mood input, track list, player bar, settings panel        │
│  - <audio> element = the player                              │
│        │ invoke(commands)            ▲ events                │
└────────┼─────────────────────────────┼───────────────────────┘
         ▼                             │
┌──────────────────────── Rust backend ────────────────────────┐
│  config      – read/write ~/.moodbeat/config.json            │
│  llm         – OpenAI client, prompt, JSON parsing           │
│  resolver    – YouTube search via yt-dlp                     │
│  downloader  – yt-dlp + ffmpeg → MP3, progress, cancellation │
│  cache       – audio cache index, lookup by video id         │
│  playlist    – playlist model + persistence                  │
│  tools       – locate/install/update yt-dlp & ffmpeg         │
└──────────────────────────────────────────────────────────────┘
         │                      │
         ▼                      ▼
   api.openai.com         youtube.com (via yt-dlp)
```

### 4.1 Responsibilities split

- **Rust owns** all network I/O, process spawning, file system access and secrets.
  The API key never reaches the frontend (the frontend only learns whether a key is set).
- **Frontend owns** rendering and playback. Audio is played by an HTML5 `<audio>`
  element pointing at the local MP3 via Tauri's asset protocol (`convertFileSrc`).
  The `ended` event drives auto-advance.

  *Why not play audio in Rust (e.g. `rodio`)?* The `<audio>` element gives play/pause,
  seeking, duration, time updates and `ended` for free, and keeps the player state in one
  place (the UI). Rust playback can be revisited if WebView codec support becomes a problem.

### 4.2 Tech stack

| Area | Choice |
|------|--------|
| App shell | Tauri 2.x |
| Backend | Rust (stable, edition 2021), `tokio` async runtime |
| HTTP | `reqwest` (rustls) |
| JSON | `serde`, `serde_json` |
| Paths | `dirs` (home directory) |
| Errors | `thiserror` (domain errors), `anyhow` at the edges |
| Logging | `tracing` + `tracing-appender` → `~/.moodbeat/logs/` |
| Frontend | TypeScript + Vite, plain DOM, plain CSS with custom properties |
| YouTube search/download | `yt-dlp` (external binary) |
| Audio conversion | `ffmpeg` (external binary, used by yt-dlp) |

### 4.3 Tauri commands (frontend → backend)

| Command | Input | Output | Notes |
|---------|-------|--------|-------|
| `get_settings` | – | `{ hasApiKey, model, dataDir }` | Never returns the key |
| `save_api_key` | `{ apiKey }` | `Ok` / error | Validates first, then saves |
| `remove_api_key` | – | `Ok` | |
| `set_model` | `{ model }` | `Ok` | |
| `set_volume` | `{ volume, muted }` | `Ok` | Volume 0–1, persisted in config |
| `generate_playlist` | `{ mood }` | `Playlist` | Returns after the LLM step; downloads continue in the background |
| `cancel_playlist` | `{ playlistId }` | `Ok` | Cancels pending/running downloads |
| `retry_track` | `{ playlistId, trackId }` | `Ok` | Re-runs resolve + download for a failed track |
| `get_tools_status` | – | `{ ytDlp, ffmpeg }` | Version or "missing" |
| `install_tools` | – | `Ok` / error | Emits `tools://progress` |
| `clear_cache` | – | `{ freedBytes }` | Deletes `~/.moodbeat/cache/audio/*` |

### 4.4 Events (backend → frontend)

| Event | Payload |
|-------|---------|
| `track://status` | `{ playlistId, trackId, status, progress?, error? }` |
| `track://ready` | `{ playlistId, trackId, filePath, durationSec }` |
| `tools://progress` | `{ tool, progress, message }` |

Events carry `playlistId` so the frontend can ignore events from a replaced playlist.

---

## 5. Playlist generation (OpenAI)

### 5.1 API usage

- Endpoint: OpenAI **Responses API** (`POST /v1/responses`) with **Structured Outputs**
  (JSON schema, `strict: true`) so the response is always valid, parseable JSON.
- Model: configurable in Settings. Default: a small, cheap, current model
  (e.g. `gpt-4.1-mini`). The default is a constant in code, and the user picks from a
  dropdown of models (a custom id set in `config.json` is kept as an extra option).
- Temperature: moderate (≈ 0.8) — the same mood should give some variety between runs.
- Timeout: 60 s. Retries: 1 retry on network errors / HTTP 5xx / 429 (with backoff).
  No retry on 401/403 (bad key) — show the error.

### 5.2 Prompt

System prompt (draft):

> You are a music curator. Given a mood, vibe, place, time or genre description, return a
> playlist of 10 to 15 real, existing, well-known enough songs that can be found on
> YouTube. Mix artists — no more than 2 songs by the same artist. Order the songs so the
> playlist flows well. Only return songs you are confident exist; never invent songs.
> Use the original artist and the canonical song title.

User message: the mood text as typed (trimmed, max 200 chars). The mood is data, not
instructions — the system prompt says to treat it only as a description of a vibe.

### 5.3 Response schema

```json
{
  "type": "object",
  "properties": {
    "title": { "type": "string", "description": "Short playlist name, max 40 chars" },
    "songs": {
      "type": "array",
      "minItems": 10,
      "maxItems": 15,
      "items": {
        "type": "object",
        "properties": {
          "artist": { "type": "string" },
          "title":  { "type": "string" },
          "year":   { "type": ["integer", "null"] }
        },
        "required": ["artist", "title", "year"],
        "additionalProperties": false
      }
    }
  },
  "required": ["title", "songs"],
  "additionalProperties": false
}
```

### 5.4 Post-processing (Rust)

- Trim strings, drop entries with empty artist or title.
- Drop duplicates (case-insensitive `artist + title`).
- If more than 15 remain → keep the first 15. If fewer than 10 remain but at least 5 →
  accept. Fewer than 5 → treat as an error (*"Couldn't build a playlist for this mood. Try
  describing it differently."*).

---

## 6. YouTube search, download & conversion

### 6.1 External tools

Downloading from YouTube reliably needs `yt-dlp` (it tracks YouTube changes and is
updated often) and `ffmpeg` (audio extraction/conversion). Pure-Rust YouTube crates
break too often to rely on.

Lookup order for each tool:

1. `~/.moodbeat/bin/<tool>` (managed by the app)
2. System `PATH`

If neither exists, the app offers to install the tool into `~/.moodbeat/bin/`:

- `yt-dlp`: official standalone release binary for the platform from GitHub releases
  (`yt-dlp_macos`, `yt-dlp_linux`, `yt-dlp.exe`), marked executable.
- `ffmpeg`: a static build for the platform from a pinned, trusted source (URL + SHA-256
  checksum pinned in code).

On app start (at most once every 24 h), the app runs `yt-dlp -U` for the managed copy in
the background, because stale yt-dlp versions are the most common cause of download failures.

### 6.2 Resolving a song to a video

For each song, run:

```
yt-dlp "ytsearch5:<artist> - <title>" --dump-json --flat-playlist --no-warnings
```

Choose the best of the (up to) 5 results with a simple scoring function:

- `+` title contains the song title and the artist name (normalized: lowercase, no
  punctuation, no diacritics)
- `+` channel is `<Artist> - Topic`, `<Artist>VEVO` or the artist's name
- `+` "official audio" / "official video" in title
- `−` "live", "cover", "karaoke", "remix", "reaction", "8d", "slowed", "sped up",
  "nightcore", "instrumental" in the title when not in the requested song title
- `−` duration < 60 s or > 15 min (drop), duration > 8 min (penalty)

Pick the highest score. No result → track `failed` with *"Not found on YouTube"*.

### 6.3 Downloading and converting

```
yt-dlp -f bestaudio -x --audio-format mp3 --audio-quality 0 \
       --ffmpeg-location <ffmpeg> --no-playlist --newline \
       --embed-metadata \
       -o "~/.moodbeat/cache/audio/<videoId>.%(ext)s" \
       "https://www.youtube.com/watch?v=<videoId>"
```

- Output format: **MP3** (VBR, highest quality). MP3 is chosen because it plays in every
  target WebView (WKWebView, WebView2, WebKitGTK+GStreamer); AAC/Opus support is not
  consistent across them.
- Download to a temporary name first (`<videoId>.part...`, yt-dlp does this), and consider
  the track ready only when the final `.mp3` exists and yt-dlp exited with code 0.
- Progress: parse yt-dlp `--newline` progress lines (`[download]  42.3% ...`) and emit
  `track://status` events (throttled to max ~4 per second per track).
- Timeout: 5 minutes per track. On timeout → kill the process, delete partial files,
  mark `failed`.

### 6.4 Scheduling

- Tracks are processed **in playlist order**, with **max 3 concurrent** jobs, so the
  first tracks are ready first.
- If the user clicks a track that is still `pending`, that track moves to the front of the queue.
- Every job holds a cancellation token. `cancel_playlist` (or a new generation) kills
  running `yt-dlp` processes and deletes their partial files.
- One automatic retry per track on download failure (not on "not found").

### 6.5 Cache

- Audio files are stored by YouTube video id: `cache/audio/<videoId>.mp3`. If a song
  resolves to a video id that is already cached, no download happens — the track is
  `ready` right away.
- `cache/index.json` maps a normalized `artist|title` key → `videoId`, so repeated songs
  skip the search step too.
- Cache limit: **2 GB** by default. When exceeded, the least recently played files
  are deleted (never files of the current playlist).
- Settings has a **Clear cache** button that shows the current cache size.

---

## 7. UI specification

Follows `DESIGN.md` (dark, Spotify-inspired). Only the parts relevant to this small app
are used: no sidebar, no grids, no album art.

### 7.1 Window

- Default size **420 × 720**, min size **360 × 560**, resizable.
- Title: `moodbeat`. Native title bar.
- Background: Near Black `#121212`.

### 7.2 Layout

```
┌──────────────────────────────────────┐
│ moodbeat                         ⚙   │  header
├──────────────────────────────────────┤
│ ╭──────────────────────────────╮     │
│ │ rainy day in New York        │ (→) │  mood input (pill) + Generate
│ ╰──────────────────────────────╯     │
│                                      │
│ Rainy Manhattan Afternoon            │  playlist title (24px / 700)
│ 12 songs                             │  caption, #b3b3b3
│                                      │
│  1  Fast Car                    ✓    │
│     Tracy Chapman                    │
│  2  Englishman in New York     ▶︎    │  ← playing (title in green)
│     Sting                            │
│  3  Rainy Days and Mondays    42%    │  ← downloading
│     Carpenters                       │
│  4  ...                        ⚠     │  ← failed (retry on hover)
│  …                                   │  scrollable list
├──────────────────────────────────────┤
│ Englishman in New York · Sting       │  player bar
│ 1:12 ━━━━━━━━━●──────────────── 4:25  │
│          (⏮)    ( ▶ )    (⏭)          │
└──────────────────────────────────────┘
```

### 7.3 Components

**Header** — `moodbeat` wordmark, 18px / 700, white; gear icon button on the right
(circular, 50% radius, transparent background, `#b3b3b3` icon, white on hover).

**Mood input** — pill (500px radius), background `#1f1f1f`, white text, placeholder
`#b3b3b3` *"What's the vibe?"*, inset border
`rgb(18,18,18) 0px 1px 0px, rgb(124,124,124) 0px 0px 0px 1px inset`, padding
12px 48px 12px 16px. Focus: inset border becomes white.

**Clear button** — "×" icon inside the input, left of Generate (circular, transparent,
`#b3b3b3`, white on hover). Shown when a playlist is on screen or the input has text; hidden
while generating. With only typed text it just clears the input. With a playlist it asks for
confirmation in a modal (*"Clear playlist?"*, **Cancel** focused by default, **Clear** as a
light pill; Escape or a backdrop click cancels). Confirming stops playback, cancels the
playlist's downloads (`cancel_playlist`), clears the input and returns to the home screen
(empty state) with fresh mood suggestions. The playlist file and cached audio are kept.

**Generate button** — circular, Spotify Green `#1ed760` background, black arrow icon,
inside or right next to the input. Disabled: `#1f1f1f` background, `#7c7c7c` icon.
While generating: spinner instead of the icon.

**Playlist header** — playlist title from the LLM (24px / 700, white) and a caption
*"12 songs"* (14px / 400, `#b3b3b3`).

**Track row** — height ~56px, radius 6px, no border.
- Left: index (14px, `#b3b3b3`); when playing → small animated equalizer icon in green.
- Middle: title (16px / 400, white; green `#1ed760` when current) and artist
  (14px / 400, `#b3b3b3`), both truncated with ellipsis.
- Right: status indicator —
  pending: nothing · searching/downloading: small percentage or thin circular progress in
  `#b3b3b3` · ready: nothing (duration `m:ss` in `#b3b3b3`) ·
  failed: warning icon in Negative Red `#f3727f`, tooltip with the reason, click → retry.
- Hover: background `#1f1f1f`. Cursor `pointer` only for ready rows.

**Player bar** — pinned to the bottom, background `#181818`, shadow
`rgba(0,0,0,0.5) 0px 8px 24px`.
- Now-playing line: title (14px / 700, white) · artist (14px / 400, `#b3b3b3`).
- Progress: 4px track `#4d4d4d`, fill white (green on hover), thumb visible on hover;
  times left/right in 12px `#b3b3b3`.
- Controls: Prev and Next — circular, transparent background, `#b3b3b3` icon (white on
  hover). Play/Pause — circular, 48px, Spotify Green background, black icon; scales to
  1.04 on hover.
- Volume (centered row below the controls, slider aligned under Play/Pause): speaker icon button (click → mute/unmute; icon shows
  high/low/muted) + slider (4px track like the progress bar, white fill, green on hover).
  Loudness follows a squared curve of the slider value. Volume and mute are saved in
  `config.json` (`player.volume`, `player.muted`) and restored on start.
- Disabled (no playlist): all controls `#4d4d4d`, no hover effects.
- Buffering state: Play/Pause shows a spinner.

**Settings panel** — modal dialog, background `#181818`, radius 8px, shadow
`rgba(0,0,0,0.5) 0px 8px 24px`, dark overlay behind.
- *OpenAI API key*: masked input (pill) + show/hide; status *"Key saved"* in `#b3b3b3`
  or error in `#f3727f`.
- *Model*: dropdown of supported OpenAI models, the default pre-selected.
- *Tools*: yt-dlp and ffmpeg versions, **Install / Update** button.
- *Storage*: path `~/.moodbeat` and cache size, **Clear cache** button (outlined pill).
- Buttons: **Save** (green pill, black uppercase label, letter-spacing 1.4px),
  **Close** (dark pill `#1f1f1f`).

**Empty state** (no playlist yet) — centered text in `#b3b3b3`:
*"Type a mood above and press Enter."* plus 3 example chips (dark pills:
*"90s rock"*, *"rainy day in New York"*, *"Polish summertime"*) that fill the input on click.

**Errors** — inline, under the input, 14px, Negative Red `#f3727f`. No `alert()` dialogs.

### 7.4 Typography

`SpotifyMixUI` is proprietary and is **not** bundled. Use the DESIGN.md fallback stack:
`"Helvetica Neue", Helvetica, Arial, "Hiragino Sans", "Hiragino Kaku Gothic ProN", Meiryo, sans-serif`
(on macOS this also works with `-apple-system` / `system-ui` first — decide in
implementation, keep one stack in a CSS variable). Only weights 400 and 700 (600 sparingly).
Size range 12–24px.

### 7.5 Colors (CSS custom properties)

```css
--bg:            #121212;
--surface:       #181818;
--surface-2:     #1f1f1f;
--text:          #ffffff;
--text-muted:    #b3b3b3;
--border:        #4d4d4d;
--border-light:  #7c7c7c;
--accent:        #1ed760;
--negative:      #f3727f;
--warning:       #ffa42b;
```

Green is used **only** for: Generate button, Play/Pause button, the currently playing
track title/equalizer, progress hover, primary Save button.

---

## 8. Data storage — `~/.moodbeat/`

```
~/.moodbeat/
├── config.json            # settings (incl. API key), file mode 0600
├── bin/                   # managed yt-dlp and ffmpeg binaries
│   ├── yt-dlp
│   └── ffmpeg
├── cache/
│   ├── audio/             # <videoId>.mp3
│   └── index.json         # artist|title → videoId, file size, last played
├── playlists/             # one JSON file per generated playlist
│   └── 2026-09-28T21-14-03_rainy-day-in-new-york.json
└── logs/
    └── moodbeat.log       # rolling daily, keep last 7 files
```

On Windows `~` resolves to `%USERPROFILE%`.

### 8.1 `config.json`

```json
{
  "version": 1,
  "openai": {
    "apiKey": "sk-...",
    "model": "gpt-4.1-mini"
  },
  "cache": {
    "maxSizeMb": 2048
  },
  "tools": {
    "lastYtDlpUpdateCheck": "2026-09-28T19:00:00Z"
  },
  "player": {
    "volume": 0.8,
    "muted": false
  }
}
```

- Created with permissions `0600` (owner read/write only) on macOS/Linux. The directory
  `~/.moodbeat` is `0700`.
- Writes are atomic (write to temp file + rename).
- The API key is never logged, never sent to the frontend, and never included in error messages.
- Env var `OPENAI_API_KEY` is used as a fallback when no key is saved (handy for development).

### 8.2 Playlist file

```json
{
  "version": 1,
  "id": "01J9Z...",
  "createdAt": "2026-09-28T21:14:03Z",
  "mood": "rainy day in New York",
  "title": "Rainy Manhattan Afternoon",
  "model": "gpt-4.1-mini",
  "tracks": [
    {
      "id": "t1",
      "artist": "Sting",
      "title": "Englishman in New York",
      "year": 1987,
      "videoId": "d27gTrPPAyk",
      "status": "ready",
      "durationSec": 265,
      "error": null
    }
  ]
}
```

### 8.3 What is not stored

- No listening analytics or telemetry. Nothing is sent anywhere except the mood text
  (to OpenAI) and search queries/downloads (to YouTube).

---

## 9. Error handling

| Situation | Behavior |
|-----------|----------|
| No API key | Generate disabled, hint *"Add your OpenAI API key in Settings"* with a link that opens Settings |
| Invalid key (401) | *"OpenAI rejected the API key. Check it in Settings."* |
| Quota / rate limit (429) | *"OpenAI rate limit or quota reached. Try again later."* |
| OpenAI timeout / network down | *"Couldn't reach OpenAI. Check your connection."* — input stays filled so the user can retry |
| LLM returned < 5 usable songs | *"Couldn't build a playlist for this mood. Try describing it differently."* |
| yt-dlp / ffmpeg missing and install failed | Blocking message in the list area with install instructions and a **Retry** button |
| Song not found on YouTube | Row `failed`, reason in tooltip, skipped in playback |
| Download failed (after retry) | Row `failed`, click to retry manually; hint to update yt-dlp in Settings |
| All tracks failed | *"None of the songs could be downloaded."* + suggestion to update yt-dlp |
| Disk full / can't write `~/.moodbeat` | *"Can't write to ~/.moodbeat: <reason>"* |
| Audio element error on a ready file | Delete the cached file, mark track `failed`, go to next |

All errors are also logged with details (without secrets) to `~/.moodbeat/logs/`.

---

## 10. Security & legal notes

- Tauri capabilities: allow only the commands listed in §4.3. The asset protocol scope is
  restricted to `$HOME/.moodbeat/cache/audio/**`. No shell/fs plugins exposed to the frontend.
- External processes are spawned with an argument array (never via a shell string),
  so a mood or song title can't inject commands.
- Downloaded binaries (yt-dlp, ffmpeg) are fetched over HTTPS; ffmpeg is verified
  against a pinned SHA-256 checksum.
- CSP: `default-src 'self'; media-src 'self' asset: http://asset.localhost; connect-src ipc: http://ipc.localhost`.
- Downloading from YouTube may conflict with YouTube's Terms of Service and with copyright in
  some countries. The app is meant for personal use; the README should state this, and the
  app must not provide any way to export or share downloaded files.

---

## 11. Project structure (proposed)

```
moodbeat/
├── SPEC.md
├── DESIGN.md
├── README.md
├── package.json
├── vite.config.ts
├── index.html
├── ui/                        # frontend
│   ├── main.ts                # bootstrapping, event wiring
│   ├── api.ts                 # typed wrappers for invoke() and listen()
│   ├── player.ts              # <audio> control, queue logic (next/prev/auto-advance)
│   ├── ui/
│   │   ├── moodInput.ts
│   │   ├── trackList.ts
│   │   ├── playerBar.ts
│   │   └── settings.ts
│   └── styles/
│       ├── tokens.css         # colors, radii, fonts from DESIGN.md
│       └── app.css
└── core/
    ├── Cargo.toml
    ├── tauri.conf.json
    ├── capabilities/default.json
    └── src/
        ├── main.rs
        ├── commands.rs        # #[tauri::command] functions
        ├── config.rs
        ├── paths.rs           # ~/.moodbeat layout
        ├── llm.rs             # OpenAI client + prompt + schema
        ├── resolver.rs        # yt-dlp search + scoring
        ├── downloader.rs      # queue, workers, progress, cancellation
        ├── cache.rs
        ├── playlist.rs
        ├── tools.rs           # yt-dlp/ffmpeg install & update
        └── error.rs
```

### 11.1 Testing

- **Rust unit tests**: LLM response parsing and post-processing (§5.4), video scoring
  (§6.2) with recorded yt-dlp JSON fixtures, yt-dlp progress-line parsing, config
  read/write and file permissions, cache eviction.
- **Frontend unit tests** (Vitest): player queue logic — auto-advance, skip failed,
  wait on buffering, prev/next edge cases, stop after the last track.
- **Integration (manual/optional, not in CI)**: real OpenAI call and real download of one
  short track, behind an env flag.

---

## 12. Future ideas (out of scope for v1)

- History screen: list of past playlists from `~/.moodbeat/playlists/`, replay from cache.
- "More like this" — extend the current playlist.
- Shuffle / repeat, drag-to-reorder.
- Other LLM providers (Anthropic, local models via Ollama).
- API key stored in the OS keychain instead of `config.json`.
- Export playlist as text / M3U.

---

## 13. Acceptance criteria (v1)

1. On first launch, `~/.moodbeat/` is created with the structure from §8, and the user is
   asked for an OpenAI API key.
2. An invalid API key is rejected with a clear message; a valid one is saved in
   `~/.moodbeat/config.json` with `0600` permissions.
3. Typing *"90s rock"* and pressing Enter shows a list of 10–15 songs within ~15 s
   (depends on the model).
4. The first song starts playing automatically as soon as it is downloaded, while the
   rest keep downloading with visible per-track progress.
5. Downloaded files are MP3s in `~/.moodbeat/cache/audio/`, and generating a playlist with
   an already cached song does not download it again.
6. When a song ends, the next one starts automatically; failed songs are skipped.
7. When the last song ends, playback stops and the player shows the finished state;
   pressing Play starts again from the first song.
8. Play/Pause, Previous and Next work as described in §3.4, including edge cases on the
   first and last track.
9. Generating a new playlist during playback stops the old one and cancels its downloads.
10. The UI matches DESIGN.md: dark theme, green used only as a functional accent, pill and
    circle shapes, compact typography.
11. No data is written outside `~/.moodbeat/` (except OS-managed WebView caches).
12. The API key never appears in logs, the frontend, or error messages.
