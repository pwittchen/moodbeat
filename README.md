<p align="center">
  <img src="core/icons/app-icon.png" alt="moodbeat logo" width="180">
</p>

<h1 align="center">moodbeat</h1>

<p align="center">
  music player creating custom playlist depending on your vibe
</p>

<p align="center">
  Type a mood (<em>"90s rock"</em>, <em>"rainy day in New York"</em>, <em>"Polish summertime"</em>),
  and moodbeat asks OpenAI for 10–15 matching songs, finds them on YouTube, downloads them as MP3
  and plays them in order.
</p>

<p align="center">
  <a href="https://github.com/pwittchen/moodbeat/actions/workflows/ci.yml"><img src="https://github.com/pwittchen/moodbeat/actions/workflows/ci.yml/badge.svg" alt="CI"></a>
</p>

<p align="center">
  <a href="./SPEC.md">Specification</a> · <a href="./DESIGN.md">Design system</a>
</p>

<p align="center">
  <img src="screenshot_home.png" alt="moodbeat home screen with mood suggestions" width="266">
  <img src="screenshot_playlist.png" alt="moodbeat playing a “rain-soaked city noir” playlist" width="266">
  <img src="screenshot_settings.png" alt="moodbeat settings" width="266">
</p>

---

## Requirements

- Rust (stable) and Node.js 20+
- Tauri 2 system dependencies: <https://v2.tauri.app/start/prerequisites/>
  (on Linux also GStreamer plugins for MP3 playback, e.g. `gstreamer1.0-plugins-good`)
- An OpenAI API key (entered in Settings, or `OPENAI_API_KEY` in the environment)
- `yt-dlp` and `ffmpeg` — found on `PATH`, or installed automatically into `~/.moodbeat/bin/`

## Development

```sh
npm install
npm run tauri dev      # run the app
npm run tauri build    # build a release bundle
npm run kill           # stop a running dev session (Vite + app)
```

Tests:

```sh
npm test                                         # frontend (player queue logic)
cd core && cargo test                            # backend unit tests
MOODBEAT_INTEGRATION=1 cargo test -- --ignored   # real YouTube search + download
```

All app data (settings, audio cache, playlists, logs) lives in `~/.moodbeat/`.

To change the app icon, replace `logo.png` (a rounded-square icon on a white background) and run:

```sh
python3 scripts/make-icon.py && npx tauri icon core/icons/app-icon.png -o core/icons
rm -rf core/icons/android core/icons/ios && touch core/build.rs
```

## Release

Pushing a `vX.Y.Z` tag runs the [Release](.github/workflows/release.yml) workflow: it bumps the
version on `master` to match the tag, builds the app for Apple Silicon, signs it with the
Developer ID, notarizes and staples it, and publishes `moodbeat-macos-aarch64.dmg` to GitHub
Releases.

```sh
git tag v0.2.0 && git push origin v0.2.0
```

## Legal note

Downloading from YouTube may conflict with YouTube's Terms of Service and with copyright law
in some countries. moodbeat is meant for personal use only and intentionally provides no way to
export or share downloaded files.
