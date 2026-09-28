// Playback queue on top of an <audio> element (SPEC §3.4).
//
// "Playable" means status `ready`. Auto-advance waits (buffering) for a track that is still
// downloading and silently skips failed ones; when nothing playable is left the player
// finishes and resets its pointer to track #1.

export type TrackStatus = 'pending' | 'searching' | 'downloading' | 'ready' | 'failed';

export interface QueueTrack {
  id: string;
  status: TrackStatus;
  /** Playable URL, set once the track is ready. */
  src?: string;
}

/** The subset of HTMLAudioElement the player needs (faked in tests). */
export interface AudioLike {
  src: string;
  currentTime: number;
  readonly paused: boolean;
  readonly ended: boolean;
  play(): Promise<void>;
  pause(): void;
  removeAttribute(name: string): void;
  load(): void;
}

export type PlayerState = 'idle' | 'playing' | 'paused' | 'buffering' | 'finished';

export interface PlayerSnapshot {
  state: PlayerState;
  index: number;
  trackId: string | null;
}

/** Previous restarts the current track instead when past this position. */
export const RESTART_THRESHOLD_SEC = 3;

export class Player {
  private tracks: QueueTrack[] = [];
  private index = -1;
  private state: PlayerState = 'idle';
  private loadedId: string | null = null;

  constructor(
    private readonly audio: AudioLike,
    private readonly onChange: (s: PlayerSnapshot) => void = () => {},
    private readonly onTrackStart: (trackId: string) => void = () => {},
  ) {}

  snapshot(): PlayerSnapshot {
    return { state: this.state, index: this.index, trackId: this.tracks[this.index]?.id ?? null };
  }

  hasTracks(): boolean {
    return this.tracks.length > 0;
  }

  /** Replaces the queue. With `autoplay`, track #1 starts as soon as it is ready. */
  load(tracks: QueueTrack[], autoplay = true): void {
    this.unload();
    this.tracks = tracks.map((t) => ({ ...t }));
    this.index = -1;
    this.state = 'idle';
    if (autoplay && this.tracks.length > 0) {
      this.advanceFrom(-1);
    }
    this.emit();
  }

  clear(): void {
    this.load([], false);
  }

  updateTrack(id: string, patch: Partial<Omit<QueueTrack, 'id'>>): void {
    const i = this.tracks.findIndex((t) => t.id === id);
    if (i < 0) return;
    this.tracks[i] = { ...this.tracks[i], ...patch };
    if (this.state === 'buffering' && i === this.index) {
      const t = this.tracks[i];
      if (t.status === 'ready' && t.src) this.start();
      else if (t.status === 'failed') this.advanceFrom(i);
    }
    this.emit();
  }

  toggle(): void {
    switch (this.state) {
      case 'playing':
        this.state = 'paused';
        this.audio.pause();
        break;
      case 'paused':
        if (this.loadedId !== null && this.loadedId === this.tracks[this.index]?.id) {
          this.state = 'playing';
          void this.audio.play().catch(() => {});
        } else {
          this.goTo(this.index);
        }
        break;
      case 'buffering':
        // Stop waiting; Play resumes waiting on the same track.
        this.state = 'paused';
        break;
      case 'idle':
      case 'finished':
        if (this.tracks.length > 0) this.advanceFrom(-1);
        break;
    }
    this.emit();
  }

  /** Jumps to the next ready track; waits for the next pending one if none is ready yet. */
  next(): void {
    if (this.tracks.length === 0) return;
    const ready = this.findForward(this.index, (t) => t.status === 'ready');
    if (ready >= 0) this.goTo(ready);
    else this.advanceFrom(this.index);
    this.emit();
  }

  previous(): void {
    if (this.tracks.length === 0) return;
    if (this.isLoaded() && this.audio.currentTime > RESTART_THRESHOLD_SEC) {
      this.audio.currentTime = 0;
    } else {
      const ready = this.findBackward(this.index, (t) => t.status === 'ready');
      if (ready >= 0) this.goTo(ready);
      else if (this.isLoaded()) this.audio.currentTime = 0;
      else if (this.index >= 0) this.goTo(this.index);
      else this.advanceFrom(-1);
    }
    this.emit();
  }

  /** Row click: plays the track if it is ready. Returns false otherwise. */
  playTrack(id: string): boolean {
    const i = this.tracks.findIndex((t) => t.id === id);
    if (i < 0 || this.tracks[i].status !== 'ready') return false;
    this.goTo(i);
    this.emit();
    return true;
  }

  seek(seconds: number): void {
    if (this.isLoaded()) this.audio.currentTime = Math.max(0, seconds);
  }

  /** Wire to the audio `ended` event. */
  handleEnded(): void {
    this.advanceFrom(this.index);
    this.emit();
  }

  /** Wire to the audio `error` event. Returns the id of the track that failed, if any. */
  handleError(): string | null {
    const t = this.tracks[this.index];
    if (!t || this.loadedId !== t.id) return null;
    this.tracks[this.index] = { ...t, status: 'failed', src: undefined };
    this.unload();
    this.advanceFrom(this.index);
    this.emit();
    return t.id;
  }

  /** Wire to audio `pause`/`play` events so OS-initiated changes are reflected. */
  handleAudioPaused(): void {
    if (this.state === 'playing' && !this.audio.ended) {
      this.state = 'paused';
      this.emit();
    }
  }

  handleAudioPlaying(): void {
    if (this.state === 'paused' && this.isLoaded()) {
      this.state = 'playing';
      this.emit();
    }
  }

  private isLoaded(): boolean {
    return this.loadedId !== null && this.loadedId === this.tracks[this.index]?.id;
  }

  private findForward(from: number, pred: (t: QueueTrack) => boolean): number {
    for (let i = from + 1; i < this.tracks.length; i++) if (pred(this.tracks[i])) return i;
    return -1;
  }

  private findBackward(from: number, pred: (t: QueueTrack) => boolean): number {
    for (let i = from - 1; i >= 0; i--) if (pred(this.tracks[i])) return i;
    return -1;
  }

  /** Moves to the first non-failed track after `from`, or finishes. */
  private advanceFrom(from: number): void {
    const n = this.findForward(from, (t) => t.status !== 'failed');
    if (n < 0) this.finish();
    else this.goTo(n);
  }

  private goTo(i: number): void {
    this.index = i;
    const t = this.tracks[i];
    if (t.status === 'ready' && t.src) {
      this.start();
    } else if (t.status === 'failed') {
      this.advanceFrom(i);
    } else {
      this.unload();
      this.state = 'buffering';
    }
  }

  private start(): void {
    const t = this.tracks[this.index];
    if (this.loadedId === t.id) {
      this.audio.currentTime = 0;
    } else {
      this.audio.src = t.src!;
      this.loadedId = t.id;
    }
    this.state = 'playing';
    void this.audio.play().catch(() => {});
    this.onTrackStart(t.id);
  }

  private finish(): void {
    this.unload();
    this.index = this.tracks.length > 0 ? 0 : -1;
    this.state = 'finished';
  }

  private unload(): void {
    this.audio.pause();
    if (this.loadedId !== null) {
      this.audio.removeAttribute('src');
      this.audio.load();
      this.loadedId = null;
    }
  }

  private emit(): void {
    this.onChange(this.snapshot());
  }
}
