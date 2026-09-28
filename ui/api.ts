// Typed wrappers for Tauri commands and events (SPEC §4.3, §4.4).

import { convertFileSrc, invoke } from '@tauri-apps/api/core';
import { listen, type UnlistenFn } from '@tauri-apps/api/event';
import type { TrackStatus } from './player';

export interface Settings {
  hasApiKey: boolean;
  model: string;
  dataDir: string;
  cacheSizeBytes: number;
  cacheMaxBytes: number;
  volume: number;
  muted: boolean;
}

export interface Track {
  id: string;
  artist: string;
  title: string;
  year: number | null;
  videoId: string | null;
  status: TrackStatus;
  durationSec: number | null;
  error: string | null;
}

export interface Playlist {
  id: string;
  createdAt: string;
  mood: string;
  title: string;
  model: string;
  tracks: Track[];
}

export interface ToolInfo {
  version: string | null;
  path: string | null;
  managed: boolean;
}

export interface ToolsStatus {
  ytDlp: ToolInfo;
  ffmpeg: ToolInfo;
}

export interface TrackStatusEvent {
  playlistId: string;
  trackId: string;
  status: TrackStatus;
  progress?: number;
  error?: string;
}

export interface TrackReadyEvent {
  playlistId: string;
  trackId: string;
  filePath: string;
  durationSec: number | null;
}

export interface ToolsProgressEvent {
  tool: 'ytDlp' | 'ffmpeg';
  progress: number;
  message: string;
}

export const api = {
  getSettings: () => invoke<Settings>('get_settings'),
  saveApiKey: (apiKey: string) => invoke<void>('save_api_key', { apiKey }),
  removeApiKey: () => invoke<void>('remove_api_key'),
  setModel: (model: string) => invoke<void>('set_model', { model }),
  setVolume: (volume: number, muted: boolean) => invoke<void>('set_volume', { volume, muted }),
  generatePlaylist: (mood: string) => invoke<Playlist>('generate_playlist', { mood }),
  suggestMoods: () => invoke<string[]>('suggest_moods'),
  cancelPlaylist: (playlistId: string) => invoke<void>('cancel_playlist', { playlistId }),
  retryTrack: (playlistId: string, trackId: string) => invoke<void>('retry_track', { playlistId, trackId }),
  prioritizeTrack: (playlistId: string, trackId: string) =>
    invoke<void>('prioritize_track', { playlistId, trackId }),
  trackPlayed: (playlistId: string, trackId: string) => invoke<void>('track_played', { playlistId, trackId }),
  reportPlaybackError: (playlistId: string, trackId: string) =>
    invoke<void>('report_playback_error', { playlistId, trackId }),
  getToolsStatus: () => invoke<ToolsStatus>('get_tools_status'),
  installTools: () => invoke<void>('install_tools'),
  clearCache: () => invoke<{ freedBytes: number }>('clear_cache'),
};

export const events = {
  onTrackStatus: (cb: (e: TrackStatusEvent) => void): Promise<UnlistenFn> =>
    listen<TrackStatusEvent>('track://status', (e) => cb(e.payload)),
  onTrackReady: (cb: (e: TrackReadyEvent) => void): Promise<UnlistenFn> =>
    listen<TrackReadyEvent>('track://ready', (e) => cb(e.payload)),
  onToolsProgress: (cb: (e: ToolsProgressEvent) => void): Promise<UnlistenFn> =>
    listen<ToolsProgressEvent>('tools://progress', (e) => cb(e.payload)),
};

export const fileSrc = (path: string): string => convertFileSrc(path);

/** Commands reject with the backend's user-facing message. */
export function errorMessage(e: unknown): string {
  if (typeof e === 'string') return e;
  if (e instanceof Error) return e.message;
  return 'Something went wrong.';
}
