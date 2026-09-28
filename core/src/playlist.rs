//! Playlist model + persistence in `~/.moodbeat/playlists/` (SPEC §8.2).

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::llm::LlmPlaylist;
use crate::paths::{write_atomic, Paths};
use crate::resolver::normalize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TrackStatus {
    Pending,
    Searching,
    Downloading,
    Ready,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Track {
    pub id: String,
    pub artist: String,
    pub title: String,
    pub year: Option<i32>,
    pub video_id: Option<String>,
    pub status: TrackStatus,
    pub duration_sec: Option<u32>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Playlist {
    pub version: u32,
    pub id: String,
    pub created_at: DateTime<Utc>,
    pub mood: String,
    pub title: String,
    pub model: String,
    pub tracks: Vec<Track>,
}

impl Playlist {
    pub fn new(mood: &str, model: &str, llm: LlmPlaylist) -> Self {
        let tracks = llm
            .songs
            .into_iter()
            .enumerate()
            .map(|(i, s)| Track {
                id: format!("t{}", i + 1),
                artist: s.artist,
                title: s.title,
                year: s.year,
                video_id: None,
                status: TrackStatus::Pending,
                duration_sec: None,
                error: None,
            })
            .collect();
        Self {
            version: 1,
            id: ulid::Ulid::new().to_string(),
            created_at: Utc::now(),
            mood: mood.to_string(),
            title: llm.title,
            model: model.to_string(),
            tracks,
        }
    }

    /// e.g. `2026-09-28T21-14-03_rainy-day-in-new-york.json`
    pub fn file_name(&self) -> String {
        format!("{}_{}.json", self.created_at.format("%Y-%m-%dT%H-%M-%S"), slugify(&self.mood))
    }

    pub fn save(&self, paths: &Paths) -> std::io::Result<()> {
        let json = serde_json::to_vec_pretty(self).expect("playlist serializes");
        write_atomic(&paths.playlists.join(self.file_name()), &json, None)
    }

    pub fn track(&self, id: &str) -> Option<&Track> {
        self.tracks.iter().find(|t| t.id == id)
    }

    pub fn track_mut(&mut self, id: &str) -> Option<&mut Track> {
        self.tracks.iter_mut().find(|t| t.id == id)
    }
}

pub fn slugify(text: &str) -> String {
    let slug: String = normalize(text).replace(' ', "-").chars().take(50).collect();
    let slug = slug.trim_matches('-').to_string();
    if slug.is_empty() { "playlist".into() } else { slug }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::llm::Song;

    #[test]
    fn builds_and_names() {
        let llm = LlmPlaylist {
            title: "Rainy".into(),
            songs: vec![Song { artist: "Sting".into(), title: "Englishman".into(), year: Some(1987) }],
        };
        let pl = Playlist::new("Rainy day in New York!", "gpt-4.1-mini", llm);
        assert_eq!(pl.tracks[0].id, "t1");
        assert_eq!(pl.tracks[0].status, TrackStatus::Pending);
        assert!(pl.file_name().ends_with("_rainy-day-in-new-york.json"));
        assert_eq!(pl.file_name().len(), "2026-09-28T21-14-03_rainy-day-in-new-york.json".len());

        let json = serde_json::to_value(&pl).unwrap();
        assert_eq!(json["tracks"][0]["status"], "pending");
        assert!(json["tracks"][0].get("durationSec").is_some());
        assert!(json.get("createdAt").is_some());
    }

    #[test]
    fn slugs() {
        assert_eq!(slugify("Polish summertime"), "polish-summertime");
        assert_eq!(slugify("!!!"), "playlist");
        assert_eq!(slugify("Łódź nocą"), "lodz-noca");
    }
}
