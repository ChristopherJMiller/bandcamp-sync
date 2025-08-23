use anyhow::Result;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

pub mod local;
pub mod sync;
pub mod webdav;

pub use local::LocalStorage;
pub use sync::SyncEngine;
pub use webdav::WebDavStorage;

/// Represents a file or directory in the storage
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StorageItem {
    pub path: PathBuf,
    pub is_directory: bool,
    pub size: Option<u64>,
    pub modified: Option<chrono::DateTime<chrono::Utc>>,
}

/// Represents an artist/album structure
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MusicLibraryItem {
    pub artist: String,
    pub album: String,
    pub tracks: Vec<String>,
    pub has_cover: bool,
}

/// Common trait for storage backends
#[async_trait]
pub trait StorageBackend: Send + Sync {
    /// List all items in a directory
    async fn list_directory(&self, path: &Path) -> Result<Vec<StorageItem>>;

    /// Check if a path exists
    async fn exists(&self, path: &Path) -> Result<bool>;

    /// Create a directory (including parents)
    async fn create_directory(&self, path: &Path) -> Result<()>;

    /// Upload/write a file
    async fn write_file(&self, path: &Path, data: &[u8]) -> Result<()>;

    /// Get storage type name for display
    fn storage_type(&self) -> &str;

    /// Get the root path/URL
    fn root_path(&self) -> String;
}

/// Options for sync operations
#[derive(Debug, Clone)]
pub struct SyncOptions {
    pub dry_run: bool,
    pub parallel_downloads: usize,  // 0 = disabled, 1-6 = number of workers
    pub skip_cover_art: bool,
    pub audio_format: AudioFormat,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AudioFormat {
    Aac,
    Mp3,
    Flac,
    Wav,
}

impl AudioFormat {
    pub fn extension(&self) -> &str {
        match self {
            AudioFormat::Aac => "m4a",
            AudioFormat::Mp3 => "mp3",
            AudioFormat::Flac => "flac",
            AudioFormat::Wav => "wav",
        }
    }
}

/// Result of a dry-run sync operation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DryRunResult {
    pub albums_to_download: Vec<AlbumToDownload>,
    pub total_albums: usize,
    pub total_tracks: usize,
    pub estimated_size_mb: f64,
    pub directories_to_create: Vec<PathBuf>,
    pub files_to_write: Vec<PathBuf>,
    pub conflicts: Vec<ConflictInfo>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AlbumToDownload {
    pub artist: String,
    pub album: String,
    pub track_count: Option<usize>,
    pub has_cover_art: bool,
    pub destination_path: PathBuf,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConflictInfo {
    pub path: PathBuf,
    pub reason: String,
}

impl DryRunResult {
    pub fn new() -> Self {
        Self {
            albums_to_download: Vec::new(),
            total_albums: 0,
            total_tracks: 0,
            estimated_size_mb: 0.0,
            directories_to_create: Vec::new(),
            files_to_write: Vec::new(),
            conflicts: Vec::new(),
        }
    }
}
