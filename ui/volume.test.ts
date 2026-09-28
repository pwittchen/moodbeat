import { describe, expect, it } from 'vitest';
import { gain, setLevel, step, toggleMute, UNMUTE_DEFAULT, volumeLevel } from './volume';

describe('volume', () => {
  it('applies a squared loudness curve and silences when muted', () => {
    expect(gain({ volume: 1, muted: false })).toBe(1);
    expect(gain({ volume: 0.5, muted: false })).toBe(0.25);
    expect(gain({ volume: 0.8, muted: true })).toBe(0);
    expect(gain({ volume: 7, muted: false })).toBe(1);
    expect(gain({ volume: NaN, muted: false })).toBe(1);
  });

  it('slider unmutes at any audible level and mutes at zero', () => {
    expect(setLevel({ volume: 0.4, muted: true }, 0.3)).toEqual({ volume: 0.3, muted: false });
    expect(setLevel({ volume: 0.4, muted: false }, 0)).toEqual({ volume: 0, muted: true });
  });

  it('steps within bounds', () => {
    expect(step({ volume: 0.5, muted: false }, 0.05)).toEqual({ volume: 0.55, muted: false });
    expect(step({ volume: 0.98, muted: false }, 0.05)).toEqual({ volume: 1, muted: false });
    expect(step({ volume: 0.03, muted: false }, -0.05)).toEqual({ volume: 0, muted: true });
    // Stepping up from mute starts from silence, not from the old level.
    expect(step({ volume: 0.8, muted: true }, 0.05)).toEqual({ volume: 0.05, muted: false });
  });

  it('toggles mute and restores an audible level', () => {
    expect(toggleMute({ volume: 0.7, muted: false })).toEqual({ volume: 0.7, muted: true });
    expect(toggleMute({ volume: 0.7, muted: true })).toEqual({ volume: 0.7, muted: false });
    expect(toggleMute({ volume: 0, muted: true })).toEqual({ volume: UNMUTE_DEFAULT, muted: false });
  });

  it('picks an icon level', () => {
    expect(volumeLevel({ volume: 0.9, muted: false })).toBe('high');
    expect(volumeLevel({ volume: 0.2, muted: false })).toBe('low');
    expect(volumeLevel({ volume: 0.9, muted: true })).toBe('off');
  });
});
