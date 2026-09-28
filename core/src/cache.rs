//! Audio cache: `cache/audio/<videoId>.mp3` + `cache/index.json` (SPEC §6.5).

use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs;
use std::sync::{Arc, Mutex};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::paths::{write_atomic, Paths};
use crate::resolver::normalize;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SongEntry {
    pub video_id: String,
    pub duration_sec: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FileEntry {
    pub size_bytes: u64,
    pub added_at: DateTime<Utc>,
    pub last_played: Option<DateTime<Utc>>,
}

impl FileEntry {
    fn last_used(&self) -> DateTime<Utc> {
        self.last_played.unwrap_or(self.added_at)
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CacheIndex {
    #[serde(default)]
    pub version: u32,
    /// normalized `artist|title` → resolved video
    #[serde(default)]
    pub songs: BTreeMap<String, SongEntry>,
    /// videoId → cached file metadata
    #[serde(default)]
    pub files: BTreeMap<String, FileEntry>,
}

pub fn song_key(artist: &str, title: &str) -> String {
    format!("{}|{}", normalize(artist), normalize(title))
}

/// Least recently used first, skipping protected ids, until the total fits `max_bytes`.
pub fn plan_eviction(files: &BTreeMap<String, FileEntry>, max_bytes: u64, protected: &HashSet<String>) -> Vec<String> {
    let mut total: u64 = files.values().map(|f| f.size_bytes).sum();
    if total <= max_bytes {
        return vec![];
    }
    let mut candidates: Vec<(&String, &FileEntry)> = files.iter().filter(|(id, _)| !protected.contains(*id)).collect();
    candidates.sort_by_key(|(_, f)| f.last_used());
    let mut evict = vec![];
    for (id, f) in candidates {
        if total <= max_bytes {
            break;
        }
        total -= f.size_bytes;
        evict.push(id.clone());
    }
    evict
}

pub struct Cache {
    paths: Paths,
    index: Mutex<CacheIndex>,
    /// One lock per video id so two jobs never write/delete the same file concurrently.
    locks: Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>,
}

impl Cache {
    /// Loads the index and reconciles it with the files actually on disk.
    pub fn load(paths: &Paths) -> Self {
        let mut index: CacheIndex = fs::read(&paths.index)
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default();
        index.version = 1;

        let mut on_disk = HashMap::new();
        if let Ok(entries) = fs::read_dir(&paths.audio) {
            for e in entries.flatten() {
                let path = e.path();
                if path.extension().is_some_and(|x| x == "mp3") {
                    if let (Some(stem), Ok(meta)) = (path.file_stem(), e.metadata()) {
                        on_disk.insert(stem.to_string_lossy().to_string(), meta.len());
                    }
                }
            }
        }
        index.files.retain(|id, _| on_disk.contains_key(id));
        for (id, size) in on_disk {
            let entry = index.files.entry(id).or_insert(FileEntry {
                size_bytes: size,
                added_at: Utc::now(),
                last_played: None,
            });
            entry.size_bytes = size;
        }
        let cache = Self {
            paths: paths.clone(),
            index: Mutex::new(index),
            locks: Mutex::new(HashMap::new()),
        };
        cache.persist();
        cache
    }

    fn persist(&self) {
        let json = serde_json::to_vec_pretty(&*self.index.lock().unwrap()).expect("index serializes");
        if let Err(e) = write_atomic(&self.paths.index, &json, None) {
            tracing::error!("failed to write cache index: {e}");
        }
    }

    pub fn video_lock(&self, video_id: &str) -> Arc<tokio::sync::Mutex<()>> {
        self.locks
            .lock()
            .unwrap()
            .entry(video_id.to_string())
            .or_default()
            .clone()
    }

    pub fn lookup_song(&self, artist: &str, title: &str) -> Option<SongEntry> {
        self.index.lock().unwrap().songs.get(&song_key(artist, title)).cloned()
    }

    pub fn remember_song(&self, artist: &str, title: &str, entry: SongEntry) {
        self.index.lock().unwrap().songs.insert(song_key(artist, title), entry);
        self.persist();
    }

    pub fn forget_song(&self, artist: &str, title: &str) {
        self.index.lock().unwrap().songs.remove(&song_key(artist, title));
        self.persist();
    }

    pub fn has_audio(&self, video_id: &str) -> bool {
        self.paths.audio_file(video_id).is_file()
    }

    pub fn record_file(&self, video_id: &str) {
        let size = fs::metadata(self.paths.audio_file(video_id))
            .map(|m| m.len())
            .unwrap_or(0);
        self.index.lock().unwrap().files.insert(
            video_id.to_string(),
            FileEntry {
                size_bytes: size,
                added_at: Utc::now(),
                last_played: None,
            },
        );
        self.persist();
    }

    pub fn touch(&self, video_id: &str) {
        if let Some(f) = self.index.lock().unwrap().files.get_mut(video_id) {
            f.last_played = Some(Utc::now());
        }
        self.persist();
    }

    pub fn remove_file(&self, video_id: &str) {
        let _ = fs::remove_file(self.paths.audio_file(video_id));
        self.index.lock().unwrap().files.remove(video_id);
        self.persist();
    }

    pub fn total_bytes(&self) -> u64 {
        self.index.lock().unwrap().files.values().map(|f| f.size_bytes).sum()
    }

    /// Deletes least recently played files until the cache fits `max_bytes`.
    pub fn evict(&self, max_bytes: u64, protected: &HashSet<String>) {
        let victims = plan_eviction(&self.index.lock().unwrap().files, max_bytes, protected);
        for id in victims {
            tracing::info!("evicting {id} from audio cache");
            self.remove_file(&id);
        }
    }

    /// Clears `cache/audio/*`, except files of the current playlist. Returns bytes freed.
    pub fn clear(&self, protected: &HashSet<String>) -> u64 {
        let mut freed = 0;
        if let Ok(entries) = fs::read_dir(&self.paths.audio) {
            for e in entries.flatten() {
                let path = e.path();
                let stem = path
                    .file_name()
                    .and_then(|n| n.to_str())
                    .and_then(|n| n.split('.').next())
                    .unwrap_or("");
                if protected.contains(stem) {
                    continue;
                }
                let size = e.metadata().map(|m| m.len()).unwrap_or(0);
                if fs::remove_file(&path).is_ok() {
                    freed += size;
                }
            }
        }
        self.index.lock().unwrap().files.retain(|id, _| protected.contains(id));
        self.persist();
        freed
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Duration;

    fn entry(size: u64, age_min: i64, played_min: Option<i64>) -> FileEntry {
        let now = Utc::now();
        FileEntry {
            size_bytes: size,
            added_at: now - Duration::minutes(age_min),
            last_played: played_min.map(|m| now - Duration::minutes(m)),
        }
    }

    #[test]
    fn evicts_least_recently_played_first() {
        let mut files = BTreeMap::new();
        files.insert("old".to_string(), entry(100, 60, None));
        files.insert("played_recently".to_string(), entry(100, 120, Some(1)));
        files.insert("played_long_ago".to_string(), entry(100, 120, Some(30)));
        let plan = plan_eviction(&files, 150, &HashSet::new());
        assert_eq!(plan, vec!["old", "played_long_ago"]);
    }

    #[test]
    fn never_evicts_protected() {
        let mut files = BTreeMap::new();
        files.insert("a".to_string(), entry(100, 60, None));
        files.insert("b".to_string(), entry(100, 10, None));
        let protected: HashSet<String> = ["a".to_string()].into();
        assert_eq!(plan_eviction(&files, 50, &protected), vec!["b"]);
        assert!(plan_eviction(&files, 500, &HashSet::new()).is_empty());
    }

    #[test]
    fn reconciles_with_disk_and_clears() {
        let dir = tempfile::tempdir().unwrap();
        let paths = Paths::new(dir.path().to_path_buf());
        paths.ensure().unwrap();
        fs::write(paths.audio_file("keep"), vec![0u8; 10]).unwrap();
        fs::write(paths.audio_file("drop"), vec![0u8; 20]).unwrap();

        let cache = Cache::load(&paths);
        assert_eq!(cache.total_bytes(), 30);
        cache.remember_song(
            "Sting",
            "Fragile",
            SongEntry {
                video_id: "keep".into(),
                duration_sec: Some(200),
            },
        );
        assert_eq!(cache.lookup_song("sting", "FRAGILE!").unwrap().video_id, "keep");

        let freed = cache.clear(&["keep".to_string()].into());
        assert_eq!(freed, 20);
        assert!(cache.has_audio("keep"));
        assert!(!cache.has_audio("drop"));

        let reloaded = Cache::load(&paths);
        assert_eq!(reloaded.total_bytes(), 10);
        assert!(reloaded.lookup_song("Sting", "Fragile").is_some());
    }
}
