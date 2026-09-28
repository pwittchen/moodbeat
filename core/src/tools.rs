//! Locating, installing and updating `yt-dlp` and `ffmpeg` (SPEC §6.1).

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use futures_util::StreamExt;
use serde::Serialize;
use sha2::{Digest, Sha256};

use crate::error::{AppError, AppResult};
use crate::paths::{set_mode, Paths};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Tool {
    YtDlp,
    Ffmpeg,
}

impl Tool {
    pub fn name(self) -> &'static str {
        match self {
            Tool::YtDlp => "yt-dlp",
            Tool::Ffmpeg => "ffmpeg",
        }
    }

    fn exe_name(self) -> String {
        if cfg!(windows) { format!("{}.exe", self.name()) } else { self.name().to_string() }
    }

    fn version_arg(self) -> &'static str {
        match self {
            Tool::YtDlp => "--version",
            Tool::Ffmpeg => "-version",
        }
    }
}

const YT_DLP_BASE: &str = "https://github.com/yt-dlp/yt-dlp/releases/latest/download";
const FFMPEG_BASE: &str = "https://github.com/eugeneware/ffmpeg-static/releases/download/b6.0";

/// Release asset name of the standalone yt-dlp binary for this platform.
fn yt_dlp_asset() -> Option<&'static str> {
    if cfg!(target_os = "macos") {
        Some("yt-dlp_macos")
    } else if cfg!(all(target_os = "linux", target_arch = "x86_64")) {
        Some("yt-dlp_linux")
    } else if cfg!(all(target_os = "linux", target_arch = "aarch64")) {
        Some("yt-dlp_linux_aarch64")
    } else if cfg!(windows) {
        Some("yt-dlp.exe")
    } else {
        None
    }
}

/// Static ffmpeg build (gzip) and its pinned SHA-256.
fn ffmpeg_asset() -> Option<(&'static str, &'static str)> {
    if cfg!(all(target_os = "macos", target_arch = "aarch64")) {
        Some(("ffmpeg-darwin-arm64.gz", "6be74d6f449889c2e87a75873894f8520cad56c08ac76f2a628d85b0519daaca"))
    } else if cfg!(all(target_os = "macos", target_arch = "x86_64")) {
        Some(("ffmpeg-darwin-x64.gz", "a12354fce7eb62361473bbe10d53a1893695babd35869ec8e92e5dfea8d0440b"))
    } else if cfg!(all(target_os = "linux", target_arch = "x86_64")) {
        Some(("ffmpeg-linux-x64.gz", "17c1ae10b52ac499180679fe6ba77e17642390c4eedb0f1e3b0ac045da55128f"))
    } else if cfg!(all(target_os = "linux", target_arch = "aarch64")) {
        Some(("ffmpeg-linux-arm64.gz", "2b708b2d15041d2a192c1db24c7a8a1d24f645a8242dce1c744ff2392b86ada1"))
    } else if cfg!(all(windows, target_arch = "x86_64")) {
        Some(("ffmpeg-win32-x64.gz", "450d66226c79405c724e821f291cab0911e934bfa9fa2231adcab587f3e07b50"))
    } else {
        None
    }
}

/// A command for an external binary: argument array only, no shell, no console window.
pub fn command(program: &Path) -> tokio::process::Command {
    #[allow(unused_mut)]
    let mut cmd = tokio::process::Command::new(program);
    #[cfg(windows)]
    {
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    cmd
}

pub fn managed_path(paths: &Paths, tool: Tool) -> PathBuf {
    paths.bin.join(tool.exe_name())
}

/// `~/.moodbeat/bin/<tool>` first, then the system `PATH`. GUI apps on macOS get a minimal
/// `PATH`, so the usual Homebrew/MacPorts locations are checked as well.
pub fn locate(paths: &Paths, tool: Tool) -> Option<PathBuf> {
    let managed = managed_path(paths, tool);
    if managed.is_file() {
        return Some(managed);
    }
    let mut dirs: Vec<PathBuf> = std::env::var_os("PATH")
        .map(|p| std::env::split_paths(&p).collect())
        .unwrap_or_default();
    if cfg!(unix) {
        dirs.extend(["/opt/homebrew/bin", "/usr/local/bin", "/opt/local/bin", "/usr/bin"].map(PathBuf::from));
    }
    dirs.into_iter().map(|d| d.join(tool.exe_name())).find(|p| p.is_file())
}

pub async fn version(path: &Path, tool: Tool) -> Option<String> {
    let out = tokio::time::timeout(
        Duration::from_secs(15),
        command(path).arg(tool.version_arg()).stdin(Stdio::null()).output(),
    )
    .await
    .ok()?
    .ok()?;
    if !out.status.success() {
        return None;
    }
    parse_version(tool, &String::from_utf8_lossy(&out.stdout))
}

pub fn parse_version(tool: Tool, stdout: &str) -> Option<String> {
    let first = stdout.lines().next()?.trim();
    match tool {
        Tool::YtDlp => Some(first.to_string()).filter(|s| !s.is_empty()),
        // "ffmpeg version 6.0-static https://johnvansickle.com/ffmpeg/  Copyright ..."
        Tool::Ffmpeg => first.strip_prefix("ffmpeg version ")?.split_whitespace().next().map(str::to_string),
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolInfo {
    /// Version string, or `None` when the tool is missing.
    pub version: Option<String>,
    pub path: Option<String>,
    pub managed: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolsStatus {
    pub yt_dlp: ToolInfo,
    pub ffmpeg: ToolInfo,
}

impl ToolsStatus {
    pub fn all_present(&self) -> bool {
        self.yt_dlp.version.is_some() && self.ffmpeg.version.is_some()
    }
}

async fn info(paths: &Paths, tool: Tool) -> ToolInfo {
    match locate(paths, tool) {
        Some(p) => ToolInfo {
            version: version(&p, tool).await,
            managed: p == managed_path(paths, tool),
            path: Some(p.display().to_string()),
        },
        None => ToolInfo { version: None, path: None, managed: false },
    }
}

pub async fn status(paths: &Paths) -> ToolsStatus {
    ToolsStatus { yt_dlp: info(paths, Tool::YtDlp).await, ffmpeg: info(paths, Tool::Ffmpeg).await }
}

/// Installs missing tools into `~/.moodbeat/bin/` and updates a managed yt-dlp.
pub async fn install(
    paths: &Paths,
    http: &reqwest::Client,
    progress: &(dyn Fn(Tool, f32, &str) + Send + Sync),
) -> AppResult<()> {
    let current = status(paths).await;

    if current.yt_dlp.version.is_none() {
        let asset = yt_dlp_asset().ok_or_else(|| AppError::Tools("No yt-dlp build for this platform".into()))?;
        let dest = managed_path(paths, Tool::YtDlp);
        let tmp = dest.with_extension("part");
        progress(Tool::YtDlp, 0.0, "Downloading yt-dlp…");
        download(http, &format!("{YT_DLP_BASE}/{asset}"), &tmp, None, |p| {
            progress(Tool::YtDlp, p, "Downloading yt-dlp…")
        })
        .await
        .map_err(|e| AppError::Tools(format!("Couldn't download yt-dlp: {e}")))?;
        finalize_binary(&tmp, &dest).map_err(AppError::storage)?;
        progress(Tool::YtDlp, 100.0, "yt-dlp installed");
    } else if current.yt_dlp.managed {
        progress(Tool::YtDlp, 0.0, "Updating yt-dlp…");
        let msg = match update_yt_dlp(&managed_path(paths, Tool::YtDlp)).await {
            Ok(_) => "yt-dlp is up to date".to_string(),
            Err(e) => format!("yt-dlp update failed: {e}"),
        };
        progress(Tool::YtDlp, 100.0, &msg);
    }

    if current.ffmpeg.version.is_none() {
        let (asset, sha) = ffmpeg_asset().ok_or_else(|| AppError::Tools("No ffmpeg build for this platform".into()))?;
        let dest = managed_path(paths, Tool::Ffmpeg);
        let gz = paths.bin.join("ffmpeg.gz.part");
        progress(Tool::Ffmpeg, 0.0, "Downloading ffmpeg…");
        let result = download(http, &format!("{FFMPEG_BASE}/{asset}"), &gz, Some(sha), |p| {
            progress(Tool::Ffmpeg, p * 0.95, "Downloading ffmpeg…")
        })
        .await;
        if let Err(e) = result {
            let _ = std::fs::remove_file(&gz);
            return Err(AppError::Tools(format!("Couldn't download ffmpeg: {e}")));
        }
        progress(Tool::Ffmpeg, 96.0, "Unpacking ffmpeg…");
        let tmp = dest.with_extension("part");
        let unpacked = {
            let (gz, tmp) = (gz.clone(), tmp.clone());
            tokio::task::spawn_blocking(move || gunzip(&gz, &tmp)).await.expect("gunzip task")
        };
        let _ = std::fs::remove_file(&gz);
        unpacked.map_err(AppError::storage)?;
        finalize_binary(&tmp, &dest).map_err(AppError::storage)?;
        progress(Tool::Ffmpeg, 100.0, "ffmpeg installed");
    }

    let after = status(paths).await;
    if after.all_present() {
        Ok(())
    } else {
        Err(AppError::Tools("Installed tools don't run on this system".into()))
    }
}

/// `yt-dlp -U` for the managed copy.
pub async fn update_yt_dlp(path: &Path) -> Result<String, String> {
    let out = tokio::time::timeout(
        Duration::from_secs(120),
        command(path).arg("-U").stdin(Stdio::null()).output(),
    )
    .await
    .map_err(|_| "timed out".to_string())?
    .map_err(|e| e.to_string())?;
    let stdout = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if out.status.success() {
        Ok(stdout)
    } else {
        Err(String::from_utf8_lossy(&out.stderr).lines().last().unwrap_or("unknown error").to_string())
    }
}

/// Streams `url` into `dest`, reporting 0–100 progress and verifying an optional SHA-256.
async fn download(
    http: &reqwest::Client,
    url: &str,
    dest: &Path,
    sha256: Option<&str>,
    on_progress: impl Fn(f32),
) -> Result<(), String> {
    let resp = http
        .get(url)
        .timeout(Duration::from_secs(600))
        .send()
        .await
        .and_then(|r| r.error_for_status())
        .map_err(|e| e.to_string())?;
    let total = resp.content_length();
    let mut file = std::fs::File::create(dest).map_err(|e| e.to_string())?;
    let mut hasher = Sha256::new();
    let mut received: u64 = 0;
    let mut last_reported = -1.0f32;
    let mut stream = resp.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|e| e.to_string())?;
        file.write_all(&chunk).map_err(|e| e.to_string())?;
        hasher.update(&chunk);
        received += chunk.len() as u64;
        if let Some(total) = total.filter(|t| *t > 0) {
            let pct = (received as f32 / total as f32 * 100.0).floor();
            if pct > last_reported {
                last_reported = pct;
                on_progress(pct);
            }
        }
    }
    file.sync_all().map_err(|e| e.to_string())?;
    if let Some(expected) = sha256 {
        let actual = format!("{:x}", hasher.finalize());
        if !actual.eq_ignore_ascii_case(expected) {
            let _ = std::fs::remove_file(dest);
            return Err(format!("checksum mismatch (expected {expected}, got {actual})"));
        }
    }
    Ok(())
}

fn gunzip(src: &Path, dest: &Path) -> std::io::Result<()> {
    let mut decoder = flate2::read::GzDecoder::new(std::fs::File::open(src)?);
    let mut out = std::fs::File::create(dest)?;
    std::io::copy(&mut decoder, &mut out)?;
    out.sync_all()
}

fn finalize_binary(tmp: &Path, dest: &Path) -> std::io::Result<()> {
    set_mode(tmp, 0o755)?;
    std::fs::rename(tmp, dest)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_versions() {
        assert_eq!(parse_version(Tool::YtDlp, "2026.08.19\n").as_deref(), Some("2026.08.19"));
        assert_eq!(
            parse_version(Tool::Ffmpeg, "ffmpeg version 6.0-static https://x Copyright (c) 2000-2023\n").as_deref(),
            Some("6.0-static")
        );
        assert_eq!(parse_version(Tool::Ffmpeg, "garbage"), None);
    }

    #[test]
    fn managed_copy_wins() {
        let dir = tempfile::tempdir().unwrap();
        let paths = Paths::new(dir.path().to_path_buf());
        paths.ensure().unwrap();
        let managed = managed_path(&paths, Tool::YtDlp);
        std::fs::write(&managed, b"").unwrap();
        assert_eq!(locate(&paths, Tool::YtDlp), Some(managed));
    }

    #[test]
    fn this_platform_has_assets() {
        assert!(yt_dlp_asset().is_some());
        assert!(ffmpeg_asset().is_some());
    }
}
