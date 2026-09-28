use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

/// Layout of `~/.moodbeat/` (SPEC §8).
#[derive(Debug, Clone)]
pub struct Paths {
    pub root: PathBuf,
    pub bin: PathBuf,
    pub cache: PathBuf,
    pub audio: PathBuf,
    pub index: PathBuf,
    pub playlists: PathBuf,
    pub logs: PathBuf,
    pub config: PathBuf,
}

impl Paths {
    pub fn from_home() -> io::Result<Self> {
        let home =
            dirs::home_dir().ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "home directory not found"))?;
        Ok(Self::new(home.join(".moodbeat")))
    }

    pub fn new(root: PathBuf) -> Self {
        let cache = root.join("cache");
        Self {
            bin: root.join("bin"),
            audio: cache.join("audio"),
            index: cache.join("index.json"),
            playlists: root.join("playlists"),
            logs: root.join("logs"),
            config: root.join("config.json"),
            cache,
            root,
        }
    }

    /// Creates the directory tree; the root is owner-only (`0700`) on unix.
    pub fn ensure(&self) -> io::Result<()> {
        for dir in [
            &self.root,
            &self.bin,
            &self.cache,
            &self.audio,
            &self.playlists,
            &self.logs,
        ] {
            fs::create_dir_all(dir)?;
        }
        set_mode(&self.root, 0o700)
    }

    pub fn audio_file(&self, video_id: &str) -> PathBuf {
        self.audio.join(format!("{video_id}.mp3"))
    }
}

#[cfg(unix)]
pub fn set_mode(path: &Path, mode: u32) -> io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(mode))
}

#[cfg(not(unix))]
pub fn set_mode(_path: &Path, _mode: u32) -> io::Result<()> {
    Ok(())
}

/// Atomic write: temp file in the same directory, optional unix mode, fsync, rename.
pub fn write_atomic(path: &Path, bytes: &[u8], mode: Option<u32>) -> io::Result<()> {
    let tmp = path.with_extension("tmp");
    {
        let mut file = fs::File::create(&tmp)?;
        if let Some(mode) = mode {
            set_mode(&tmp, mode)?;
        }
        file.write_all(bytes)?;
        file.sync_all()?;
    }
    fs::rename(&tmp, path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn creates_layout() {
        let dir = tempfile::tempdir().unwrap();
        let paths = Paths::new(dir.path().join(".moodbeat"));
        paths.ensure().unwrap();
        for p in [&paths.bin, &paths.audio, &paths.playlists, &paths.logs] {
            assert!(p.is_dir(), "{p:?} missing");
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = fs::metadata(&paths.root).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o700);
        }
    }
}
