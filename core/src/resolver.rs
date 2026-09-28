//! Resolving a song to a YouTube video via `yt-dlp ytsearch5:` (SPEC §6.2).

use std::path::Path;
use std::process::Stdio;
use std::time::Duration;

use serde::Deserialize;
use tokio_util::sync::CancellationToken;
use unicode_normalization::char::is_combining_mark;
use unicode_normalization::UnicodeNormalization;

use crate::tools;

const SEARCH_TIMEOUT: Duration = Duration::from_secs(60);
const MIN_DURATION: f64 = 60.0;
const MAX_DURATION: f64 = 15.0 * 60.0;
const LONG_DURATION: f64 = 8.0 * 60.0;

const BAD_WORDS: &[&str] = &[
    "live",
    "cover",
    "karaoke",
    "remix",
    "reaction",
    "8d",
    "slowed",
    "sped up",
    "nightcore",
    "instrumental",
];

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct SearchResult {
    pub id: String,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub channel: Option<String>,
    #[serde(default)]
    pub uploader: Option<String>,
    #[serde(default)]
    pub duration: Option<f64>,
}

#[derive(Debug, thiserror::Error)]
pub enum SearchError {
    #[error("cancelled")]
    Cancelled,
    #[error("search failed: {0}")]
    Failed(String),
}

/// Runs the search and returns the parsed candidates.
pub async fn search(
    yt_dlp: &Path,
    artist: &str,
    title: &str,
    cancel: &CancellationToken,
) -> Result<Vec<SearchResult>, SearchError> {
    let mut cmd = tools::command(yt_dlp);
    cmd.arg(format!("ytsearch5:{artist} - {title}"))
        .args(["--dump-json", "--flat-playlist", "--no-warnings"])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    let child = cmd.spawn().map_err(|e| SearchError::Failed(e.to_string()))?;

    let output = tokio::select! {
        () = cancel.cancelled() => return Err(SearchError::Cancelled),
        r = tokio::time::timeout(SEARCH_TIMEOUT, child.wait_with_output()) => match r {
            Err(_) => return Err(SearchError::Failed("timed out".into())),
            Ok(Err(e)) => return Err(SearchError::Failed(e.to_string())),
            Ok(Ok(o)) => o,
        },
    };
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let last = stderr.lines().rfind(|l| !l.trim().is_empty()).unwrap_or("").to_string();
        return Err(SearchError::Failed(last));
    }
    Ok(parse_search_output(&String::from_utf8_lossy(&output.stdout)))
}

/// One JSON object per line; unparseable lines are skipped.
pub fn parse_search_output(stdout: &str) -> Vec<SearchResult> {
    stdout
        .lines()
        .filter(|l| !l.trim().is_empty())
        .filter_map(|l| serde_json::from_str::<SearchResult>(l).ok())
        .filter(|r| !r.id.is_empty())
        .collect()
}

/// Lowercase, no diacritics, punctuation → spaces, whitespace collapsed.
pub fn normalize(s: &str) -> String {
    let folded: String = s
        .nfd()
        .filter(|c| !is_combining_mark(*c))
        .flat_map(|c| {
            // Letters NFD does not decompose.
            let mapped: &str = match c {
                'ł' | 'Ł' => "l",
                'ø' | 'Ø' => "o",
                'đ' | 'Đ' => "d",
                'ß' => "ss",
                'æ' | 'Æ' => "ae",
                '&' => " and ",
                _ => "",
            };
            if mapped.is_empty() {
                vec![c]
            } else {
                mapped.chars().collect()
            }
        })
        .flat_map(char::to_lowercase)
        .map(|c| if c.is_alphanumeric() { c } else { ' ' })
        .collect();
    folded.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn contains_phrase(haystack: &str, needle: &str) -> bool {
    !needle.is_empty() && format!(" {haystack} ").contains(&format!(" {needle} "))
}

/// Scores a candidate; `None` means it is dropped (bad duration).
pub fn score(artist: &str, title: &str, r: &SearchResult) -> Option<i32> {
    if let Some(d) = r.duration {
        if !(MIN_DURATION..=MAX_DURATION).contains(&d) {
            return None;
        }
    }
    let n_artist = normalize(artist);
    let n_title = normalize(title);
    let n_video = normalize(r.title.as_deref().unwrap_or(""));
    let n_channel = normalize(r.channel.as_deref().or(r.uploader.as_deref()).unwrap_or(""));

    let mut score = 0;
    if contains_phrase(&n_video, &n_title) {
        score += 30;
    }
    if contains_phrase(&n_video, &n_artist) {
        score += 20;
    }

    let artist_compact = n_artist.replace(' ', "");
    let channel_compact = n_channel.replace(' ', "");
    let is_topic = n_channel == format!("{n_artist} topic");
    let is_vevo =
        channel_compact == format!("{artist_compact}vevo") || channel_compact == format!("{artist_compact}official");
    if is_topic || is_vevo {
        score += 25;
    } else if channel_compact == artist_compact && !artist_compact.is_empty() {
        score += 20;
    }

    if ["official audio", "official video", "official music video"]
        .iter()
        .any(|p| n_video.contains(p))
    {
        score += 10;
    }

    for bad in BAD_WORDS {
        if contains_phrase(&n_video, bad) && !contains_phrase(&n_title, bad) {
            score -= 40;
        }
    }

    if r.duration.is_some_and(|d| d > LONG_DURATION) {
        score -= 15;
    }
    Some(score)
}

/// Highest score wins; ties go to the earlier (higher-ranked by YouTube) result.
pub fn pick_best(artist: &str, title: &str, results: &[SearchResult]) -> Option<SearchResult> {
    results
        .iter()
        .enumerate()
        .filter_map(|(i, r)| score(artist, title, r).map(|s| (s, i, r)))
        .max_by(|a, b| a.0.cmp(&b.0).then(b.1.cmp(&a.1)))
        .map(|(_, _, r)| r.clone())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn r(id: &str, title: &str, channel: &str, duration: f64) -> SearchResult {
        SearchResult {
            id: id.into(),
            title: Some(title.into()),
            channel: Some(channel.into()),
            uploader: None,
            duration: Some(duration),
        }
    }

    #[test]
    fn normalizes() {
        assert_eq!(normalize("Beyoncé — Crazy in Love!"), "beyonce crazy in love");
        assert_eq!(
            normalize("Czesław Niemen: Dziwny jest ten świat"),
            "czeslaw niemen dziwny jest ten swiat"
        );
        assert_eq!(normalize("Guns N' Roses"), "guns n roses");
    }

    #[test]
    fn picks_official_from_recorded_fixture() {
        let out = parse_search_output(include_str!("../tests/fixtures/ytsearch_sting.jsonl"));
        assert_eq!(out.len(), 5);
        let best = pick_best("Sting", "Englishman in New York", &out).unwrap();
        assert_eq!(best.id, "d27gTrPPAyk");
    }

    #[test]
    fn prefers_topic_channel_and_penalizes_variants() {
        let results = vec![
            r("live", "Artist - Song (Live at Wembley)", "Fan", 250.0),
            r("cover", "Song - Artist cover", "Someone", 250.0),
            r("topic", "Song", "Artist - Topic", 250.0),
        ];
        assert_eq!(pick_best("Artist", "Song", &results).unwrap().id, "topic");
    }

    #[test]
    fn bad_word_allowed_when_in_song_title() {
        let live = r("a", "Artist - Live Forever (Official Video)", "ArtistVEVO", 270.0);
        assert!(score("Artist", "Live Forever", &live).unwrap() > 50);
    }

    #[test]
    fn duration_rules() {
        assert_eq!(score("A", "S", &r("x", "A - S", "A", 30.0)), None);
        assert_eq!(score("A", "S", &r("x", "A - S", "A", 16.0 * 60.0)), None);
        let normal = score("A", "S", &r("x", "A - S", "A", 240.0)).unwrap();
        let long = score("A", "S", &r("x", "A - S", "A", 9.0 * 60.0)).unwrap();
        assert!(long < normal);
        assert!(pick_best("A", "S", &[r("x", "A - S", "A", 20.0)]).is_none());
    }

    #[test]
    fn tie_goes_to_first() {
        let results = vec![r("first", "A - S", "X", 200.0), r("second", "A - S", "Y", 200.0)];
        assert_eq!(pick_best("A", "S", &results).unwrap().id, "first");
    }
}
