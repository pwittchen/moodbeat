mod about;
mod cache;
mod commands;
mod config;
mod downloader;
mod error;
mod llm;
mod paths;
mod playlist;
mod resolver;
mod tools;

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use chrono::Utc;
use tauri::Manager;
use tracing_appender::rolling::{Builder, Rotation};

use commands::AppState;
use config::Config;
use paths::Paths;
use tools::Tool;

const YT_DLP_UPDATE_INTERVAL_HOURS: i64 = 24;

/// Starts the app.
///
/// # Panics
///
/// If the home directory can't be determined or the Tauri runtime fails to start.
pub fn run() {
    let paths = Paths::from_home().expect("home directory");
    if let Err(e) = paths.ensure() {
        eprintln!("Can't write to ~/.moodbeat: {e}");
    }
    let _log_guard = init_logging(&paths);

    let config = Config::load(&paths.config).unwrap_or_else(|e| {
        tracing::error!("config unreadable, using defaults: {e}");
        Config::default()
    });
    tracing::info!("starting moodbeat {} with {config:?}", env!("CARGO_PKG_VERSION"));

    let state = AppState {
        cache: Arc::new(cache::Cache::load(&paths)),
        paths,
        config: Mutex::new(config),
        http: llm::http_client(),
        current: Mutex::new(None),
        generating: tokio::sync::Mutex::new(()),
        recent_suggestions: Mutex::new(VecDeque::new()),
    };

    tauri::Builder::default()
        .manage(state)
        .menu(about::app_menu)
        .on_menu_event(about::on_menu_event)
        .setup(|app| {
            let handle = app.handle().clone();
            tauri::async_runtime::spawn(async move { update_yt_dlp_if_due(handle).await });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::get_settings,
            commands::save_api_key,
            commands::remove_api_key,
            commands::set_model,
            commands::set_volume,
            commands::generate_playlist,
            commands::suggest_moods,
            commands::cancel_playlist,
            commands::retry_track,
            commands::prioritize_track,
            commands::track_played,
            commands::report_playback_error,
            commands::get_tools_status,
            commands::install_tools,
            commands::clear_cache,
        ])
        .run(tauri::generate_context!())
        .expect("error while running moodbeat");
}

/// Daily rolling log in `~/.moodbeat/logs/`, last 7 files kept.
fn init_logging(paths: &Paths) -> Option<tracing_appender::non_blocking::WorkerGuard> {
    let appender = Builder::new()
        .rotation(Rotation::DAILY)
        .filename_prefix("moodbeat")
        .filename_suffix("log")
        .max_log_files(7)
        .build(&paths.logs)
        .ok()?;
    let (writer, guard) = tracing_appender::non_blocking(appender);
    let filter = tracing_subscriber::EnvFilter::try_from_env("MOODBEAT_LOG")
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("moodbeat_lib=info,warn"));
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(writer)
        .with_ansi(false)
        .init();
    Some(guard)
}

/// Stale yt-dlp is the most common cause of download failures: run `yt-dlp -U` on the
/// managed copy at most once every 24 h.
async fn update_yt_dlp_if_due(app: tauri::AppHandle) {
    let state = app.state::<AppState>();
    let managed = tools::managed_path(&state.paths, Tool::YtDlp);
    if !managed.is_file() {
        return;
    }
    let last = state.config.lock().unwrap().tools.last_yt_dlp_update_check;
    if last.is_some_and(|t| Utc::now() - t < chrono::Duration::hours(YT_DLP_UPDATE_INTERVAL_HOURS)) {
        return;
    }
    match tools::update_yt_dlp(&managed).await {
        Ok(out) => tracing::info!("yt-dlp update check: {out}"),
        Err(e) => tracing::warn!("yt-dlp update failed: {e}"),
    }
    let mut cfg = state.config.lock().unwrap();
    cfg.tools.last_yt_dlp_update_check = Some(Utc::now());
    if let Err(e) = cfg.save(&state.paths.config) {
        tracing::error!("failed to save config: {e}");
    }
}
