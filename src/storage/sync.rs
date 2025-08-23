//! Music collection synchronization engine
//!
//! Handles comparing Bandcamp collections with storage, downloading missing albums,
//! and managing the sync process with parallel downloads.

use anyhow::Result;
use colored::Colorize;
use indicatif::{MultiProgress, ProgressBar, ProgressStyle};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tempfile::TempDir;
use tokio::sync::Semaphore;
use tracing::{debug, info};

use super::{
    AlbumToDownload, ConflictInfo, DryRunResult, MusicLibraryItem, StorageBackend, SyncOptions,
};
use crate::bandcamp::{DownloadManager, models::CollectionItem};

/// Synchronization engine for managing music collection updates
pub struct SyncEngine {
    storage: Arc<dyn StorageBackend>,
    options: SyncOptions,
}

impl SyncEngine {
    pub fn new(storage: Box<dyn StorageBackend>, options: SyncOptions) -> Self {
        Self {
            storage: Arc::from(storage),
            options,
        }
    }

    /// Scans the storage backend for existing music
    pub async fn scan_library(&self) -> Result<Vec<MusicLibraryItem>> {
        info!(
            "Scanning {} for existing music...",
            self.storage.storage_type()
        );

        let mut library = Vec::new();
        let root_items = self.storage.list_directory(&PathBuf::new()).await?;

        debug!("Found {} root items", root_items.len());
        for item in &root_items {
            debug!("Root item: {:?} (is_dir: {})", item.path, item.is_directory);
        }

        for artist_item in root_items {
            if !artist_item.is_directory {
                continue;
            }

            let artist_name = artist_item
                .path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("")
                .to_string();

            let album_items = self.storage.list_directory(&artist_item.path).await?;

            for album_item in album_items {
                if !album_item.is_directory {
                    continue;
                }

                let album_name = album_item
                    .path
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("")
                    .to_string();

                let track_items = self.storage.list_directory(&album_item.path).await?;

                let tracks: Vec<String> = track_items
                    .iter()
                    .filter(|item| !item.is_directory)
                    .filter_map(|item| {
                        let name = item.path.file_name()?.to_str()?;
                        // Filter for audio files
                        if name.ends_with(".mp3")
                            || name.ends_with(".m4a")
                            || name.ends_with(".flac")
                            || name.ends_with(".wav")
                        {
                            Some(name.to_string())
                        } else {
                            None
                        }
                    })
                    .collect();

                let has_cover = track_items.iter().any(|item| {
                    item.path
                        .file_name()
                        .and_then(|n| n.to_str())
                        .map(|n| n == "cover.jpg" || n == "cover.png")
                        .unwrap_or(false)
                });

                if !tracks.is_empty() {
                    library.push(MusicLibraryItem {
                        artist: artist_name.clone(),
                        album: album_name,
                        tracks,
                        has_cover,
                    });
                }
            }
        }

        info!(
            "Found {} albums in {}",
            library.len(),
            self.storage.storage_type()
        );
        Ok(library)
    }

    /// Compares Bandcamp collection with storage to find missing albums
    pub async fn compare_collections(
        &self,
        bandcamp: &[CollectionItem],
        library: &[MusicLibraryItem],
    ) -> Vec<CollectionItem> {
        let library_set: HashSet<(String, String)> = library
            .iter()
            .map(|item| (item.artist.to_lowercase(), item.album.to_lowercase()))
            .collect();

        let missing: Vec<CollectionItem> = bandcamp
            .iter()
            .filter(|item| {
                let key = (
                    item.band_name.to_lowercase(),
                    item.item_title.to_lowercase(),
                );
                !library_set.contains(&key)
            })
            .cloned()
            .collect();

        info!(
            "Comparison result: {} in Bandcamp, {} in library, {} missing",
            bandcamp.len(),
            library.len(),
            missing.len()
        );

        missing
    }

    /// Plan what would be synced (dry run)
    pub async fn plan_sync(&self, missing_items: &[CollectionItem]) -> Result<DryRunResult> {
        let mut result = DryRunResult::new();

        for item in missing_items {
            // Sanitize names for filesystem
            let artist = sanitize_filename(&item.band_name);
            let album = sanitize_filename(&item.item_title);

            let album_path = PathBuf::from(&artist).join(&album);

            // Check if directory would need to be created
            if !self.storage.exists(&album_path).await? {
                if !result.directories_to_create.contains(&album_path) {
                    result.directories_to_create.push(album_path.clone());
                }
            } else {
                // Directory exists, check for conflicts
                let existing_tracks = self.storage.list_directory(&album_path).await?;
                if !existing_tracks.is_empty() {
                    result.conflicts.push(ConflictInfo {
                        path: album_path.clone(),
                        reason: format!(
                            "Album directory already exists with {} files",
                            existing_tracks.len()
                        ),
                    });
                }
            }

            // Get track count if available
            let estimated_tracks = item.num_streamable_tracks.map(|n| n as usize);

            // Check if album has cover art and we want to download it
            let has_cover_art = !self.options.skip_cover_art
                && (item.item_art_id.is_some() || item.item_art_url.is_some());

            result.albums_to_download.push(AlbumToDownload {
                artist: item.band_name.clone(),
                album: item.item_title.clone(),
                track_count: estimated_tracks,
                has_cover_art,
                destination_path: album_path.clone(),
            });

            result.total_albums += 1;
            if let Some(tracks) = estimated_tracks {
                result.total_tracks += tracks;

                // Estimate file paths that would be created
                for i in 1..=tracks {
                    let track_path = album_path.join(format!(
                        "track_{:02}.{}",
                        i,
                        self.options.audio_format.extension()
                    ));
                    result.files_to_write.push(track_path);
                }
            }

            if has_cover_art {
                result.files_to_write.push(album_path.join("cover.jpg"));
            }
        }

        // Estimate size (rough: 5MB per track for AAC, 8MB for MP3, 30MB for FLAC)
        // Only calculate if we have track counts
        if result.total_tracks > 0 {
            let mb_per_track = match self.options.audio_format {
                super::AudioFormat::Aac => 5.0,
                super::AudioFormat::Mp3 => 8.0,
                super::AudioFormat::Flac => 30.0,
                super::AudioFormat::Wav => 50.0,
            };
            result.estimated_size_mb = result.total_tracks as f64 * mb_per_track;
        } else {
            result.estimated_size_mb = 0.0; // Unknown
        }

        Ok(result)
    }

    /// Display the dry run result
    pub fn display_dry_run(&self, result: &DryRunResult) {
        println!();
        println!(
            "{}",
            "═══════════════════════════════════════════════════════".cyan()
        );
        println!("{}", "DRY RUN - No changes will be made".yellow().bold());
        println!(
            "{}",
            "═══════════════════════════════════════════════════════".cyan()
        );
        println!();

        println!("{}: {}", "Storage Type".cyan(), self.storage.storage_type());
        println!("{}: {}", "Destination".cyan(), self.storage.root_path());
        println!();

        println!("{}", "Summary:".green().bold());
        println!("  {} albums to download", result.total_albums);

        // Count albums with unknown track counts
        let unknown_count = result
            .albums_to_download
            .iter()
            .filter(|a| a.track_count.is_none())
            .count();

        if unknown_count > 0 {
            if result.total_tracks > 0 {
                // We have some known tracks but also unknowns
                println!(
                    "  {} known tracks + {} albums with unknown track count",
                    result.total_tracks, unknown_count
                );
                println!(
                    "  Estimated size: >{:.1} MB (partial estimate)",
                    result.estimated_size_mb
                );
            } else {
                // All albums have unknown track counts
                println!("  Track count: Unknown (will be determined during download)");
                println!("  Estimated size: Unknown");
            }
        } else {
            // All track counts are known
            println!("  {} tracks total", result.total_tracks);
            println!("  Estimated size: ~{:.1} MB", result.estimated_size_mb);
        }
        println!();

        if !result.directories_to_create.is_empty() {
            println!("{}", "Directories to create:".blue().bold());
            for dir in &result.directories_to_create {
                println!("  📁 {}", dir.display());
            }
            println!();
        }

        if !result.albums_to_download.is_empty() {
            println!("{}", "Albums to download:".green().bold());
            for album in &result.albums_to_download {
                let track_info = match album.track_count {
                    Some(count) => format!("{} tracks", count),
                    None => "?? tracks".to_string(),
                };
                println!(
                    "  🎵 {} - {} ({}{})",
                    album.artist.bright_cyan(),
                    album.album.bright_white(),
                    track_info,
                    if album.has_cover_art { " + cover" } else { "" }
                );
            }
            println!();
        }

        if !result.conflicts.is_empty() {
            println!("{}", "⚠️  Potential conflicts:".yellow().bold());
            for conflict in &result.conflicts {
                println!(
                    "  {} - {}",
                    conflict.path.display(),
                    conflict.reason.yellow()
                );
            }
            println!();
        }

        println!("{}", "─".repeat(56).bright_black());
        println!();
        println!(
            "{}",
            "To proceed with the actual sync, run without --dry-run".cyan()
        );
        println!();
    }

    /// Perform the actual sync
    pub async fn sync_missing(&self, missing_items: &[CollectionItem], cookie: &str) -> Result<()> {
        if self.options.dry_run {
            let plan = self.plan_sync(missing_items).await?;
            self.display_dry_run(&plan);
            return Ok(());
        }

        if missing_items.is_empty() {
            info!("Nothing to sync - collection is up to date!");
            return Ok(());
        }

        println!();
        println!("{}", "Starting sync...".green().bold());
        println!("Found {} albums to download", missing_items.len());

        if self.options.parallel_downloads > 0 {
            println!(
                "Using {} parallel download workers",
                self.options.parallel_downloads
            );
        }
        println!();

        let temp_dir = Arc::new(TempDir::new()?);

        if self.options.parallel_downloads == 0 {
            // Sequential mode (original implementation)
            self.sync_sequential(missing_items, cookie, &temp_dir).await
        } else {
            // Parallel mode
            self.sync_parallel(missing_items, cookie, temp_dir).await
        }
    }

    /// Sequential sync (original implementation)
    async fn sync_sequential(
        &self,
        missing_items: &[CollectionItem],
        cookie: &str,
        temp_dir: &TempDir,
    ) -> Result<()> {
        let download_manager = DownloadManager::new(cookie.to_string());

        // Create progress bar
        let pb = ProgressBar::new(missing_items.len() as u64);
        pb.set_style(
            ProgressStyle::default_bar()
                .template("{spinner:.green} [{bar:40.cyan/blue}] {pos}/{len} {msg}")
                .unwrap()
                .progress_chars("#>-"),
        );

        for item in missing_items.iter() {
            pb.set_message(format!(
                "Downloading: {} - {}",
                item.band_name, item.item_title
            ));

            // Download the album
            match self
                .download_and_sync_album(&download_manager, item, temp_dir.path())
                .await
            {
                Ok(_) => {
                    pb.inc(1);
                    println!(
                        "  {} {} - {}",
                        "✓".green(),
                        item.band_name.bright_cyan(),
                        item.item_title.bright_white()
                    );
                }
                Err(e) => {
                    pb.inc(1);
                    println!(
                        "  {} {} - {}: {}",
                        "✗".red(),
                        item.band_name.bright_cyan(),
                        item.item_title.bright_white(),
                        e.to_string().red()
                    );
                    // Continue with next album even if one fails
                }
            }
        }

        pb.finish_with_message("Sync complete!");
        println!();
        println!("{}", "All albums processed!".green().bold());

        Ok(())
    }

    /// Parallel sync with worker pool
    async fn sync_parallel(
        &self,
        missing_items: &[CollectionItem],
        cookie: &str,
        temp_dir: Arc<TempDir>,
    ) -> Result<()> {
        // Create semaphore to limit concurrent downloads
        let semaphore = Arc::new(Semaphore::new(self.options.parallel_downloads));

        // Create multi-progress for parallel progress bars
        let multi_progress = MultiProgress::new();
        let main_pb = multi_progress.add(ProgressBar::new(missing_items.len() as u64));
        main_pb.set_style(
            ProgressStyle::default_bar()
                .template("[{elapsed_precise}] [{bar:40.cyan/blue}] {pos}/{len} albums ({eta})")
                .unwrap()
                .progress_chars("#>-"),
        );

        // Clone necessary data for async tasks
        let storage = self.storage.clone();
        let cookie = Arc::new(cookie.to_string());

        // Create tasks for all albums
        let tasks: Vec<_> = missing_items
            .iter()
            .map(|item| {
                let item = item.clone();
                let semaphore = semaphore.clone();
                let storage = storage.clone();
                let cookie = cookie.clone();
                let temp_dir = temp_dir.clone();
                let multi_progress = multi_progress.clone();
                let main_pb = main_pb.clone();

                tokio::spawn(async move {
                    // Acquire permit from semaphore
                    let _permit = semaphore.acquire().await.unwrap();

                    // Create a progress bar for this album
                    let album_pb = multi_progress.add(ProgressBar::new_spinner());
                    album_pb.set_style(
                        ProgressStyle::default_spinner()
                            .template("{spinner:.green} {msg}")
                            .unwrap(),
                    );
                    album_pb.set_message(format!("{} - {}", item.band_name, item.item_title));

                    // Download the album
                    let download_manager = DownloadManager::new((*cookie).clone());
                    let result = Self::download_and_sync_album_static(
                        &*storage,
                        &download_manager,
                        &item,
                        temp_dir.path(),
                    )
                    .await;

                    // Update progress
                    album_pb.finish_and_clear();
                    main_pb.inc(1);

                    match &result {
                        Ok(_) => {
                            println!(
                                "  {} {} - {}",
                                "✓".green(),
                                item.band_name.bright_cyan(),
                                item.item_title.bright_white()
                            );
                        }
                        Err(e) => {
                            println!(
                                "  {} {} - {}: {}",
                                "✗".red(),
                                item.band_name.bright_cyan(),
                                item.item_title.bright_white(),
                                e.to_string().red()
                            );
                        }
                    }

                    (item, result)
                })
            })
            .collect();

        // Wait for all tasks to complete
        let results = futures::future::join_all(tasks).await;

        main_pb.finish_with_message("Sync complete!");

        // Count successes and failures
        let mut success_count = 0;
        let mut failure_count = 0;

        for result in results {
            match result {
                Ok((_, Ok(_))) => success_count += 1,
                Ok((_, Err(_))) => failure_count += 1,
                Err(_) => failure_count += 1,
            }
        }

        println!();
        if failure_count == 0 {
            println!(
                "{}",
                format!("All {} albums synced successfully!", success_count)
                    .green()
                    .bold()
            );
        } else {
            println!(
                "{}",
                format!(
                    "Sync complete: {} succeeded, {} failed",
                    success_count, failure_count
                )
                .yellow()
                .bold()
            );
        }

        Ok(())
    }

    /// Static version for parallel execution
    async fn download_and_sync_album_static(
        storage: &dyn StorageBackend,
        download_manager: &DownloadManager,
        item: &CollectionItem,
        temp_dir: &Path,
    ) -> Result<()> {
        debug!("Processing album: {} - {}", item.band_name, item.item_title);

        // Sanitize names for filesystem
        let artist = sanitize_filename(&item.band_name);
        let album = sanitize_filename(&item.item_title);
        let album_path = PathBuf::from(&artist).join(&album);

        // Create the album directory in storage
        storage.create_directory(&album_path).await?;

        // Download tracks directly from the album page (like bandcamp-dl does)
        debug!(
            "Album: {} - {}, URL: {}",
            item.band_name, item.item_title, item.item_url
        );

        // Download to temp directory first
        let temp_album_dir = temp_dir.join(format!("{}-{}", artist, album));
        let downloaded_files = download_manager
            .download_tracks_from_album_page(&item.item_url, &temp_album_dir)
            .await?;

        // Upload downloaded files to storage
        for file_name in downloaded_files {
            let source_path = temp_album_dir.join(&file_name);
            let dest_path = album_path.join(&file_name);

            // Read file and upload to storage
            let data = tokio::fs::read(&source_path).await?;
            storage.write_file(&dest_path, &data).await?;

            debug!("Uploaded: {}", dest_path.display());
        }

        Ok(())
    }

    /// Download and sync a single album
    async fn download_and_sync_album(
        &self,
        download_manager: &DownloadManager,
        item: &CollectionItem,
        temp_dir: &Path,
    ) -> Result<()> {
        Self::download_and_sync_album_static(&*self.storage, download_manager, item, temp_dir).await
    }
}

/// Sanitize a filename for safe filesystem usage
fn sanitize_filename(name: &str) -> String {
    // Replace problematic characters
    name.chars()
        .map(|c| match c {
            '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' => '_',
            '\0' => '_',
            _ => c,
        })
        .collect::<String>()
        .trim()
        .to_string()
}
