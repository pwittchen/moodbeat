//! Per-playlist download queue: resolve → download → convert, max 3 concurrent jobs,
//! in playlist order, cancellable (SPEC §6.3, §6.4).

use std::collections::{HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde::Serialize;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, BufReader};
use tokio::sync::Notify;
use tokio_util::sync::CancellationToken;

use crate::cache::{Cache, SongEntry};
use crate::error::{AppError, AppResult};
use crate::paths::Paths;
use crate::playlist::{Playlist, Track, TrackStatus};
use crate::resolver::{self, SearchError};
use crate::tools;

const WORKERS: usize = 3;
const DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(5 * 60);
const PROGRESS_INTERVAL: Duration = Duration::from_millis(250);
const DOWNLOAD_ATTEMPTS: usize = 2;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StatusEvent {
    pub playlist_id: String,
    pub track_id: String,
    pub status: TrackStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub progress: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReadyEvent {
    pub playlist_id: String,
    pub track_id: String,
    pub file_path: String,
    pub duration_sec: Option<u32>,
}

/// Where job events go (the Tauri app handle in production).
pub trait EventSink: Send + Sync {
    fn status(&self, event: StatusEvent);
    fn ready(&self, event: ReadyEvent);
}

pub struct Job {
    pub id: String,
    playlist: Mutex<Playlist>,
    queue: Mutex<VecDeque<String>>,
    notify: Notify,
    cancel: CancellationToken,
    yt_dlp: PathBuf,
    ffmpeg: PathBuf,
    paths: Paths,
    cache: Arc<Cache>,
    cache_max_bytes: u64,
    events: Arc<dyn EventSink>,
}

impl Job {
    /// Queues every track in playlist order and starts the workers.
    pub fn start(
        playlist: Playlist,
        yt_dlp: PathBuf,
        ffmpeg: PathBuf,
        paths: Paths,
        cache: Arc<Cache>,
        cache_max_bytes: u64,
        events: Arc<dyn EventSink>,
    ) -> Arc<Self> {
        let queue = playlist.tracks.iter().map(|t| t.id.clone()).collect();
        let job = Arc::new(Self {
            id: playlist.id.clone(),
            playlist: Mutex::new(playlist),
            queue: Mutex::new(queue),
            notify: Notify::new(),
            cancel: CancellationToken::new(),
            yt_dlp,
            ffmpeg,
            paths,
            cache,
            cache_max_bytes,
            events,
        });
        for _ in 0..WORKERS {
            let job = job.clone();
            tauri::async_runtime::spawn(async move { job.worker().await });
        }
        job
    }

    /// Stops all workers; running yt-dlp processes are killed and partial files removed.
    pub fn cancel(&self) {
        self.cancel.cancel();
        self.notify.notify_waiters();
    }

    pub fn is_cancelled(&self) -> bool {
        self.cancel.is_cancelled()
    }

    pub fn snapshot(&self) -> Playlist {
        self.playlist.lock().unwrap().clone()
    }

    pub fn track(&self, track_id: &str) -> Option<Track> {
        self.playlist.lock().unwrap().track(track_id).cloned()
    }

    pub fn video_ids(&self) -> HashSet<String> {
        self.playlist.lock().unwrap().tracks.iter().filter_map(|t| t.video_id.clone()).collect()
    }

    /// Moves a pending track to the front of the queue.
    pub fn prioritize(&self, track_id: &str) {
        let mut queue = self.queue.lock().unwrap();
        if let Some(pos) = queue.iter().position(|t| t == track_id) {
            let id = queue.remove(pos).expect("position is valid");
            queue.push_front(id);
        }
    }

    /// Re-runs resolve + download for a failed track, ahead of the rest of the queue.
    pub fn retry(&self, track_id: &str) -> AppResult<()> {
        if self.is_cancelled() {
            return Err(AppError::Invalid("This playlist is no longer active".into()));
        }
        let track = self.track(track_id).ok_or_else(|| AppError::Invalid("Unknown track".into()))?;
        if track.status != TrackStatus::Failed {
            return Ok(());
        }
        if track.error.as_deref() == Some(NOT_FOUND) || track.error.as_deref() == Some(PLAYBACK_ERROR) {
            // The previous match may have been wrong: search again.
            self.cache.forget_song(&track.artist, &track.title);
        }
        self.update(track_id, |t| {
            t.status = TrackStatus::Pending;
            t.error = None;
            t.video_id = None;
        });
        self.emit_status(track_id, TrackStatus::Pending, None, None);
        self.queue.lock().unwrap().push_front(track_id.to_string());
        self.notify.notify_one();
        Ok(())
    }

    /// The WebView couldn't play a ready file: drop it from the cache and mark the track failed.
    pub fn playback_failed(&self, track_id: &str) {
        if let Some(video_id) = self.track(track_id).and_then(|t| t.video_id) {
            self.cache.remove_file(&video_id);
        }
        self.fail(track_id, PLAYBACK_ERROR);
    }

    async fn worker(self: Arc<Self>) {
        loop {
            if self.is_cancelled() {
                return;
            }
            let next = self.queue.lock().unwrap().pop_front();
            match next {
                Some(track_id) => self.process(&track_id).await,
                None => tokio::select! {
                    _ = self.cancel.cancelled() => return,
                    _ = self.notify.notified() => {}
                },
            }
        }
    }

    async fn process(&self, track_id: &str) {
        let Some(track) = self.track(track_id) else { return };
        if track.status != TrackStatus::Pending {
            return;
        }
        self.set_status(track_id, TrackStatus::Searching, None);

        let entry = match self.cache.lookup_song(&track.artist, &track.title) {
            Some(entry) => entry,
            None => match resolver::search(&self.yt_dlp, &track.artist, &track.title, &self.cancel).await {
                Err(SearchError::Cancelled) => return,
                Err(e) => {
                    tracing::warn!("search for {} - {} failed: {e}", track.artist, track.title);
                    return self.fail(track_id, "Couldn't search YouTube");
                }
                Ok(results) => match resolver::pick_best(&track.artist, &track.title, &results) {
                    None => return self.fail(track_id, NOT_FOUND),
                    Some(best) if !is_valid_video_id(&best.id) => return self.fail(track_id, NOT_FOUND),
                    Some(best) => {
                        let entry = SongEntry {
                            video_id: best.id,
                            duration_sec: best.duration.map(|d| d.round() as u32),
                        };
                        self.cache.remember_song(&track.artist, &track.title, entry.clone());
                        entry
                    }
                },
            },
        };
        let video_id = entry.video_id.clone();
        self.update(track_id, |t| {
            t.video_id = Some(entry.video_id.clone());
            t.duration_sec = entry.duration_sec;
        });

        // Serialize work on the same file across playlists and duplicate songs.
        let lock = self.cache.video_lock(&video_id);
        let _guard = tokio::select! {
            _ = self.cancel.cancelled() => return,
            guard = lock.lock_owned() => guard,
        };

        if self.cache.has_audio(&video_id) {
            return self.ready(track_id, &video_id, entry.duration_sec);
        }

        self.set_status(track_id, TrackStatus::Downloading, Some(0.0));
        let mut last_error = String::new();
        for attempt in 1..=DOWNLOAD_ATTEMPTS {
            let mut last_emit = Instant::now() - PROGRESS_INTERVAL;
            let result = download(&self.yt_dlp, &self.ffmpeg, &self.paths.audio, &video_id, &self.cancel, |p| {
                if last_emit.elapsed() >= PROGRESS_INTERVAL {
                    last_emit = Instant::now();
                    self.emit_status(track_id, TrackStatus::Downloading, Some(p), None);
                }
            })
            .await;
            match result {
                Ok(()) => {
                    self.cache.record_file(&video_id);
                    self.ready(track_id, &video_id, entry.duration_sec);
                    self.cache.evict(self.cache_max_bytes, &self.video_ids());
                    return;
                }
                Err(DownloadError::Cancelled) => return,
                Err(e) => {
                    tracing::warn!("download of {video_id} failed (attempt {attempt}): {e}");
                    last_error = e.to_string();
                }
            }
        }
        self.fail(track_id, &format!("Download failed: {last_error}"));
    }

    fn update(&self, track_id: &str, f: impl FnOnce(&mut Track)) {
        if let Some(t) = self.playlist.lock().unwrap().track_mut(track_id) {
            f(t);
        }
    }

    fn save(&self) {
        let playlist = self.snapshot();
        if let Err(e) = playlist.save(&self.paths) {
            tracing::error!("failed to save playlist {}: {e}", playlist.id);
        }
    }

    fn emit_status(&self, track_id: &str, status: TrackStatus, progress: Option<f32>, error: Option<String>) {
        self.events.status(StatusEvent {
            playlist_id: self.id.clone(),
            track_id: track_id.to_string(),
            status,
            progress,
            error,
        });
    }

    fn set_status(&self, track_id: &str, status: TrackStatus, progress: Option<f32>) {
        self.update(track_id, |t| t.status = status);
        self.emit_status(track_id, status, progress, None);
    }

    fn fail(&self, track_id: &str, error: &str) {
        if self.is_cancelled() {
            return;
        }
        self.update(track_id, |t| {
            t.status = TrackStatus::Failed;
            t.error = Some(error.to_string());
        });
        self.save();
        self.emit_status(track_id, TrackStatus::Failed, None, Some(error.to_string()));
    }

    fn ready(&self, track_id: &str, video_id: &str, duration_sec: Option<u32>) {
        self.update(track_id, |t| {
            t.status = TrackStatus::Ready;
            t.error = None;
        });
        self.save();
        self.emit_status(track_id, TrackStatus::Ready, None, None);
        self.events.ready(ReadyEvent {
            playlist_id: self.id.clone(),
            track_id: track_id.to_string(),
            file_path: self.paths.audio_file(video_id).display().to_string(),
            duration_sec,
        });
    }
}

pub const NOT_FOUND: &str = "Not found on YouTube";
pub const PLAYBACK_ERROR: &str = "Couldn't play the downloaded file";

/// YouTube ids are `[A-Za-z0-9_-]`; anything else must never become part of a path.
pub fn is_valid_video_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= 32 && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

#[derive(Debug, thiserror::Error)]
pub enum DownloadError {
    #[error("cancelled")]
    Cancelled,
    #[error("timed out")]
    TimedOut,
    #[error("{0}")]
    Failed(String),
}

/// `[download]  42.3% of ...` → `42.3`
pub fn parse_progress(line: &str) -> Option<f32> {
    let rest = line.trim_start().strip_prefix("[download]")?.trim_start();
    let pct = rest.split_whitespace().next()?.strip_suffix('%')?;
    pct.parse::<f32>().ok().filter(|p| (0.0..=100.0).contains(p))
}

/// Downloads `video_id` as `<audio_dir>/<video_id>.mp3`. Partial files are removed on failure.
pub async fn download(
    yt_dlp: &Path,
    ffmpeg: &Path,
    audio_dir: &Path,
    video_id: &str,
    cancel: &CancellationToken,
    mut on_progress: impl FnMut(f32),
) -> Result<(), DownloadError> {
    let template = format!("{}/{video_id}.%(ext)s", audio_dir.display().to_string().replace('%', "%%"));
    let mut cmd = tools::command(yt_dlp);
    cmd.args(["-f", "bestaudio", "-x", "--audio-format", "mp3", "--audio-quality", "0"])
        .arg("--ffmpeg-location")
        .arg(ffmpeg)
        .args(["--no-playlist", "--newline", "--embed-metadata", "-o"])
        .arg(&template)
        .arg(format!("https://www.youtube.com/watch?v={video_id}"))
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    let mut child = cmd.spawn().map_err(|e| DownloadError::Failed(e.to_string()))?;

    let stdout = child.stdout.take().expect("stdout is piped");
    let mut stderr = child.stderr.take().expect("stderr is piped");
    let stderr_task = tokio::spawn(async move {
        let mut buf = String::new();
        let _ = stderr.read_to_string(&mut buf).await;
        buf
    });

    enum Outcome {
        Cancelled,
        TimedOut,
        Exited(std::io::Result<std::process::ExitStatus>),
    }
    let outcome = {
        let run = async {
            let mut lines = BufReader::new(stdout).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                if let Some(p) = parse_progress(&line) {
                    on_progress(p);
                }
            }
            child.wait().await
        };
        tokio::select! {
            _ = cancel.cancelled() => Outcome::Cancelled,
            r = tokio::time::timeout(DOWNLOAD_TIMEOUT, run) => match r {
                Err(_) => Outcome::TimedOut,
                Ok(status) => Outcome::Exited(status),
            },
        }
    };

    let final_file = audio_dir.join(format!("{video_id}.mp3"));
    let result = match outcome {
        Outcome::Cancelled => Err(DownloadError::Cancelled),
        Outcome::TimedOut => Err(DownloadError::TimedOut),
        Outcome::Exited(Err(e)) => Err(DownloadError::Failed(e.to_string())),
        Outcome::Exited(Ok(status)) if status.success() && final_file.is_file() => Ok(()),
        Outcome::Exited(Ok(status)) => {
            let stderr = stderr_task.await.unwrap_or_default();
            let reason = stderr
                .lines()
                .rev()
                .find(|l| l.contains("ERROR"))
                .or_else(|| stderr.lines().rev().find(|l| !l.trim().is_empty()))
                .map(|l| l.trim().trim_start_matches("ERROR:").trim().to_string())
                .unwrap_or_else(|| format!("yt-dlp exited with {status}"));
            Err(DownloadError::Failed(reason))
        }
    };
    if result.is_err() {
        let _ = child.kill().await;
        remove_partials(audio_dir, video_id);
    }
    result
}

fn remove_partials(audio_dir: &Path, video_id: &str) {
    let prefix = format!("{video_id}.");
    if let Ok(entries) = std::fs::read_dir(audio_dir) {
        for e in entries.flatten() {
            if e.file_name().to_string_lossy().starts_with(&prefix) {
                let _ = std::fs::remove_file(e.path());
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_progress_lines() {
        assert_eq!(parse_progress("[download]  42.3% of    3.61MiB at    1.20MiB/s ETA 00:01"), Some(42.3));
        assert_eq!(parse_progress("[download] 100% of    3.61MiB in 00:00:02 at 1.5MiB/s"), Some(100.0));
        assert_eq!(parse_progress("[download]   0.0% of ~  3.61MiB at  Unknown B/s ETA Unknown"), Some(0.0));
        assert_eq!(parse_progress("[download] Destination: /x/abc.webm"), None);
        assert_eq!(parse_progress("[ExtractAudio] Destination: /x/abc.mp3"), None);
        assert_eq!(parse_progress("[youtube] abc: Downloading webpage"), None);
    }

    #[test]
    fn validates_video_ids() {
        assert!(is_valid_video_id("d27gTrPPAyk"));
        assert!(is_valid_video_id("rp-Oiu3TI_0"));
        assert!(!is_valid_video_id("../etc/passwd"));
        assert!(!is_valid_video_id(""));
    }

    #[test]
    fn removes_only_matching_partials() {
        let dir = tempfile::tempdir().unwrap();
        for name in ["abc.webm.part", "abc.mp3", "abcd.mp3", "other.mp3"] {
            std::fs::write(dir.path().join(name), b"x").unwrap();
        }
        remove_partials(dir.path(), "abc");
        let mut left: Vec<_> = std::fs::read_dir(dir.path())
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().to_string())
            .collect();
        left.sort();
        assert_eq!(left, vec!["abcd.mp3", "other.mp3"]);
    }
}

/// Real network tests (SPEC §11.1): `MOODBEAT_INTEGRATION=1 cargo test -- --ignored`
#[cfg(test)]
mod integration {
    use super::*;
    use crate::tools::{locate, Tool};

    #[tokio::test]
    #[ignore]
    async fn resolves_and_downloads_one_track() {
        if std::env::var("MOODBEAT_INTEGRATION").is_err() {
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let paths = Paths::new(dir.path().to_path_buf());
        paths.ensure().unwrap();
        let yt_dlp = locate(&paths, Tool::YtDlp).expect("yt-dlp installed");
        let ffmpeg = locate(&paths, Tool::Ffmpeg).expect("ffmpeg installed");
        let cancel = CancellationToken::new();

        let results = resolver::search(&yt_dlp, "Ramones", "Blitzkrieg Bop", &cancel).await.unwrap();
        let best = resolver::pick_best("Ramones", "Blitzkrieg Bop", &results).expect("a match");

        let mut updates = vec![];
        download(&yt_dlp, &ffmpeg, &paths.audio, &best.id, &cancel, |p| updates.push(p)).await.unwrap();
        assert!(paths.audio_file(&best.id).metadata().unwrap().len() > 100_000);
        assert!(!updates.is_empty(), "no progress parsed");
        let leftovers = std::fs::read_dir(&paths.audio).unwrap().count();
        assert_eq!(leftovers, 1, "only the final mp3 remains");
    }
}
