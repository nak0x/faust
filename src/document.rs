//! An open tab: a file on disk plus its parsed preview.

use std::path::{Path, PathBuf};

use crate::markdown;

/// Files above this size are listed rather than rendered. A note that large is
/// not a note, and parsing it would spike memory for no benefit.
const MAX_BYTES: u64 = 4 << 20;

/// How much of the file Faust sniffs before deciding it is binary.
const SNIFF_BYTES: usize = 8 << 10;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Status {
    Ready,
    Missing,
    TooLarge(u64),
    Binary,
    Error(String),
}

pub struct Document {
    pub path: PathBuf,
    /// File name without the `.md` extension, shown on the tab.
    pub title: String,
    pub body: markdown::Document,
    pub status: Status,
    /// Bumped on every reload so the view can reset its scroll if it wants to.
    pub revision: u64,
}

impl Document {
    pub fn open(path: PathBuf) -> Self {
        let title = title_of(&path);
        let mut doc = Self {
            path,
            title,
            body: markdown::Document::default(),
            status: Status::Ready,
            revision: 0,
        };
        doc.reload();
        doc
    }

    /// Re-reads the file from disk and re-parses it.
    pub fn reload(&mut self) {
        self.revision = self.revision.wrapping_add(1);
        self.title = title_of(&self.path);

        let metadata = match std::fs::metadata(&self.path) {
            Ok(metadata) => metadata,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
                self.status = Status::Missing;
                self.body = markdown::Document::default();
                return;
            }
            Err(err) => {
                self.status = Status::Error(err.to_string());
                self.body = markdown::Document::default();
                return;
            }
        };

        if metadata.len() > MAX_BYTES {
            self.status = Status::TooLarge(metadata.len());
            self.body = markdown::Document::default();
            return;
        }

        let bytes = match std::fs::read(&self.path) {
            Ok(bytes) => bytes,
            Err(err) => {
                self.status = Status::Error(err.to_string());
                self.body = markdown::Document::default();
                return;
            }
        };

        if bytes[..bytes.len().min(SNIFF_BYTES)].contains(&0) {
            self.status = Status::Binary;
            self.body = markdown::Document::default();
            return;
        }

        let text = String::from_utf8_lossy(&bytes).into_owned();
        self.status = Status::Ready;
        self.body = if is_markdown(&self.path) {
            markdown::parse(&text)
        } else {
            markdown::as_code(extension(&self.path), text)
        };
    }
}

pub fn is_markdown(path: &Path) -> bool {
    matches!(
        extension(path).as_deref(),
        Some("md" | "markdown" | "mdown" | "mkd" | "txt")
    )
}

fn extension(path: &Path) -> Option<String> {
    path.extension()
        .map(|ext| ext.to_string_lossy().to_lowercase())
}

fn title_of(path: &Path) -> String {
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string());
    match name.rsplit_once('.') {
        Some((stem, "md" | "markdown")) if !stem.is_empty() => stem.to_owned(),
        _ => name,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn titles_drop_only_the_markdown_extension() {
        assert_eq!(title_of(Path::new("/v/Daily Note.md")), "Daily Note");
        assert_eq!(title_of(Path::new("/v/main.rs")), "main.rs");
        assert_eq!(title_of(Path::new("/v/.gitignore")), ".gitignore");
    }
}
