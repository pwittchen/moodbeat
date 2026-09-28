use serde::{Serialize, Serializer};

/// Domain errors. The `Display` text is exactly what the user sees, so it must never
/// contain secrets (see [`redact`]).
#[derive(Debug, thiserror::Error)]
pub enum AppError {
    #[error("Add your OpenAI API key in Settings")]
    NoApiKey,
    #[error("OpenAI rejected the API key. Check it in Settings.")]
    InvalidApiKey,
    #[error("OpenAI rate limit or quota reached. Try again later.")]
    RateLimited,
    #[error("Couldn't reach OpenAI. Check your connection.")]
    Network,
    #[error("OpenAI error: {0}")]
    OpenAi(String),
    #[error("Couldn't build a playlist for this mood. Try describing it differently.")]
    TooFewSongs,
    #[error("Can't write to ~/.moodbeat: {0}")]
    Storage(String),
    #[error("{0}")]
    Tools(String),
    #[error("{0}")]
    Invalid(String),
}

impl AppError {
    pub fn storage(e: impl std::fmt::Display) -> Self {
        AppError::Storage(e.to_string())
    }
}

impl Serialize for AppError {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&redact(&self.to_string()))
    }
}

pub type AppResult<T> = Result<T, AppError>;

/// Masks anything that looks like an OpenAI key (`sk-...`), e.g. inside an API error body.
pub fn redact(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(pos) = rest.find("sk-") {
        out.push_str(&rest[..pos]);
        out.push_str("sk-***");
        let tail = &rest[pos + 3..];
        let end = tail
            .find(|c: char| !(c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '*'))
            .unwrap_or(tail.len());
        rest = &tail[end..];
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redacts_keys() {
        assert_eq!(
            redact("Incorrect API key provided: sk-proj-abc*****XYZ9. See docs."),
            "Incorrect API key provided: sk-***. See docs."
        );
        assert_eq!(redact("no secrets here"), "no secrets here");
        assert_eq!(redact("sk-a and sk-b"), "sk-*** and sk-***");
    }

    #[test]
    fn serializes_as_redacted_message() {
        let e = AppError::OpenAi("bad key sk-12345".into());
        assert_eq!(serde_json::to_string(&e).unwrap(), "\"OpenAI error: bad key sk-***\"");
    }
}
