const COMMANDS: &[&str] = &[
    "get_settings",
    "save_api_key",
    "remove_api_key",
    "set_model",
    "set_volume",
    "generate_playlist",
    "suggest_moods",
    "cancel_playlist",
    "retry_track",
    "prioritize_track",
    "track_played",
    "report_playback_error",
    "get_tools_status",
    "install_tools",
    "clear_cache",
];

fn main() {
    // Registering an app manifest makes every command opt-in: only the commands
    // granted in capabilities/default.json can be invoked by the frontend (SPEC §10).
    tauri_build::try_build(
        tauri_build::Attributes::new()
            .app_manifest(tauri_build::AppManifest::new().commands(COMMANDS)),
    )
    .expect("failed to run tauri-build");
}
