import type { Playlist, Track } from '../api';
import type { PlayerState } from '../player';
import { formatTime, h, icon, ICONS } from './dom';

/** Shown when no API key is set (or AI suggestions fail). */
export const DEFAULT_MOODS: readonly string[] = ['90s rock', 'rainy day in New York', 'Polish summertime'];

export interface TrackView extends Track {
  progress?: number;
}

export interface TrackList {
  el: HTMLElement;
  /** `moods === null` shows placeholder chips while suggestions load; `onRefresh` adds a shuffle button. */
  showEmpty(moods: readonly string[] | null, onRefresh?: () => void): void;
  showLoading(mood: string): void;
  showBlocking(content: Node): void;
  setPlaylist(playlist: Playlist): void;
  updateTrack(track: TrackView): void;
  setCurrent(trackId: string | null, state: PlayerState): void;
}

interface Row {
  row: HTMLElement;
  index: HTMLElement;
  title: HTMLElement;
  status: HTMLElement;
  track: TrackView;
}

export function createTrackList(opts: {
  onPlay: (trackId: string) => void;
  onRetry: (trackId: string) => void;
  onExample: (mood: string) => void;
}): TrackList {
  const el = h('main', { class: 'content' });
  let rows = new Map<string, Row>();
  let summary: HTMLElement | null = null;
  let current: { id: string | null; state: PlayerState } = { id: null, state: 'idle' };

  const renderRow = (r: Row) => {
    const t = r.track;
    const isCurrent = current.id === t.id && current.state !== 'finished' && current.state !== 'idle';
    const isPlaying = isCurrent && current.state === 'playing';
    r.row.classList.toggle('is-current', isCurrent);
    r.row.classList.toggle('is-ready', t.status === 'ready');
    r.row.classList.toggle('is-failed', t.status === 'failed');
    r.row.setAttribute('aria-current', isCurrent ? 'true' : 'false');

    r.index.replaceChildren(
      isPlaying
        ? h('span', { class: 'eq', 'aria-label': 'Playing' }, h('i'), h('i'), h('i'))
        : String(Number(t.id.replace(/\D/g, '')) || ''),
    );

    r.status.removeAttribute('title');
    switch (t.status) {
      case 'pending':
        r.status.replaceChildren();
        break;
      case 'searching':
        r.status.replaceChildren(h('span', { class: 'spinner spinner--small', 'aria-label': 'Searching' }));
        break;
      case 'downloading':
        r.status.replaceChildren(
          t.progress != null && t.progress > 0
            ? h('span', { class: 'progress-pct' }, `${Math.floor(t.progress)}%`)
            : h('span', { class: 'spinner spinner--small', 'aria-label': 'Downloading' }),
        );
        break;
      case 'ready':
        r.status.replaceChildren(h('span', { class: 'duration' }, t.durationSec ? formatTime(t.durationSec) : ''));
        break;
      case 'failed': {
        const reason = t.error ?? 'Failed';
        const hint = reason.startsWith('Download failed') ? ' Updating yt-dlp in Settings may help.' : '';
        r.status.title = `${reason.replace(/[.!]?$/, '.')}${hint} Click to retry.`;
        const btn = h(
          'button',
          { class: 'retry-btn', type: 'button', 'aria-label': `Retry: ${reason}` },
          icon(ICONS.warning),
          icon(ICONS.retry),
        );
        btn.addEventListener('click', (e) => {
          e.stopPropagation();
          opts.onRetry(t.id);
        });
        r.status.replaceChildren(btn);
        break;
      }
    }
  };

  const renderSummary = () => {
    if (!summary) return;
    const all = [...rows.values()];
    const allFailed = all.length > 0 && all.every((r) => r.track.status === 'failed');
    summary.hidden = !allFailed;
  };

  return {
    el,
    showEmpty(moods, onRefresh) {
      rows = new Map();
      const loading = moods === null;
      const chips = h('div', { class: 'chips', 'aria-busy': loading ? 'true' : undefined });
      if (loading) {
        for (let i = 0; i < 3; i++) chips.append(h('span', { class: 'chip chip--skeleton', 'aria-hidden': 'true' }));
      } else {
        for (const mood of moods) {
          const chip = h('button', { class: 'chip', type: 'button' }, mood);
          chip.addEventListener('click', () => opts.onExample(mood));
          chips.append(chip);
        }
      }
      const empty = h('div', { class: 'empty' }, h('p', {}, 'Type a mood above and press Enter.'), chips);
      if (onRefresh) {
        const shuffle = h(
          'button',
          {
            class: `icon-btn shuffle-btn${loading ? ' is-loading' : ''}`,
            type: 'button',
            'aria-label': 'Suggest other moods',
            title: 'Suggest other moods',
            disabled: loading,
          },
          icon(ICONS.shuffle),
        );
        shuffle.addEventListener('click', onRefresh);
        empty.append(shuffle);
      }
      el.replaceChildren(empty);
    },
    showLoading(mood) {
      rows = new Map();
      el.replaceChildren(
        h('div', { class: 'empty' }, h('span', { class: 'spinner' }), h('p', {}, `Finding songs for “${mood}”…`)),
      );
    },
    showBlocking(content) {
      rows = new Map();
      el.replaceChildren(h('div', { class: 'empty' }, content));
    },
    setPlaylist(playlist) {
      rows = new Map();
      const list = h('ol', { class: 'tracks' });
      for (const track of playlist.tracks) {
        const index = h('span', { class: 'track-index' });
        const title = h('span', { class: 'track-title' }, track.title);
        const artist = h('span', { class: 'track-artist' }, track.artist);
        const status = h('span', { class: 'track-status' });
        const row = h(
          'li',
          { class: 'track', tabindex: '-1' },
          index,
          h('span', { class: 'track-meta' }, title, artist),
          status,
        );
        row.addEventListener('click', () => opts.onPlay(track.id));
        const r: Row = { row, index, title, status, track: { ...track } };
        rows.set(track.id, r);
        renderRow(r);
        list.append(row);
      }
      const count = playlist.tracks.length;
      summary = h(
        'p',
        { class: 'all-failed', hidden: true },
        'None of the songs could be downloaded. Updating yt-dlp in Settings may help.',
      );
      el.replaceChildren(
        h(
          'header',
          { class: 'playlist-header' },
          h('h2', { class: 'playlist-title' }, playlist.title),
          h('p', { class: 'caption' }, `${count} ${count === 1 ? 'song' : 'songs'}`),
          summary,
        ),
        list,
      );
      renderSummary();
    },
    updateTrack(track) {
      const r = rows.get(track.id);
      if (!r) return;
      r.track = { ...track };
      renderRow(r);
      renderSummary();
    },
    setCurrent(trackId, state) {
      const prev = current.id;
      current = { id: trackId, state };
      for (const id of new Set([prev, trackId])) {
        const r = id ? rows.get(id) : undefined;
        if (r) renderRow(r);
      }
      if (trackId && prev !== trackId && state !== 'finished') {
        rows.get(trackId)?.row.scrollIntoView({ block: 'nearest', behavior: 'smooth' });
      }
    },
  };
}
