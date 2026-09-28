import './styles/tokens.css';
import './styles/app.css';

import {
  api,
  errorMessage,
  events,
  fileSrc,
  type Playlist,
  type Settings,
  type TrackReadyEvent,
  type TrackStatusEvent,
} from './api';
import { Player, type PlayerSnapshot } from './player';
import { h, icon, ICONS } from './ui/dom';
import { createConfirmDialog } from './ui/confirmDialog';
import { createMoodInput } from './ui/moodInput';
import { createPlayerBar } from './ui/playerBar';
import { createSettings } from './ui/settings';
import { createTrackList, DEFAULT_MOODS, type TrackView } from './ui/trackList';
import { createVolumeControl } from './ui/volumeControl';
import { VOLUME_STEP } from './volume';

// ---------------------------------------------------------------- state

let settings: Settings | null = null;
let playlist: Playlist | null = null;
let tracks = new Map<string, TrackView>();
let generating = false;
let toolsReady = false;
/** AI mood suggestions for the empty state; `null` while loading. */
let suggestions: readonly string[] | null = null;
let suggestionsRequest = 0;
/** Events that arrive while `generate_playlist` is still returning (their playlist id is not known yet). */
let earlyEvents: Array<{ kind: 'status'; e: TrackStatusEvent } | { kind: 'ready'; e: TrackReadyEvent }> = [];

// ---------------------------------------------------------------- UI

const app = document.getElementById('app')!;
const audio = h('audio', { preload: 'auto' });

const settingsBtn = h('button', { class: 'icon-btn', type: 'button', 'aria-label': 'Settings', title: 'Settings' }, icon(ICONS.gear));
// On macOS the window uses an overlay title bar: the header doubles as the drag area and
// leaves room for the traffic-light buttons.
document.documentElement.classList.toggle('platform-macos', navigator.userAgent.includes('Mac'));
const header = h(
  'header',
  { class: 'app-header', 'data-tauri-drag-region': true },
  h('h1', { class: 'wordmark', 'data-tauri-drag-region': true }, 'moodbeat'),
  settingsBtn,
);

const moodInput = createMoodInput({
  onGenerate: (mood) => void generate(mood),
  onOpenSettings: () => settingsPanel.open(),
  onClear: () => void clearPlaylist(),
});

const trackList = createTrackList({
  onPlay: (id) => {
    const t = tracks.get(id);
    if (!t || !playlist) return;
    if (t.status === 'ready') player.playTrack(id);
    else if (t.status === 'pending') void api.prioritizeTrack(playlist.id, id).catch(() => {});
  },
  onRetry: (id) => {
    if (playlist) void api.retryTrack(playlist.id, id).catch((e) => moodInput.setError(errorMessage(e)));
  },
  onExample: (mood) => {
    moodInput.setValue(mood);
    moodInput.focus();
  },
});

// Volume is saved to config.json, debounced while the slider is dragged.
let volumeSaveTimer: ReturnType<typeof setTimeout> | undefined;
const volume = createVolumeControl({
  audio,
  onChange: (v) => {
    clearTimeout(volumeSaveTimer);
    volumeSaveTimer = setTimeout(() => void api.setVolume(v.volume, v.muted).catch(() => {}), 300);
  },
});

const playerBar = createPlayerBar({
  onToggle: () => player.toggle(),
  onPrevious: () => player.previous(),
  onNext: () => player.next(),
  onSeek: (sec) => player.seek(sec),
  footer: volume.el,
});

const settingsPanel = createSettings({ onChanged: () => void refreshSettings() });
const confirmDialog = createConfirmDialog();

app.append(header, moodInput.el, trackList.el, playerBar.el, settingsPanel.el, confirmDialog.el, audio);
settingsBtn.addEventListener('click', () => settingsPanel.open());

// ---------------------------------------------------------------- player

const player = new Player(audio, onPlayerChange, (trackId) => {
  if (playlist) void api.trackPlayed(playlist.id, trackId).catch(() => {});
});

function onPlayerChange(s: PlayerSnapshot): void {
  const t = s.trackId ? tracks.get(s.trackId) : undefined;
  trackList.setCurrent(s.trackId, s.state);
  playerBar.update(s.state, t ? { title: t.title, artist: t.artist } : null, player.hasTracks());
  updateMediaSession(s, t);
}

function currentDuration(): number {
  if (Number.isFinite(audio.duration) && audio.duration > 0) return audio.duration;
  const id = player.snapshot().trackId;
  return (id && tracks.get(id)?.durationSec) || 0;
}

const syncTime = () => {
  const s = player.snapshot().state;
  if (s === 'playing' || s === 'paused') playerBar.setTime(audio.currentTime, currentDuration());
};
audio.addEventListener('timeupdate', syncTime);
audio.addEventListener('durationchange', syncTime);
audio.addEventListener('loadedmetadata', syncTime);
audio.addEventListener('ended', () => player.handleEnded());
audio.addEventListener('pause', () => player.handleAudioPaused());
audio.addEventListener('play', () => player.handleAudioPlaying());
audio.addEventListener('error', () => {
  const failedId = player.handleError();
  if (!failedId || !playlist) return;
  const t = tracks.get(failedId);
  if (t) updateTrackView({ ...t, status: 'failed', error: "Couldn't play the downloaded file" });
  void api.reportPlaybackError(playlist.id, failedId).catch(() => {});
});

// ---------------------------------------------------------------- backend events

function updateTrackView(t: TrackView): void {
  tracks.set(t.id, t);
  trackList.updateTrack(t);
}

function applyStatus(e: TrackStatusEvent): void {
  if (e.playlistId !== playlist?.id) {
    if (generating) earlyEvents.push({ kind: 'status', e });
    return;
  }
  const t = tracks.get(e.trackId);
  if (!t) return;
  updateTrackView({ ...t, status: e.status, progress: e.progress, error: e.error ?? null });
  // `ready` reaches the player together with the file path (track://ready).
  if (e.status !== 'ready') player.updateTrack(e.trackId, { status: e.status, src: undefined });
}

function applyReady(e: TrackReadyEvent): void {
  if (e.playlistId !== playlist?.id) {
    if (generating) earlyEvents.push({ kind: 'ready', e });
    return;
  }
  const t = tracks.get(e.trackId);
  if (!t) return;
  updateTrackView({ ...t, status: 'ready', durationSec: e.durationSec ?? t.durationSec, progress: undefined });
  player.updateTrack(e.trackId, { status: 'ready', src: fileSrc(e.filePath) });
}

void events.onTrackStatus(applyStatus);
void events.onTrackReady(applyReady);

// ---------------------------------------------------------------- generation

function renderEmptyState(): void {
  if (settings?.hasApiKey) trackList.showEmpty(suggestions, () => void loadSuggestions());
  else trackList.showEmpty(DEFAULT_MOODS);
}

/** Fresh AI suggestions each time; hardcoded moods if there's no key or the request fails. */
async function loadSuggestions(): Promise<void> {
  const request = ++suggestionsRequest;
  suggestions = null;
  if (!playlist && !generating) renderPlaylist();
  let moods: readonly string[];
  try {
    moods = await api.suggestMoods();
  } catch {
    moods = DEFAULT_MOODS;
  }
  if (request !== suggestionsRequest) return; // superseded by a newer request
  suggestions = moods;
  if (!playlist && !generating) renderPlaylist();
}

function renderPlaylist(): void {
  if (!playlist) {
    if (toolsReady) renderEmptyState();
    return;
  }
  trackList.setPlaylist({ ...playlist, tracks: [...tracks.values()] });
  for (const t of tracks.values()) trackList.updateTrack(t);
  const s = player.snapshot();
  trackList.setCurrent(s.trackId, s.state);
}

/** × in the input: back to the home screen. Asks first when a playlist would be discarded. */
async function clearPlaylist(): Promise<void> {
  if (generating) return;
  if (!playlist) {
    moodInput.setValue('');
    moodInput.focus();
    return;
  }
  const ok = await confirmDialog.ask({
    title: 'Clear playlist?',
    message: `This stops playback and removes “${playlist.title}” from the screen. Downloaded songs stay in the cache.`,
    confirmLabel: 'Clear',
  });
  if (!ok || !playlist || generating) return;

  const id = playlist.id;
  playlist = null;
  tracks = new Map();
  player.clear(); // stops audio, disables the player bar
  void api.cancelPlaylist(id).catch(() => {});
  moodInput.setValue('');
  moodInput.setClearable(false);
  if (settings?.hasApiKey) void loadSuggestions(); // fresh ideas for the home screen
  else renderPlaylist();
  moodInput.focus();
}

async function generate(mood: string): Promise<void> {
  if (generating) return;
  if (!toolsReady) {
    moodInput.setError('yt-dlp and ffmpeg are still being set up.');
    return;
  }
  generating = true;
  earlyEvents = [];
  moodInput.setError(null);
  moodInput.setGenerating(true);
  trackList.showLoading(mood);
  try {
    const next = await api.generatePlaylist(mood);
    playlist = next;
    moodInput.setClearable(true);
    tracks = new Map(next.tracks.map((t) => [t.id, { ...t }]));
    renderPlaylist();
    // Replaces the old queue: stops playback, auto-starts track #1 when ready.
    player.load(next.tracks.map((t) => ({ id: t.id, status: t.status })));
    for (const ev of earlyEvents) {
      if (ev.kind === 'status') applyStatus(ev.e);
      else applyReady(ev.e);
    }
  } catch (e) {
    moodInput.setError(errorMessage(e));
    renderPlaylist();
  } finally {
    earlyEvents = [];
    generating = false;
    moodInput.setGenerating(false);
  }
}

// ---------------------------------------------------------------- settings & tools

async function refreshSettings(): Promise<void> {
  try {
    const first = settings === null;
    const hadKey = settings?.hasApiKey;
    settings = await api.getSettings();
    if (first) volume.restore({ volume: settings.volume, muted: settings.muted });
    moodInput.setHasApiKey(settings.hasApiKey);
    if (settings.hasApiKey !== hadKey) {
      if (settings.hasApiKey) void loadSuggestions();
      else {
        suggestionsRequest++; // drop any in-flight AI suggestions
        renderPlaylist();
      }
    }
  } catch (e) {
    moodInput.setError(errorMessage(e));
  }
  if (!toolsReady) void ensureTools();
}

const INSTALL_HELP = navigator.userAgent.includes('Mac')
  ? 'Install them manually with Homebrew: brew install yt-dlp ffmpeg'
  : navigator.userAgent.includes('Windows')
    ? 'Install them manually: winget install yt-dlp.yt-dlp Gyan.FFmpeg'
    : 'Install yt-dlp and ffmpeg with your package manager (Linux also needs GStreamer plugins for MP3).';

let ensuringTools = false;
async function ensureTools(): Promise<void> {
  if (ensuringTools) return;
  ensuringTools = true;
  try {
    const status = await api.getToolsStatus();
    if (status.ytDlp.version && status.ffmpeg.version) {
      toolsReady = true;
      renderPlaylist();
      return;
    }
    const line = h('p', {}, 'Setting up yt-dlp and ffmpeg…');
    trackList.showBlocking(h('div', { class: 'blocking' }, h('span', { class: 'spinner' }), line));
    const unlisten = await events.onToolsProgress((p) => {
      line.textContent = `${p.message} ${p.progress < 100 ? `${Math.floor(p.progress)}%` : ''}`;
    });
    try {
      await api.installTools();
      toolsReady = true;
      renderPlaylist();
    } finally {
      unlisten();
    }
  } catch (e) {
    const retry = h('button', { class: 'dark-pill', type: 'button' }, 'Retry');
    retry.addEventListener('click', () => void ensureTools());
    trackList.showBlocking(
      h(
        'div',
        { class: 'blocking' },
        h('p', { class: 'is-error' }, `Couldn't set up yt-dlp and ffmpeg: ${errorMessage(e)}`),
        h('p', {}, INSTALL_HELP),
        retry,
      ),
    );
  } finally {
    ensuringTools = false;
  }
}

// ---------------------------------------------------------------- keyboard & media keys

document.addEventListener('keydown', (e) => {
  if (settingsPanel.isOpen() || confirmDialog.isOpen() || e.metaKey || e.ctrlKey || e.altKey) return;
  if (e.target instanceof Element && e.target.closest('input, textarea, select, [contenteditable]')) return;
  if (e.key === ' ') {
    e.preventDefault();
    player.toggle();
  } else if (e.key === 'ArrowRight') {
    e.preventDefault();
    player.next();
  } else if (e.key === 'ArrowLeft') {
    e.preventDefault();
    player.previous();
  } else if (e.key === 'ArrowUp' || e.key === 'ArrowDown') {
    e.preventDefault();
    volume.step(e.key === 'ArrowUp' ? VOLUME_STEP : -VOLUME_STEP);
  } else if (e.key === 'm' || e.key === 'M') {
    volume.toggleMute();
  }
});

function updateMediaSession(s: PlayerSnapshot, t: TrackView | undefined): void {
  if (!('mediaSession' in navigator)) return;
  navigator.mediaSession.playbackState = s.state === 'playing' ? 'playing' : s.state === 'paused' ? 'paused' : 'none';
  navigator.mediaSession.metadata =
    t && s.state !== 'finished' && typeof MediaMetadata !== 'undefined'
      ? new MediaMetadata({ title: t.title, artist: t.artist, album: playlist?.title ?? '' })
      : null;
}

if ('mediaSession' in navigator) {
  const handlers: Array<[MediaSessionAction, () => void]> = [
    ['play', () => player.snapshot().state !== 'playing' && player.toggle()],
    ['pause', () => player.snapshot().state === 'playing' && player.toggle()],
    ['nexttrack', () => player.next()],
    ['previoustrack', () => player.previous()],
  ];
  for (const [action, handler] of handlers) {
    try {
      navigator.mediaSession.setActionHandler(action, handler);
    } catch {
      // Unsupported action in this WebView.
    }
  }
}

// ---------------------------------------------------------------- start

trackList.showEmpty(null);
playerBar.update('idle', null, false);
void refreshSettings().then(() => {
  if (settings && !settings.hasApiKey) settingsPanel.open('Add your OpenAI API key to start.');
  else moodInput.focus();
});
