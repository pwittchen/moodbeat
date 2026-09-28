// Volume model: the slider is linear (0–1), loudness is applied on a squared curve so
// equal slider steps sound like roughly equal changes in loudness.

export const VOLUME_STEP = 0.05;
/** Volume restored when unmuting from a slider at zero. */
export const UNMUTE_DEFAULT = 0.5;

export interface VolumeState {
  volume: number;
  muted: boolean;
}

const clamp = (v: number) => (Number.isFinite(v) ? Math.min(1, Math.max(0, v)) : 1);

/** Value for `HTMLMediaElement.volume`. */
export function gain(s: VolumeState): number {
  return s.muted ? 0 : clamp(s.volume) ** 2;
}

/** Slider moved: any audible position unmutes. */
export function setLevel(_s: VolumeState, volume: number): VolumeState {
  const v = clamp(volume);
  return { volume: v, muted: v === 0 };
}

export function step(s: VolumeState, delta: number): VolumeState {
  const base = s.muted ? 0 : s.volume;
  return setLevel(s, Math.round((base + delta) * 100) / 100);
}

export function toggleMute(s: VolumeState): VolumeState {
  if (s.muted || s.volume === 0) return { volume: s.volume > 0 ? s.volume : UNMUTE_DEFAULT, muted: false };
  return { ...s, muted: true };
}

export function volumeLevel(s: VolumeState): 'off' | 'low' | 'high' {
  if (s.muted || s.volume === 0) return 'off';
  return s.volume < 0.5 ? 'low' : 'high';
}
