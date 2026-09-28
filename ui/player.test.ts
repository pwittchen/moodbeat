import { beforeEach, describe, expect, it } from 'vitest';
import { AudioLike, Player, QueueTrack, TrackStatus } from './player';

class FakeAudio implements AudioLike {
  src = '';
  currentTime = 0;
  paused = true;
  ended = false;
  plays = 0;
  play(): Promise<void> {
    this.paused = false;
    this.plays++;
    return Promise.resolve();
  }
  pause(): void {
    this.paused = true;
  }
  removeAttribute(name: string): void {
    if (name === 'src') this.src = '';
  }
  load(): void {}
}

const t = (id: string, status: TrackStatus = 'pending'): QueueTrack => ({
  id,
  status,
  src: status === 'ready' ? `asset://${id}.mp3` : undefined,
});

const ready = (id: string) => ({ status: 'ready' as const, src: `asset://${id}.mp3` });

describe('Player', () => {
  let audio: FakeAudio;
  let player: Player;
  let started: string[];

  beforeEach(() => {
    audio = new FakeAudio();
    started = [];
    player = new Player(audio, () => {}, (id) => started.push(id));
  });

  const current = () => player.snapshot();

  describe('auto-start', () => {
    it('waits for track #1 and starts it as soon as it is ready', () => {
      player.load([t('a'), t('b')]);
      expect(current().state).toBe('buffering');
      player.updateTrack('b', ready('b'));
      expect(current().state).toBe('buffering');
      player.updateTrack('a', ready('a'));
      expect(current()).toMatchObject({ state: 'playing', trackId: 'a' });
      expect(audio.src).toBe('asset://a.mp3');
      expect(started).toEqual(['a']);
    });

    it('skips a failed first track', () => {
      player.load([t('a'), t('b')]);
      player.updateTrack('a', { status: 'failed' });
      expect(current()).toMatchObject({ state: 'buffering', trackId: 'b' });
      player.updateTrack('b', ready('b'));
      expect(current()).toMatchObject({ state: 'playing', trackId: 'b' });
    });

    it('finishes when every track failed', () => {
      player.load([t('a'), t('b')]);
      player.updateTrack('a', { status: 'failed' });
      player.updateTrack('b', { status: 'failed' });
      expect(current().state).toBe('finished');
    });

    it('does not start without autoplay', () => {
      player.load([t('a', 'ready')], false);
      expect(current().state).toBe('idle');
      expect(audio.plays).toBe(0);
    });
  });

  describe('auto-advance', () => {
    it('plays the next track when one ends', () => {
      player.load([t('a', 'ready'), t('b', 'ready')]);
      player.handleEnded();
      expect(current()).toMatchObject({ state: 'playing', trackId: 'b' });
    });

    it('skips failed tracks silently', () => {
      player.load([t('a', 'ready'), t('b', 'failed'), t('c', 'ready')]);
      player.handleEnded();
      expect(current().trackId).toBe('c');
    });

    it('buffers on a track that is still downloading, then plays it', () => {
      player.load([t('a', 'ready'), t('b', 'downloading'), t('c', 'ready')]);
      player.handleEnded();
      expect(current()).toMatchObject({ state: 'buffering', trackId: 'b' });
      expect(audio.src).toBe('');
      player.updateTrack('b', ready('b'));
      expect(current()).toMatchObject({ state: 'playing', trackId: 'b' });
    });

    it('moves on when the buffered track fails', () => {
      player.load([t('a', 'ready'), t('b', 'searching'), t('c', 'ready')]);
      player.handleEnded();
      player.updateTrack('b', { status: 'failed' });
      expect(current()).toMatchObject({ state: 'playing', trackId: 'c' });
    });

    it('stops after the last track and resets to #1; Play restarts from #1', () => {
      player.load([t('a', 'ready'), t('b', 'ready')]);
      player.handleEnded();
      player.handleEnded();
      expect(current()).toMatchObject({ state: 'finished', index: 0 });
      expect(audio.paused).toBe(true);
      player.toggle();
      expect(current()).toMatchObject({ state: 'playing', trackId: 'a' });
    });

    it('treats all remaining failed as the end', () => {
      player.load([t('a', 'ready'), t('b', 'failed'), t('c', 'failed')]);
      player.handleEnded();
      expect(current().state).toBe('finished');
    });
  });

  describe('play/pause', () => {
    it('toggles', () => {
      player.load([t('a', 'ready')]);
      player.toggle();
      expect(current().state).toBe('paused');
      expect(audio.paused).toBe(true);
      player.toggle();
      expect(current().state).toBe('playing');
      expect(audio.paused).toBe(false);
    });

    it('keeps the position on resume', () => {
      player.load([t('a', 'ready')]);
      audio.currentTime = 42;
      player.toggle();
      player.toggle();
      expect(audio.currentTime).toBe(42);
    });

    it('stops waiting while buffering and resumes waiting on play', () => {
      player.load([t('a')]);
      player.toggle();
      expect(current().state).toBe('paused');
      player.updateTrack('a', ready('a'));
      expect(current().state).toBe('paused');
      player.toggle();
      expect(current()).toMatchObject({ state: 'playing', trackId: 'a' });
    });

    it('reflects OS-initiated pause, ignoring the pause fired at the end of a track', () => {
      player.load([t('a', 'ready')]);
      audio.ended = true;
      player.handleAudioPaused();
      expect(current().state).toBe('playing');
      audio.ended = false;
      player.handleAudioPaused();
      expect(current().state).toBe('paused');
      player.handleAudioPlaying();
      expect(current().state).toBe('playing');
    });
  });

  describe('next', () => {
    it('jumps to the next ready track, skipping not-ready ones', () => {
      player.load([t('a', 'ready'), t('b', 'downloading'), t('c', 'ready')]);
      player.next();
      expect(current().trackId).toBe('c');
    });

    it('waits on the next pending track when nothing later is ready', () => {
      player.load([t('a', 'ready'), t('b', 'failed'), t('c', 'downloading')]);
      player.next();
      expect(current()).toMatchObject({ state: 'buffering', trackId: 'c' });
    });

    it('stops on the last track', () => {
      player.load([t('a', 'ready'), t('b', 'ready')]);
      player.next();
      player.next();
      expect(current()).toMatchObject({ state: 'finished', index: 0 });
    });
  });

  describe('previous', () => {
    it('restarts the current track after 3 s', () => {
      player.load([t('a', 'ready'), t('b', 'ready')]);
      player.next();
      audio.currentTime = 10;
      player.previous();
      expect(current().trackId).toBe('b');
      expect(audio.currentTime).toBe(0);
    });

    it('goes to the previous ready track within the first 3 s', () => {
      player.load([t('a', 'ready'), t('b', 'failed'), t('c', 'ready')]);
      player.next();
      expect(current().trackId).toBe('c');
      audio.currentTime = 1;
      player.previous();
      expect(current().trackId).toBe('a');
      expect(audio.src).toBe('asset://a.mp3');
    });

    it('restarts the first track', () => {
      player.load([t('a', 'ready'), t('b', 'ready')]);
      audio.currentTime = 2;
      player.previous();
      expect(current()).toMatchObject({ state: 'playing', trackId: 'a' });
      expect(audio.currentTime).toBe(0);
    });
  });

  describe('row click & errors', () => {
    it('plays a ready track and ignores others', () => {
      player.load([t('a', 'ready'), t('b', 'pending'), t('c', 'ready')]);
      expect(player.playTrack('b')).toBe(false);
      expect(current().trackId).toBe('a');
      expect(player.playTrack('c')).toBe(true);
      expect(current().trackId).toBe('c');
    });

    it('marks the track failed on an audio error and moves on', () => {
      player.load([t('a', 'ready'), t('b', 'ready')]);
      expect(player.handleError()).toBe('a');
      expect(current()).toMatchObject({ state: 'playing', trackId: 'b' });
      player.previous();
      expect(current().trackId).toBe('b');
    });

    it('a new playlist replaces the old one and stops playback', () => {
      player.load([t('a', 'ready')]);
      player.load([t('x')]);
      expect(audio.src).toBe('');
      expect(current()).toMatchObject({ state: 'buffering', trackId: 'x' });
    });
  });
});
