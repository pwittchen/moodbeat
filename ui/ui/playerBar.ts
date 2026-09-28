import type { PlayerState } from '../player';
import { formatTime, h, icon, ICONS } from './dom';

export interface NowPlaying {
  title: string;
  artist: string;
}

export interface PlayerBar {
  el: HTMLElement;
  update(state: PlayerState, track: NowPlaying | null, enabled: boolean): void;
  setTime(current: number, duration: number): void;
}

export function createPlayerBar(opts: {
  onToggle: () => void;
  onPrevious: () => void;
  onNext: () => void;
  onSeek: (seconds: number) => void;
  /** Centered row below the playback controls (volume). */
  footer?: HTMLElement;
}): PlayerBar {
  const title = h('span', { class: 'now-title' });
  const artist = h('span', { class: 'now-artist' });
  const nowPlaying = h('div', { class: 'now-playing' }, title, artist);

  const elapsed = h('span', { class: 'time' }, '0:00');
  const total = h('span', { class: 'time time--total' }, '0:00');
  const fill = h('div', { class: 'progress-fill' });
  const thumb = h('div', { class: 'progress-thumb' });
  const bar = h(
    'div',
    { class: 'progress', role: 'slider', 'aria-label': 'Seek', tabindex: '-1' },
    h('div', { class: 'progress-track' }, fill, thumb),
  );
  const progressRow = h('div', { class: 'progress-row' }, elapsed, bar, total);

  const prev = h('button', { class: 'ctrl-btn', type: 'button', 'aria-label': 'Previous', title: 'Previous' }, icon(ICONS.prev));
  const toggle = h('button', { class: 'play-btn', type: 'button', 'aria-label': 'Play' });
  const next = h('button', { class: 'ctrl-btn', type: 'button', 'aria-label': 'Next', title: 'Next' }, icon(ICONS.next));
  prev.addEventListener('click', opts.onPrevious);
  toggle.addEventListener('click', opts.onToggle);
  next.addEventListener('click', opts.onNext);

  const el = h(
    'footer',
    { class: 'player' },
    nowPlaying,
    progressRow,
    h('div', { class: 'controls' }, prev, toggle, next),
    ...(opts.footer ? [h('div', { class: 'player-footer' }, opts.footer)] : []),
  );

  let duration = 0;
  let seeking = false;
  let seekable = false;

  const setFraction = (f: number) => {
    const pct = `${Math.min(100, Math.max(0, f * 100))}%`;
    fill.style.width = pct;
    thumb.style.left = pct;
  };
  const fractionAt = (clientX: number) => {
    const rect = bar.getBoundingClientRect();
    return Math.min(1, Math.max(0, (clientX - rect.left) / rect.width));
  };

  bar.addEventListener('pointerdown', (e) => {
    if (!seekable || duration <= 0) return;
    seeking = true;
    bar.setPointerCapture(e.pointerId);
    const f = fractionAt(e.clientX);
    setFraction(f);
    elapsed.textContent = formatTime(f * duration);
  });
  bar.addEventListener('pointermove', (e) => {
    if (!seeking) return;
    const f = fractionAt(e.clientX);
    setFraction(f);
    elapsed.textContent = formatTime(f * duration);
  });
  const endSeek = (e: PointerEvent) => {
    if (!seeking) return;
    seeking = false;
    opts.onSeek(fractionAt(e.clientX) * duration);
  };
  bar.addEventListener('pointerup', endSeek);
  bar.addEventListener('pointercancel', () => (seeking = false));

  return {
    el,
    update(state, track, enabled) {
      el.classList.toggle('is-disabled', !enabled);
      prev.disabled = next.disabled = toggle.disabled = !enabled;
      seekable = enabled && (state === 'playing' || state === 'paused');
      bar.classList.toggle('is-seekable', seekable);

      if (state === 'finished') {
        title.textContent = 'Playlist finished';
        artist.textContent = 'Press play to start again';
      } else if (track) {
        title.textContent = track.title;
        artist.textContent = state === 'buffering' ? 'Waiting for download…' : track.artist;
      } else {
        title.textContent = enabled ? '' : 'Nothing playing';
        artist.textContent = '';
      }

      const playing = state === 'playing';
      toggle.setAttribute('aria-label', playing ? 'Pause' : 'Play');
      toggle.title = playing ? 'Pause' : 'Play';
      toggle.replaceChildren(
        state === 'buffering' ? h('span', { class: 'spinner spinner--dark' }) : icon(playing ? ICONS.pause : ICONS.play),
      );
      if (state === 'finished' || state === 'buffering' || state === 'idle' || !enabled) {
        this.setTime(0, state === 'buffering' || state === 'finished' ? 0 : duration);
      }
    },
    setTime(current, dur) {
      duration = Number.isFinite(dur) ? dur : 0;
      total.textContent = formatTime(duration);
      if (seeking) return;
      elapsed.textContent = formatTime(current);
      setFraction(duration > 0 ? current / duration : 0);
    },
  };
}
