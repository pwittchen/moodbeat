import { gain, setLevel, step, toggleMute, volumeLevel, type VolumeState } from '../volume';
import { h, icon, ICONS } from './dom';

export interface VolumeControl {
  el: HTMLElement;
  /** Applies a saved state without reporting it back. */
  restore(state: VolumeState): void;
  step(delta: number): void;
  toggleMute(): void;
}

/** Mute button + slider. Drives `audio.volume` and reports user changes via `onChange`. */
export function createVolumeControl(opts: {
  audio: HTMLAudioElement;
  onChange: (state: VolumeState) => void;
}): VolumeControl {
  const button = h('button', { class: 'volume-btn', type: 'button' });
  const slider = h('input', {
    class: 'volume-slider',
    type: 'range',
    min: '0',
    max: '100',
    step: '1',
    'aria-label': 'Volume',
  });
  // The spacer mirrors the button so the slider is centered under the play button.
  const el = h('div', { class: 'volume' }, button, slider, h('span', { class: 'volume-spacer', 'aria-hidden': 'true' }));

  let state: VolumeState = { volume: 1, muted: false };

  const render = () => {
    opts.audio.volume = gain(state);
    const shown = state.muted ? 0 : state.volume;
    slider.value = String(Math.round(shown * 100));
    slider.style.setProperty('--fill', `${shown * 100}%`);
    slider.setAttribute('aria-valuetext', state.muted ? 'Muted' : `${Math.round(state.volume * 100)}%`);
    const level = volumeLevel(state);
    const label = level === 'off' ? 'Unmute' : 'Mute';
    button.setAttribute('aria-label', label);
    button.title = label;
    button.replaceChildren(icon(level === 'off' ? ICONS.volumeOff : level === 'low' ? ICONS.volumeLow : ICONS.volumeHigh));
  };

  const update = (next: VolumeState) => {
    state = next;
    render();
    opts.onChange(state);
  };

  slider.addEventListener('input', () => update(setLevel(state, Number(slider.value) / 100)));
  button.addEventListener('click', () => update(toggleMute(state)));

  render();
  return {
    el,
    restore(s) {
      state = { volume: s.volume, muted: s.muted };
      render();
    },
    step: (delta) => update(step(state, delta)),
    toggleMute: () => update(toggleMute(state)),
  };
}
