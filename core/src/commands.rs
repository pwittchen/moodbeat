//! `#[tauri::command]` functions (SPEC §4.3).

// Tauri commands receive owned arguments (`String`, `State<'_, _>`) by design.
#![allow(clippy::needless_pass_by_value, reason = "Tauri command signatures")]

use std::collections::{HashSet, VecDeque};
use std::sync::{Arc, Mutex};

use serde::Serialize;
use tauri::{AppHandle, Emitter, State};

use crate::cache::Cache;
use crate::config::Config;
use crate::downloader::{EventSink, Job, ReadyEvent, StatusEvent};
use crate::error::{AppError, AppResult};
use crate::llm;
use crate::paths::Paths;
use crate::playlist::Playlist;
use crate::tools::{self, Tool, ToolsStatus};

pub struct AppState {
    pub paths: Paths,
    pub config: Mutex<Config>,
    pub cache: Arc<Cache>,
    pub http: reqwest::Client,
    pub current: Mutex<Option<Arc<Job>>>,
    /// Serializes `generate_playlist` calls.
    pub generating: tokio::sync::Mutex<()>,
    /// Moods suggested recently, so new suggestions don't repeat them.
    pub recent_suggestions: Mutex<VecDeque<String>>,
}

const RECENT_SUGGESTIONS: usize = 30;

impl AppState {
    fn config(&self) -> Config {
        self.config.lock().unwrap().clone()
    }

    fn update_config(&self, f: impl FnOnce(&mut Config)) -> AppResult<()> {
        let mut cfg = self.config.lock().unwrap();
        let mut next = cfg.clone();
        f(&mut next);
        next.save(&self.paths.config).map_err(AppError::storage)?;
        *cfg = next;
        Ok(())
    }

    fn job(&self, playlist_id: &str) -> AppResult<Arc<Job>> {
        self.current
            .lock()
            .unwrap()
            .clone()
            .filter(|j| j.id == playlist_id)
            .ok_or_else(|| AppError::Invalid("This playlist is no longer active".into()))
    }

    fn protected_video_ids(&self) -> HashSet<String> {
        self.current
            .lock()
            .unwrap()
            .as_ref()
            .map(|j| j.video_ids())
            .unwrap_or_default()
    }
}

struct TauriEvents(AppHandle);

impl EventSink for TauriEvents {
    fn status(&self, event: StatusEvent) {
        let _ = self.0.emit("track://status", event);
    }
    fn ready(&self, event: ReadyEvent) {
        let _ = self.0.emit("track://ready", event);
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Settings {
    has_api_key: bool,
    model: String,
    data_dir: String,
    cache_size_bytes: u64,
    cache_max_bytes: u64,
    volume: f64,
    muted: bool,
}

#[tauri::command]
pub fn get_settings(state: State<'_, AppState>) -> Settings {
    let cfg = state.config();
    Settings {
        has_api_key: cfg.effective_api_key().is_some(),
        model: cfg.openai.model.clone(),
        data_dir: state.paths.root.display().to_string(),
        cache_size_bytes: state.cache.total_bytes(),
        cache_max_bytes: cfg.cache_max_bytes(),
        volume: cfg.player.volume,
        muted: cfg.player.muted,
    }
}

#[tauri::command]
pub async fn save_api_key(state: State<'_, AppState>, api_key: String) -> AppResult<()> {
    let key = api_key.trim().to_string();
    if key.is_empty() {
        return Err(AppError::Invalid("Paste your OpenAI API key first.".into()));
    }
    llm::validate_key(&state.http, &key).await?;
    state.update_config(|c| c.openai.api_key = Some(key))?;
    tracing::info!("API key saved");
    Ok(())
}

#[tauri::command]
pub fn remove_api_key(state: State<'_, AppState>) -> AppResult<()> {
    state.update_config(|c| c.openai.api_key = None)?;
    tracing::info!("API key removed");
    Ok(())
}

#[tauri::command]
pub fn set_model(state: State<'_, AppState>, model: String) -> AppResult<()> {
    let model = model.trim();
    let model = if model.is_empty() { llm::DEFAULT_MODEL } else { model };
    state.update_config(|c| c.openai.model = model.to_string())
}

/// Persists the player volume (0.0–1.0) and mute state.
#[tauri::command]
pub fn set_volume(state: State<'_, AppState>, volume: f64, muted: bool) -> AppResult<()> {
    let volume = if volume.is_finite() {
        volume.clamp(0.0, 1.0)
    } else {
        1.0
    };
    state.update_config(|c| {
        c.player.volume = volume;
        c.player.muted = muted;
    })
}

#[tauri::command]
pub async fn generate_playlist(app: AppHandle, state: State<'_, AppState>, mood: String) -> AppResult<Playlist> {
    let _busy = state.generating.lock().await;
    let mood = llm::clean_mood(&mood);
    if mood.is_empty() {
        return Err(AppError::Invalid("Describe a mood first.".into()));
    }
    let cfg = state.config();
    let api_key = cfg.effective_api_key().ok_or(AppError::NoApiKey)?;
    let yt_dlp = tools::locate(&state.paths, Tool::YtDlp);
    let ffmpeg = tools::locate(&state.paths, Tool::Ffmpeg);
    let (Some(yt_dlp), Some(ffmpeg)) = (yt_dlp, ffmpeg) else {
        return Err(AppError::Tools(
            "yt-dlp and ffmpeg are required. Install them in Settings.".into(),
        ));
    };

    tracing::info!(
        "generating playlist for mood ({} chars) with {}",
        mood.chars().count(),
        cfg.openai.model
    );
    let result = llm::generate(&state.http, &api_key, &cfg.openai.model, &mood).await;
    let llm_playlist = result.inspect_err(|e| tracing::warn!("playlist generation failed: {e}"))?;
    let playlist = Playlist::new(&mood, &cfg.openai.model, llm_playlist);
    playlist.save(&state.paths).map_err(AppError::storage)?;

    // The new playlist replaces the old one: stop its downloads.
    let job = Job::start(
        playlist.clone(),
        yt_dlp,
        ffmpeg,
        state.paths.clone(),
        state.cache.clone(),
        cfg.cache_max_bytes(),
        Arc::new(TauriEvents(app)),
    );
    if let Some(old) = state.current.lock().unwrap().replace(job) {
        old.cancel();
    }
    tracing::info!("playlist {} with {} tracks", playlist.id, playlist.tracks.len());
    Ok(playlist)
}

/// Fresh mood ideas from the LLM, different every call (needs an API key).
#[tauri::command]
pub async fn suggest_moods(state: State<'_, AppState>) -> AppResult<Vec<String>> {
    let cfg = state.config();
    let api_key = cfg.effective_api_key().ok_or(AppError::NoApiKey)?;
    let avoid: Vec<String> = state.recent_suggestions.lock().unwrap().iter().cloned().collect();
    let seed = ulid::Ulid::new().random();
    let moods = llm::suggest_moods(&state.http, &api_key, &cfg.openai.model, seed, &avoid)
        .await
        .inspect_err(|e| tracing::warn!("mood suggestions failed: {e}"))?;
    let mut recent = state.recent_suggestions.lock().unwrap();
    for m in &moods {
        recent.push_back(m.clone());
    }
    while recent.len() > RECENT_SUGGESTIONS {
        recent.pop_front();
    }
    Ok(moods)
}

#[tauri::command]
pub fn cancel_playlist(state: State<'_, AppState>, playlist_id: String) {
    if let Ok(job) = state.job(&playlist_id) {
        job.cancel();
    }
}

#[tauri::command]
pub fn retry_track(state: State<'_, AppState>, playlist_id: String, track_id: String) -> AppResult<()> {
    state.job(&playlist_id)?.retry(&track_id)
}

/// Clicking a pending track moves it to the front of the download queue.
#[tauri::command]
pub fn prioritize_track(state: State<'_, AppState>, playlist_id: String, track_id: String) -> AppResult<()> {
    state.job(&playlist_id)?.prioritize(&track_id);
    Ok(())
}

/// Updates "last played" for cache eviction.
#[tauri::command]
pub fn track_played(state: State<'_, AppState>, playlist_id: String, track_id: String) -> AppResult<()> {
    if let Some(video_id) = state.job(&playlist_id)?.track(&track_id).and_then(|t| t.video_id) {
        state.cache.touch(&video_id);
    }
    Ok(())
}

/// The `<audio>` element failed on a ready file (SPEC §9).
#[tauri::command]
pub fn report_playback_error(state: State<'_, AppState>, playlist_id: String, track_id: String) -> AppResult<()> {
    tracing::warn!("playback error on {playlist_id}/{track_id}");
    state.job(&playlist_id)?.playback_failed(&track_id);
    Ok(())
}

#[tauri::command]
pub async fn get_tools_status(state: State<'_, AppState>) -> AppResult<ToolsStatus> {
    Ok(tools::status(&state.paths).await)
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct ToolsProgress {
    tool: Tool,
    progress: f32,
    message: String,
}

#[tauri::command]
pub async fn install_tools(app: AppHandle, state: State<'_, AppState>) -> AppResult<()> {
    let progress = move |tool: Tool, progress: f32, message: &str| {
        let _ = app.emit(
            "tools://progress",
            ToolsProgress {
                tool,
                progress,
                message: message.to_string(),
            },
        );
    };
    tools::install(&state.paths, &state.http, &progress)
        .await
        .inspect_err(|e| tracing::error!("tool install failed: {e}"))
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ClearCacheResult {
    freed_bytes: u64,
}

#[tauri::command]
pub fn clear_cache(state: State<'_, AppState>) -> ClearCacheResult {
    let freed_bytes = state.cache.clear(&state.protected_video_ids());
    tracing::info!("cache cleared, freed {freed_bytes} bytes");
    ClearCacheResult { freed_bytes }
}
