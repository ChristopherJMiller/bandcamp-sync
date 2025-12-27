use crate::utils::sanitize_filename;
use anyhow::{Context, Result};
use colored::Colorize;
use indicatif::{ProgressBar, ProgressStyle};
use keyring::Entry;
use std::path::PathBuf;
use tracing::debug;

use super::{AudioFormat, AuthService, Commands, OutputFormat};
use crate::bandcamp::BandcampClient;
use crate::cli::auth::AuthManager;
use crate::storage::{LocalStorage, StorageBackend, SyncEngine, SyncOptions, WebDavStorage};

/// Options for filtering collections by artist/album
struct FilterOptions {
    artist_filter: Option<String>,
    exclude_artist: Option<String>,
    album_filter: Option<String>,
}

/// Options for the sync command
struct SyncCommandOptions {
    storage: StorageOptions,
    dry_run: bool,
    format: AudioFormat,
    parallel: usize,
    no_parallel: bool,
    no_cover: bool,
    filters: FilterOptions,
    shallow: bool,
}

/// Storage backend options
struct StorageOptions {
    webdav_url: Option<String>,
    local_path: Option<String>,
}

pub struct CommandHandler;

impl CommandHandler {
    pub async fn handle(command: Commands) -> Result<()> {
        match command {
            Commands::Auth { service } => Self::handle_auth(service).await,
            Commands::List {
                artist_filter,
                exclude_artist,
                album_filter,
                format,
            } => Self::handle_list(artist_filter, exclude_artist, album_filter, format).await,
            Commands::Scan {
                webdav_url,
                local_path,
                detailed,
            } => Self::handle_scan(webdav_url, local_path, detailed).await,
            Commands::Diff {
                webdav_url,
                local_path,
                missing_only,
                artist_filter,
                exclude_artist,
                shallow,
            } => {
                Self::handle_diff(
                    webdav_url,
                    local_path,
                    missing_only,
                    artist_filter,
                    exclude_artist,
                    shallow,
                )
                .await
            }
            Commands::Sync {
                webdav_url,
                local_path,
                dry_run,
                format,
                parallel,
                no_parallel,
                no_cover,
                artist_filter,
                exclude_artist,
                album_filter,
                shallow,
            } => {
                let options = SyncCommandOptions {
                    storage: StorageOptions {
                        webdav_url,
                        local_path,
                    },
                    dry_run,
                    format,
                    parallel,
                    no_parallel,
                    no_cover,
                    filters: FilterOptions {
                        artist_filter,
                        exclude_artist,
                        album_filter,
                    },
                    shallow,
                };
                Self::handle_sync(options).await
            }
            Commands::Completion { shell } => {
                Self::handle_completion(shell);
                Ok(())
            }
            Commands::ImportZip {
                zip_path,
                webdav_url,
                local_path,
            } => {
                let storage = StorageOptions {
                    webdav_url,
                    local_path,
                };
                Self::handle_import_zip(&zip_path, &storage).await
            }
            Commands::Status => Self::handle_status().await,
            Commands::QueryCd {
                device,
                show_toc,
                no_lookup,
                disc_number,
            } => Self::handle_query_cd(device, show_toc, no_lookup, disc_number).await,
            Commands::ImportCd {
                device,
                webdav_url,
                local_path,
                format,
                no_lookup,
                just_cover,
                disc_number,
            } => {
                Self::handle_import_cd(
                    device,
                    webdav_url,
                    local_path,
                    format,
                    no_lookup,
                    just_cover,
                    disc_number,
                )
                .await
            }
        }
    }

    async fn handle_auth(service: AuthService) -> Result<()> {
        match service {
            AuthService::Bandcamp {
                headless,
                driver,
                driver_port,
                username,
                password,
                cookie,
                force,
            } => {
                println!("{}", "Authenticating with Bandcamp...".blue());
                let _cookie = AuthManager::authenticate_bandcamp(
                    headless,
                    driver,
                    driver_port,
                    username,
                    password,
                    cookie,
                    force,
                )
                .await?;
                println!("{}", "✓ Bandcamp authentication successful".green());
            }
            AuthService::Webdav {
                url,
                username,
                password,
            } => {
                println!(
                    "{}",
                    format!("Authenticating with WebDAV at {}...", url).blue()
                );
                let (_user, _pass) =
                    AuthManager::authenticate_webdav(&url, username, password).await?;
                println!("{}", "✓ WebDAV authentication successful".green());
            }
        }
        Ok(())
    }

    async fn handle_list(
        artist_filter: Option<String>,
        exclude_artist: Option<String>,
        album_filter: Option<String>,
        format: OutputFormat,
    ) -> Result<()> {
        // Get cookie from keyring using AuthManager
        let cookie = match AuthManager::get_bandcamp_cookie() {
            Ok(cookie) => {
                debug!("Using stored Bandcamp authentication");
                cookie
            }
            Err(_) => {
                println!("{}", "No Bandcamp authentication found.".yellow());
                println!("Please run: {} auth bandcamp", "bandcamp-sync".cyan());
                return Ok(());
            }
        };

        let spinner = ProgressBar::new_spinner();
        spinner.set_style(
            ProgressStyle::default_spinner()
                .template("{spinner:.green} {msg}")
                .unwrap(),
        );
        spinner.set_message("Fetching Bandcamp collection...");

        // Fetch collection
        let client = BandcampClient::new(cookie)?;
        let mut collection = client.fetch_collection().await?;

        spinner.finish_with_message("Collection fetched");

        // Apply filters if provided
        if let Some(artist) = &artist_filter {
            let artist_lower = artist.to_lowercase();
            collection.retain(|item| item.band_name.to_lowercase().contains(&artist_lower));
        } else if let Some(exclude) = &exclude_artist {
            let exclude_lower = exclude.to_lowercase();
            collection.retain(|item| !item.band_name.to_lowercase().contains(&exclude_lower));
        }

        if let Some(album) = &album_filter {
            let album_lower = album.to_lowercase();
            collection.retain(|item| {
                item.item_title.to_lowercase().contains(&album_lower)
                    || item
                        .album_title
                        .as_ref()
                        .is_some_and(|t| t.to_lowercase().contains(&album_lower))
            });
        }

        // Sort by artist then album
        collection.sort_by(|a, b| {
            a.band_name
                .cmp(&b.band_name)
                .then_with(|| a.item_title.cmp(&b.item_title))
        });

        match format {
            OutputFormat::Table => {
                println!("\n{}", "Your Bandcamp Collection:".green().bold());
                println!("{}", "─".repeat(80).bright_black());

                if collection.is_empty() {
                    println!("{}", "No items found matching filters".yellow());
                } else {
                    println!(
                        "{:<40} {:<35} {}",
                        "Artist".cyan().bold(),
                        "Album".cyan().bold(),
                        "Type".cyan().bold()
                    );
                    println!("{}", "─".repeat(80).bright_black());

                    for item in &collection {
                        // Print URL for debugging
                        debug!("Album URL: {}", item.item_url);

                        let artist = if item.band_name.chars().count() > 38 {
                            let truncated: String = item.band_name.chars().take(35).collect();
                            format!("{}...", truncated)
                        } else {
                            item.band_name.clone()
                        };

                        let album = if item.item_title.chars().count() > 33 {
                            let truncated: String = item.item_title.chars().take(30).collect();
                            format!("{}...", truncated)
                        } else {
                            item.item_title.clone()
                        };

                        println!(
                            "{:<40} {:<35} {}",
                            artist,
                            album,
                            item.item_type.bright_black()
                        );
                    }

                    println!("{}", "─".repeat(80).bright_black());
                    println!("\n{} {} items", "Total:".green().bold(), collection.len());
                }
            }
            OutputFormat::Json => {
                let json = serde_json::to_string_pretty(&collection)?;
                println!("{}", json);
            }
            OutputFormat::Csv => {
                println!("Artist,Album,Type,URL");
                for item in &collection {
                    println!(
                        "{},{},{},{}",
                        item.band_name.replace(',', ";"),
                        item.item_title.replace(',', ";"),
                        item.item_type,
                        item.item_url
                    );
                }
            }
        }

        Ok(())
    }

    async fn handle_scan(
        webdav_url: Option<String>,
        local_path: Option<String>,
        detailed: bool,
    ) -> Result<()> {
        let storage = Self::get_storage(webdav_url, local_path).await?;

        let spinner = ProgressBar::new_spinner();
        spinner.set_style(
            ProgressStyle::default_spinner()
                .template("{spinner:.green} {msg}")
                .unwrap(),
        );
        spinner.set_message(format!("Scanning {}...", storage.storage_type()));

        let sync_engine = SyncEngine::new(
            storage,
            SyncOptions {
                dry_run: false,
                parallel_downloads: 0, // Not needed for scan
                skip_cover_art: false,
                audio_format: crate::storage::AudioFormat::Aac,
                shallow: true, // Scan doesn't need deep checking
            },
        );

        let library = sync_engine.scan_library().await?;
        spinner.finish_with_message("Scan complete");

        if detailed {
            println!("{}", "Detailed Library:".green().bold());
            println!("{}", "─".repeat(60).bright_black());
            for item in &library {
                println!("{} / {}", item.artist.cyan(), item.album.bright_white());
                for track in &item.tracks {
                    println!("  🎵 {}", track);
                }
                if item.has_cover {
                    println!("  🇺 Cover art present");
                }
            }
        } else {
            println!("{}", "Library Summary:".green().bold());
            println!("{}", "─".repeat(60).bright_black());
            println!("Total albums: {}", library.len());
            let total_tracks: usize = library.iter().map(|i| i.tracks.len()).sum();
            println!("Total tracks: {}", total_tracks);
        }

        Ok(())
    }

    async fn handle_diff(
        webdav_url: Option<String>,
        local_path: Option<String>,
        missing_only: bool,
        artist_filter: Option<String>,
        exclude_artist: Option<String>,
        shallow: bool,
    ) -> Result<()> {
        let storage = Self::get_storage(webdav_url, local_path).await?;

        let spinner = ProgressBar::new_spinner();
        spinner.set_style(
            ProgressStyle::default_spinner()
                .template("{spinner:.green} {msg}")
                .unwrap(),
        );
        spinner.set_message("Fetching Bandcamp collection...");

        // Get Bandcamp collection
        let cookie = AuthManager::get_bandcamp_cookie()?;
        let client = BandcampClient::new(cookie)?;
        let mut bandcamp_collection = client.fetch_collection().await?;

        // Apply filters
        if let Some(artist) = &artist_filter {
            let artist_lower = artist.to_lowercase();
            bandcamp_collection
                .retain(|item| item.band_name.to_lowercase().contains(&artist_lower));
        } else if let Some(exclude) = &exclude_artist {
            let exclude_lower = exclude.to_lowercase();
            bandcamp_collection
                .retain(|item| !item.band_name.to_lowercase().contains(&exclude_lower));
        }

        spinner.set_message(format!("Scanning {}...", storage.storage_type()));

        // Scan storage
        let sync_engine = SyncEngine::new(
            storage,
            SyncOptions {
                dry_run: false,
                parallel_downloads: 0, // Not needed for scan
                skip_cover_art: false,
                audio_format: crate::storage::AudioFormat::Aac,
                shallow,
            },
        );
        let library = sync_engine.scan_library().await?;

        spinner.set_message("Comparing collections...");
        let missing = sync_engine
            .compare_collections(&bandcamp_collection, &library)
            .await;

        spinner.finish_with_message("Comparison complete");

        if missing_only {
            println!("{}", "Missing Albums:".yellow().bold());
            println!("{}", "─".repeat(60).bright_black());
            for item in &missing {
                println!(
                    "💿 {} - {}",
                    item.band_name.cyan(),
                    item.item_title.bright_white()
                );
            }
            println!();
            println!("Total missing: {} albums", missing.len());
        } else {
            println!("{}", "Library Comparison:".yellow().bold());
            println!("{}", "─".repeat(60).bright_black());
            println!("Bandcamp collection: {} items", bandcamp_collection.len());
            println!("Local library: {} albums", library.len());
            println!("Missing in library: {} albums", missing.len());
            println!();
            if !missing.is_empty() {
                println!("{}", "Missing albums:".yellow());
                for item in &missing {
                    println!(
                        "  💿 {} - {}",
                        item.band_name.cyan(),
                        item.item_title.bright_white()
                    );
                }
            }
        }

        Ok(())
    }

    async fn handle_sync(options: SyncCommandOptions) -> Result<()> {
        let storage =
            Self::get_storage(options.storage.webdav_url, options.storage.local_path).await?;

        // Convert AudioFormat
        let storage_format = match options.format {
            AudioFormat::Aac => crate::storage::AudioFormat::Aac,
            AudioFormat::Mp3 => crate::storage::AudioFormat::Mp3,
            AudioFormat::Flac => crate::storage::AudioFormat::Flac,
            AudioFormat::Wav => crate::storage::AudioFormat::Wav,
        };

        let spinner = ProgressBar::new_spinner();
        spinner.set_style(
            ProgressStyle::default_spinner()
                .template("{spinner:.green} {msg}")
                .unwrap(),
        );

        spinner.set_message("Fetching Bandcamp collection...");

        // Get Bandcamp collection
        let cookie = AuthManager::get_bandcamp_cookie()?;
        let client = BandcampClient::new(cookie.clone())?;
        let mut bandcamp_collection = client.fetch_collection().await?;

        // Apply filters
        if let Some(artist) = &options.filters.artist_filter {
            let artist_lower = artist.to_lowercase();
            bandcamp_collection
                .retain(|item| item.band_name.to_lowercase().contains(&artist_lower));
        } else if let Some(exclude) = &options.filters.exclude_artist {
            let exclude_lower = exclude.to_lowercase();
            bandcamp_collection
                .retain(|item| !item.band_name.to_lowercase().contains(&exclude_lower));
        }

        if let Some(album) = &options.filters.album_filter {
            let album_lower = album.to_lowercase();
            bandcamp_collection.retain(|item| {
                item.item_title.to_lowercase().contains(&album_lower)
                    || item
                        .album_title
                        .as_ref()
                        .is_some_and(|t| t.to_lowercase().contains(&album_lower))
            });
        }

        spinner.set_message(format!("Scanning {}...", storage.storage_type()));

        // Calculate parallel downloads
        let parallel_downloads = if options.no_parallel {
            0 // Disabled
        } else if options.parallel > 0 {
            options.parallel.min(6) // User specified, cap at 6
        } else {
            // Auto: use number of CPU cores, capped at 6
            let cpu_count = num_cpus::get();
            cpu_count.clamp(1, 6)
        };

        if !options.dry_run && parallel_downloads > 0 {
            debug!("Using {} parallel download workers", parallel_downloads);
        }

        // Create sync engine
        let sync_engine = SyncEngine::new(
            storage,
            SyncOptions {
                dry_run: options.dry_run,
                parallel_downloads,
                skip_cover_art: options.no_cover,
                audio_format: storage_format,
                shallow: options.shallow,
            },
        );

        let library = sync_engine.scan_library().await?;

        spinner.set_message("Comparing collections...");
        let missing = sync_engine
            .compare_collections(&bandcamp_collection, &library)
            .await;

        spinner.finish_and_clear();

        if missing.is_empty() {
            println!("{}", "✓ Everything is already synced!".green().bold());
            return Ok(());
        }

        // Perform sync (dry-run or actual)
        sync_engine.sync_missing(&missing, &cookie).await?;

        Ok(())
    }

    fn handle_completion(shell: clap_complete::Shell) {
        use crate::cli::Cli;
        use clap::CommandFactory;

        let mut cmd = Cli::command();
        super::completions::generate_completions(shell, &mut cmd);
    }

    async fn get_storage(
        webdav_url: Option<String>,
        local_path: Option<String>,
    ) -> Result<Box<dyn StorageBackend>> {
        match (webdav_url, local_path) {
            (Some(url), None) => {
                // Get WebDAV credentials from keyring or prompt
                let (username, password) =
                    AuthManager::authenticate_webdav(&url, None, None).await?;

                let storage = WebDavStorage::new(&url, Some(username), Some(password)).await?;
                Ok(Box::new(storage))
            }
            (None, Some(path)) => {
                let storage = LocalStorage::new(&path)?;
                Ok(Box::new(storage))
            }
            (None, None) => {
                anyhow::bail!("Either --webdav-url or --local-path must be provided")
            }
            (Some(_), Some(_)) => {
                anyhow::bail!("Cannot specify both --webdav-url and --local-path")
            }
        }
    }

    async fn handle_status() -> Result<()> {
        println!("{}", "Authentication Status:".cyan().bold());
        println!("{}", "─".repeat(40).bright_black());

        // Check Bandcamp cookie
        let bc_entry = Entry::new("bandcamp-sync", "bandcamp:cookie")?;
        match bc_entry.get_password() {
            Ok(stored) => {
                // Parse timestamp:cookie:fan_id format
                let parts: Vec<&str> = stored.splitn(3, ':').collect();
                if parts.len() >= 2 {
                    let timestamp_str = parts[0];
                    let has_fan_id = parts.len() == 3;
                    if let Ok(timestamp) = timestamp_str.parse::<i64>() {
                        let now = chrono::Utc::now().timestamp();
                        let age_seconds = now - timestamp;
                        let remaining = 600 - age_seconds;

                        let fan_id_status = if has_fan_id {
                            format!(" [fan_id: {}]", parts[2]).bright_black()
                        } else {
                            " [fan_id: missing]".red()
                        };

                        if remaining > 0 {
                            println!(
                                "{} {}{} (expires in {} seconds)",
                                "✓".green(),
                                "Bandcamp".green(),
                                fan_id_status,
                                remaining
                            );
                        } else {
                            println!(
                                "{} {}{} (expired {} seconds ago)",
                                "✗".yellow(),
                                "Bandcamp".yellow(),
                                fan_id_status,
                                -remaining
                            );
                        }
                    } else {
                        println!("{} {} (invalid timestamp)", "✗".red(), "Bandcamp".red());
                    }
                } else {
                    println!(
                        "{} {} (old format, please re-auth)",
                        "✗".yellow(),
                        "Bandcamp".yellow()
                    );
                }
            }
            Err(e) => {
                println!(
                    "{} {} - {}",
                    "✗".red(),
                    "Bandcamp".red(),
                    e.to_string().bright_black()
                );
            }
        }

        // Could check WebDAV creds here too
        println!();
        Ok(())
    }

    /// Handle import-zip command
    async fn handle_import_zip(
        zip_path: &std::path::Path,
        storage_options: &StorageOptions,
    ) -> Result<()> {
        use crate::utils::sanitize_filename;
        use std::io::Cursor;
        use zip::ZipArchive;

        // Verify zip file exists
        if !zip_path.exists() {
            anyhow::bail!("Zip file not found: {}", zip_path.display());
        }

        // Initialize storage backend
        let storage = Self::get_storage(
            storage_options.webdav_url.clone(),
            storage_options.local_path.clone(),
        )
        .await?;

        // Get Bandcamp collection to show user options
        let cookie = match AuthManager::get_bandcamp_cookie() {
            Ok(cookie) => cookie,
            Err(_) => {
                println!("{}", "No Bandcamp authentication found.".yellow());
                println!("Please run: {} auth bandcamp", "bandcamp-sync".cyan());
                anyhow::bail!("Authentication required to match zip to collection");
            }
        };

        let client = BandcampClient::new(cookie)?;
        let collection = client.fetch_collection().await?;

        // Show user the albums to choose from
        println!("\n{}", "Available albums in your collection:".bright_cyan());
        println!("{}", "─".repeat(60));

        for (idx, item) in collection.iter().enumerate() {
            if item.item_type == "album" {
                println!(
                    "{:3}. {} - {}",
                    idx + 1,
                    item.band_name.bright_yellow(),
                    item.item_title.bright_green()
                );
            }
        }

        // Ask user to select which album this zip is for
        println!("\n{}", "Which album is this zip file for?".bright_cyan());
        print!("Enter number (1-{}): ", collection.len());
        std::io::Write::flush(&mut std::io::stdout())?;

        let mut input = String::new();
        std::io::stdin().read_line(&mut input)?;
        let selection: usize = input.trim().parse().context("Invalid number")?;

        if selection == 0 || selection > collection.len() {
            anyhow::bail!("Invalid selection");
        }

        let selected_item = &collection[selection - 1];
        println!(
            "\n{} {} - {}",
            "Selected:".bright_green(),
            selected_item.band_name,
            selected_item.item_title
        );

        // Extract and import the zip
        println!(
            "\n{} {}",
            "Importing from:".bright_cyan(),
            zip_path.display()
        );

        // Read the zip file
        let zip_data = std::fs::read(zip_path)?;
        let cursor = Cursor::new(zip_data);
        let mut archive = ZipArchive::new(cursor)?;

        // Create the album directory in storage
        let artist = sanitize_filename(&selected_item.band_name);
        let album = sanitize_filename(&selected_item.item_title);
        let album_path = PathBuf::from(&artist).join(&album);

        storage.create_directory(&album_path).await?;
        println!("Created directory: {}/{}", artist, album);

        // Extract and upload each file
        let mut uploaded_count = 0;
        for i in 0..archive.len() {
            let mut file = archive.by_index(i)?;
            let file_name = file.name().to_string();

            // Skip directories and system files
            if file.is_dir() || file_name.starts_with("__MACOSX") || file_name.starts_with('.') {
                continue;
            }

            // Extract just the filename (not the full path)
            let output_name = std::path::Path::new(&file_name)
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or(&file_name)
                .to_string();

            // Read file contents
            let mut contents = Vec::new();
            std::io::Read::read_to_end(&mut file, &mut contents)?;

            // Upload to storage
            let dest_path = album_path.join(&output_name);
            storage.write_file(&dest_path, &contents).await?;

            println!("  ✓ Uploaded: {}", output_name);
            uploaded_count += 1;
        }

        println!(
            "\n{} Imported {} files to {}/{}",
            "✓".bright_green(),
            uploaded_count,
            artist,
            album
        );

        Ok(())
    }

    /// Helper function to lookup releases in MusicBrainz using both disc ID and TOC
    async fn lookup_releases_in_musicbrainz(
        mb_client: &crate::cd::MusicBrainzClient,
        disc_id: &str,
        toc: &crate::cd::models::CDToc,
    ) -> Result<Vec<crate::cd::CDAlbum>> {
        use tracing::debug;

        // Try disc ID lookup first
        let mut releases = match mb_client.lookup_by_disc_id(disc_id).await {
            Ok(r) => r,
            Err(e) => {
                debug!("Disc ID lookup error: {}", e);
                Vec::new()
            }
        };

        // If disc ID lookup fails, try TOC submission
        if releases.is_empty() {
            println!("Disc ID not found, trying TOC submission...");
            releases = match mb_client.lookup_by_toc(toc).await {
                Ok(r) => r,
                Err(e) => {
                    debug!("TOC lookup error: {}", e);
                    Vec::new()
                }
            };
        }

        Ok(releases)
    }

    async fn handle_query_cd(
        device: String,
        show_toc: bool,
        no_lookup: bool,
        manual_disc_number: Option<i32>,
    ) -> Result<()> {
        use crate::cd::models::CDToc;
        use crate::cd::{CDReader, MusicBrainzClient};
        use colored::Colorize;

        // Helper to generate MusicBrainz TOC submission string
        fn generate_mb_toc_string(toc: &CDToc) -> String {
            let mut parts = vec![
                toc.first_track.to_string(),
                toc.last_track.to_string(),
                toc.leadout_offset.to_string(),
            ];
            for offset in &toc.track_offsets {
                parts.push(offset.to_string());
            }
            parts.join("+")
        }

        println!("{}", "🔍 Querying CD information...".blue().bold());

        // Initialize CD reader
        let reader = if device == "auto" {
            println!("Auto-detecting CD device...");
            CDReader::auto_detect()?
        } else {
            CDReader::new(device.clone())
        };

        println!("Using device: {}\n", device.green());

        // Check if disc is present
        if !reader.has_disc().await? {
            anyhow::bail!("No disc found in drive");
        }

        // Read TOC
        println!("{}", "Reading Table of Contents...".yellow());
        let toc = reader.read_toc().await?;
        let disc_id = toc.calculate_disc_id();

        println!("✓ Disc ID: {}", disc_id.bright_cyan());
        println!("  Tracks: {} to {}", toc.first_track, toc.last_track);
        println!("  Leadout: {}", toc.leadout_offset);

        if show_toc {
            println!("\n{}", "Raw TOC Data:".underline());
            println!("  First track: {}", toc.first_track);
            println!("  Last track: {}", toc.last_track);
            println!("  Leadout offset: {}", toc.leadout_offset);
            println!("  Track offsets:");
            for (i, offset) in toc.track_offsets.iter().enumerate() {
                let track_num = i + 1;
                let seconds = offset / 75;
                let frames = offset % 75;
                let minutes = seconds / 60;
                let secs = seconds % 60;
                println!(
                    "    Track {:2}: {:7} ({}:{:02}.{:02})",
                    track_num, offset, minutes, secs, frames
                );
            }
        }

        // Try to read CD-TEXT
        println!("\n{}", "Checking for CD-TEXT...".yellow());
        if let Some(cd_text) = reader.read_cd_text().await? {
            println!("✓ CD-TEXT found:");
            if let Some(artist) = &cd_text.artist {
                println!("  Artist: {}", artist.bright_green());
            }
            if let Some(album) = &cd_text.album_title {
                println!("  Album: {}", album.bright_green());
            }
        } else {
            println!("  No CD-TEXT data found");
        }

        // MusicBrainz lookup
        if !no_lookup {
            println!("\n{}", "Looking up in MusicBrainz...".yellow());
            let mb_client = MusicBrainzClient::new();

            // Use shared lookup logic
            let mut releases =
                Self::lookup_releases_in_musicbrainz(&mb_client, &disc_id, &toc).await?;

            // Apply manual disc number override if provided
            if let Some(disc_num) = manual_disc_number {
                println!(
                    "{}",
                    format!(
                        "⚠ Manually overriding disc number to {} for all releases",
                        disc_num
                    )
                    .yellow()
                );
                for release in releases.iter_mut() {
                    release.disc_number = Some(disc_num);
                    if release.total_discs == Some(1) {
                        release.total_discs = Some(disc_num.max(2));
                    }
                }
            }

            if !releases.is_empty() {
                println!("✓ Found {} release(s) in MusicBrainz:", releases.len());

                for (i, release) in releases.iter().enumerate() {
                    let disc_info = match (release.disc_number, release.total_discs) {
                        (Some(disc), Some(total)) if total > 1 => {
                            format!(" [Disc {}/{}]", disc, total)
                        }
                        _ => String::new(),
                    };

                    println!(
                        "\n  {}. {} - {}{}",
                        i + 1,
                        release.artist.bright_cyan(),
                        release.album_title.bright_magenta(),
                        disc_info.bright_yellow()
                    );

                    if let Some(date) = &release.release_date {
                        println!("     Released: {}", date);
                    }
                    if let Some(label) = &release.label {
                        println!("     Label: {}", label);
                    }
                    if let Some(catalog) = &release.catalog_number {
                        println!("     Catalog: {}", catalog);
                    }
                    if !release.genres.is_empty() {
                        println!("     Genres: {}", release.genres.join(", "));
                    }
                    if release.cover_art_available {
                        println!("     Cover art: {}", "Available".green());
                    }

                    println!("     Tracks:");
                    for track in &release.tracks {
                        let duration_str = if track.duration > 0.0 {
                            let mins = (track.duration / 60.0) as u32;
                            let secs = (track.duration % 60.0) as u32;
                            format!(" ({}:{:02})", mins, secs)
                        } else {
                            String::new()
                        };

                        println!(
                            "       {:2}. {}{}",
                            track.track_num, track.title, duration_str
                        );
                    }
                }
            } else {
                println!(
                    "❌ No releases found in MusicBrainz for disc ID: {}",
                    disc_id
                );
                println!("\nPossible reasons:");
                println!("  • This CD is not in the MusicBrainz database");
                println!("  • Try submitting this disc to MusicBrainz at:");
                println!(
                    "    https://musicbrainz.org/cdtoc/attach?toc={}",
                    generate_mb_toc_string(&toc)
                );
            }
        }

        Ok(())
    }

    async fn handle_import_cd(
        device: String,
        webdav_url: Option<String>,
        local_path: Option<String>,
        format: AudioFormat,
        no_lookup: bool,
        just_cover: bool,
        manual_disc_number: Option<i32>,
    ) -> Result<()> {
        use crate::cd::models::CDTrack;
        use crate::cd::{CDReader, CDRipper, MusicBrainzClient};
        use dialoguer::{Confirm, Select, theme::ColorfulTheme};
        use std::io::{self, Write};

        println!("{}", "🎵 CD Import".bright_blue().bold());
        println!();

        // Check dependencies first
        CDRipper::check_dependencies().await?;

        // Setup storage backend
        let storage: Box<dyn StorageBackend> = if let Some(url) = webdav_url {
            let auth = AuthManager::get_webdav_auth(&url).await?;
            Box::new(WebDavStorage::new(&url, Some(auth.username), Some(auth.password)).await?)
        } else if let Some(path) = local_path {
            Box::new(LocalStorage::new(PathBuf::from(path))?)
        } else {
            anyhow::bail!("Either --webdav-url or --local-path must be specified");
        };

        // Initialize CD reader
        let reader = if device == "auto" {
            println!("Auto-detecting CD drive...");
            CDReader::auto_detect()?
        } else {
            CDReader::new(&device)
        };

        // Check if disc is present
        if !reader.has_disc().await? {
            anyhow::bail!("No disc found in drive. Please insert a CD and try again.");
        }

        println!("{}", "✓ CD detected".green());

        // Read TOC for first disc
        println!("Reading CD table of contents...");
        let toc = reader.read_toc().await?;
        let disc_id = toc.calculate_disc_id();

        println!("Disc ID: {}", disc_id.bright_cyan());
        println!("Tracks: {}", toc.last_track);
        println!();

        // Try to get CD-TEXT info first
        let cd_text = reader.read_cd_text().await?;

        // Prepare album metadata - will be used as template for multi-disc
        let base_album = if no_lookup {
            // Use CD-TEXT only or prompt for manual entry
            if let Some(text_info) = cd_text {
                use crate::cd::CDAlbum;
                CDAlbum {
                    disc_id: disc_id.clone(),
                    artist: text_info.artist.unwrap_or_else(|| {
                        dialoguer::Input::new()
                            .with_prompt("Artist name")
                            .interact_text()
                            .unwrap_or_else(|_| "Unknown Artist".to_string())
                    }),
                    album_title: text_info.album_title.unwrap_or_else(|| {
                        dialoguer::Input::new()
                            .with_prompt("Album title")
                            .interact_text()
                            .unwrap_or_else(|_| "Unknown Album".to_string())
                    }),
                    release_date: None,
                    label: None,
                    catalog_number: None,
                    barcode: None,
                    tracks: Vec::new(), // Will need to fill this
                    genres: Vec::new(),
                    total_duration: 0.0,
                    mb_release_id: None,
                    mb_release_group_id: None,
                    mb_artist_id: None,
                    cover_art_url: None,
                    cover_art_available: false,
                    disc_number: Some(1),
                    total_discs: Some(1),
                    media_format: "CD".to_string(),
                }
            } else {
                anyhow::bail!(
                    "No CD-TEXT found and MusicBrainz lookup disabled. Remove --no-lookup to search MusicBrainz."
                );
            }
        } else {
            // Look up in MusicBrainz
            println!("Looking up disc in MusicBrainz...");
            let mb_client = MusicBrainzClient::new();

            // Use shared lookup logic (disc ID first, then TOC)
            let mut releases =
                Self::lookup_releases_in_musicbrainz(&mb_client, &disc_id, &toc).await?;

            if releases.is_empty() {
                println!("{}", "No exact match found in MusicBrainz".yellow());

                // Try with CD-TEXT if available
                if let Some(text_info) = cd_text
                    && let (Some(artist), Some(album)) = (&text_info.artist, &text_info.album_title)
                {
                    println!("Searching by CD-TEXT: {} - {}", artist, album);
                    releases = mb_client
                        .search_by_metadata(artist, album, toc.last_track as usize)
                        .await?;
                }

                if releases.is_empty() {
                    anyhow::bail!("No releases found. Try using --no-lookup for manual entry.");
                }
            }

            // Always prompt user to select when multiple releases are found
            if releases.len() > 1 {
                println!("\n{} Multiple releases found:", "⚠".yellow());
                let items: Vec<String> = releases
                    .iter()
                    .enumerate()
                    .map(|(i, r)| {
                        let disc_info = match (r.disc_number, r.total_discs) {
                            (Some(disc), Some(total)) if total > 1 => {
                                format!(" [Disc {}/{}]", disc, total)
                            }
                            _ => String::new(),
                        };
                        format!(
                            "{}. {} - {}{} ({}, {})",
                            i + 1,
                            r.artist,
                            r.album_title,
                            disc_info,
                            r.release_date.as_deref().unwrap_or("unknown year"),
                            r.label.as_deref().unwrap_or("unknown label")
                        )
                    })
                    .collect();

                // Show all releases
                for item in &items {
                    println!("  {}", item);
                }
                println!();

                let selection = Select::with_theme(&ColorfulTheme::default())
                    .with_prompt("Select the correct release")
                    .items(&items)
                    .default(0)
                    .interact()?;

                releases[selection].clone()
            } else if releases.len() == 1 {
                println!(
                    "Found release: {} - {}",
                    releases[0].artist, releases[0].album_title
                );
                releases.into_iter().next().unwrap()
            } else {
                anyhow::bail!("No releases found in MusicBrainz")
            }
        };

        // Apply manual disc number override if provided for starting disc
        let mut base_album = base_album;
        let starting_disc = if let Some(disc_num) = manual_disc_number {
            println!(
                "\n{}",
                format!("⚠ Manually overriding starting disc number to {}", disc_num).yellow()
            );
            base_album.disc_number = Some(disc_num);
            // If we're manually setting disc number and total_discs is 1, assume it's a multi-disc
            if base_album.total_discs == Some(1) {
                println!("{}", "  Also setting total_discs to at least 2 (since you're overriding disc number)".yellow());
                base_album.total_discs = Some(disc_num.max(2));
            }
            disc_num
        } else {
            base_album.disc_number.unwrap_or(1)
        };

        // Store the MusicBrainz release ID for subsequent disc lookups
        let mb_release_id = base_album.mb_release_id.clone();

        // Check if this is a multi-disc release
        let total_discs = base_album.total_discs.unwrap_or(1);
        let is_multi_disc = total_discs > 1;

        if is_multi_disc {
            println!(
                "\n{}",
                format!("Multi-disc release detected: {} discs total", total_discs).bright_cyan()
            );
            println!("{}", "Will import each disc sequentially.".bright_cyan());
            println!(
                "{}",
                "Press 's' + ENTER at any disc prompt to skip remaining discs.".yellow()
            );
        }

        // Display base album info
        println!("\n{}", "Album Information:".bright_green());
        println!("  Artist: {}", base_album.artist.bright_white());
        println!("  Album:  {}", base_album.album_title.bright_white());
        if is_multi_disc {
            println!("  Total Discs: {}", total_discs);
        }
        if let Some(date) = &base_album.release_date {
            println!("  Year:   {}", date);
        }
        if let Some(label) = &base_album.label {
            println!("  Label:  {}", label);
        }
        println!();

        // Initial confirmation
        let prompt = if just_cover {
            "Download and upload cover art for all discs?".to_string()
        } else if is_multi_disc {
            format!("Proceed with ripping {} disc(s)?", total_discs)
        } else {
            "Proceed with ripping?".to_string()
        };

        if !Confirm::new()
            .with_prompt(&prompt)
            .default(true)
            .interact()?
        {
            println!("Cancelled.");
            return Ok(());
        }

        // Create ripper - convert CLI AudioFormat to storage AudioFormat
        let storage_format = match format {
            AudioFormat::Aac => crate::storage::AudioFormat::Aac,
            AudioFormat::Mp3 => crate::storage::AudioFormat::Mp3,
            AudioFormat::Flac => crate::storage::AudioFormat::Flac,
            AudioFormat::Wav => crate::storage::AudioFormat::Wav,
        };
        let ripper = CDRipper::new(
            if device == "auto" {
                "/dev/cdrom"
            } else {
                &device
            },
            storage_format,
        );

        // Process each disc in the multi-disc set
        let mut current_disc = starting_disc;
        let mut skip_remaining = false;
        let mut total_tracks_so_far = 0i32; // Track count from previous discs

        while current_disc <= total_discs as i32 && !skip_remaining {
            // For discs after the first, eject and wait for next disc
            if current_disc > starting_disc {
                println!("\n{}", "─".repeat(60).bright_black());
                println!("\n{}", "Ejecting disc...".yellow());
                if let Err(e) = reader.eject_disc().await {
                    println!(
                        "⚠ Failed to auto-eject: {}. Please remove disc manually.",
                        e
                    );
                }

                // Prompt for next disc with skip option
                println!(
                    "\n{}",
                    format!("Please insert Disc {} of {}", current_disc, total_discs).bright_cyan()
                );
                println!(
                    "Press {} when ready, or {} to skip remaining discs",
                    "ENTER".bright_green(),
                    "'s' + ENTER".bright_yellow()
                );

                // Read user input
                io::stdout().flush()?;
                let mut input = String::new();
                io::stdin().read_line(&mut input)?;

                if input.trim().to_lowercase() == "s" {
                    println!("Skipping remaining discs.");
                    skip_remaining = true;
                    continue;
                }

                // Wait for disc to be inserted
                println!("Waiting for disc...");
                reader.wait_for_disc().await?;

                // Read TOC of new disc
                println!("Reading new disc...");
                let new_toc = reader.read_toc().await?;
                let new_disc_id = new_toc.calculate_disc_id();
                println!("Disc ID: {}", new_disc_id.bright_cyan());

                // Look up this specific disc in MusicBrainz to get correct tracks
                if !no_lookup {
                    let mb_client = MusicBrainzClient::new();

                    // First try to get the specific disc from the known release
                    let mut disc_found = false;
                    if let Some(release_id) = &mb_release_id {
                        println!(
                            "Looking up disc {} in the selected release...",
                            current_disc
                        );
                        if let Ok(Some(disc_album)) = mb_client
                            .get_release_disc(release_id, &new_disc_id, current_disc)
                            .await
                        {
                            println!(
                                "✓ Found disc {} with {} tracks",
                                current_disc,
                                disc_album.tracks.len()
                            );
                            base_album.tracks = disc_album.tracks;
                            base_album.disc_id = new_disc_id.clone();
                            disc_found = true;
                        }
                    }

                    // If that fails, fall back to searching by disc ID and TOC
                    if !disc_found {
                        println!("Searching for disc by ID and TOC...");
                        let releases = Self::lookup_releases_in_musicbrainz(
                            &mb_client,
                            &new_disc_id,
                            &new_toc,
                        )
                        .await?;

                        // Find the release that matches our base album and has the right disc number
                        if let Some(matching_release) = releases.iter().find(|r| {
                            r.artist == base_album.artist
                                && r.album_title == base_album.album_title
                                && r.disc_number == Some(current_disc)
                        }) {
                            // Update tracks for current disc
                            base_album.tracks = matching_release.tracks.clone();
                            base_album.disc_id = new_disc_id.clone();
                            println!(
                                "✓ Found matching disc with {} tracks",
                                base_album.tracks.len()
                            );
                        } else {
                            println!("{}", "Warning: Could not find matching disc in MusicBrainz, using generic track names".yellow());
                            // Generate generic track names based on TOC
                            base_album.tracks.clear();
                            for i in 1..=new_toc.last_track {
                                base_album.tracks.push(CDTrack {
                                    track_num: i as i32,
                                    title: format!("Track {}", i),
                                    artist: None,
                                    duration: 0.0,
                                    isrc: None,
                                    mb_recording_id: None,
                                    start_offset: 0,
                                    end_offset: 0,
                                    pregap: None,
                                });
                            }
                        }
                    }
                }
            }

            // Update current disc number in album metadata
            let mut album = base_album.clone();
            album.disc_number = Some(current_disc);

            // Store original track numbers for cdparanoia and adjust display numbers for continuous numbering
            if current_disc > starting_disc && !album.tracks.is_empty() {
                for track in album.tracks.iter_mut() {
                    // Store the original disc track number in start_offset temporarily
                    track.start_offset = track.track_num;
                    // Adjust track number for continuous display
                    track.track_num += total_tracks_so_far;
                }
            } else {
                // For disc 1, just store original track numbers
                for track in album.tracks.iter_mut() {
                    track.start_offset = track.track_num;
                }
            }

            println!(
                "\n{}",
                format!("Processing Disc {} of {}", current_disc, total_discs).bright_green()
            );
            if !just_cover {
                if total_tracks_so_far > 0 {
                    println!(
                        "  Tracks: {} (numbered {}-{})",
                        album.tracks.len(),
                        total_tracks_so_far + 1,
                        total_tracks_so_far + album.tracks.len() as i32
                    );
                } else {
                    println!("  Tracks: {}", album.tracks.len());
                }
            }

            // Determine output directory
            let temp_dir = tempfile::tempdir()?;
            let rip_output = temp_dir.path();

            // Handle just_cover mode vs full rip
            let ripped_files = if just_cover {
                println!("\n{}", "Downloading cover art only...".bright_blue());

                // Create the album directory structure in temp (no Disc N subdirectory)
                let album_dir = rip_output
                    .join(sanitize_filename(&album.artist))
                    .join(sanitize_filename(&album.album_title));

                tokio::fs::create_dir_all(&album_dir).await?;

                // Download only the cover art (only for disc 1)
                let mut files = Vec::new();
                if current_disc == starting_disc && album.cover_art_url.is_some() {
                    if let Some(cover_url) = &album.cover_art_url {
                        println!("Found cover art URL: {}", cover_url);
                        match CDRipper::download_cover_art_static(cover_url, &album_dir).await {
                            Ok(cover_path) => {
                                println!("✓ Cover art downloaded");
                                files.push(cover_path);
                            }
                            Err(e) => {
                                println!("❌ Failed to download cover art: {}", e);
                            }
                        }
                    }
                } else if current_disc > starting_disc {
                    println!(
                        "Skipping cover art for disc {} (only downloading once)",
                        current_disc
                    );
                }
                files
            } else {
                // Full CD rip
                println!("\n{}", "Starting CD rip...".bright_blue());
                ripper.rip_cd(&album, rip_output).await?
            };

            // Upload to storage
            println!("\n{}", "Uploading to storage...".bright_blue());
            let artist_dir = PathBuf::from(sanitize_filename(&album.artist));
            let album_dir = artist_dir.join(sanitize_filename(&album.album_title));

            // Create album directory if it doesn't exist (same directory for all discs)
            storage.create_directory(&album_dir).await?;

            let pb = ProgressBar::new(ripped_files.len() as u64);
            pb.set_style(
                ProgressStyle::default_bar()
                    .template("{spinner:.green} [{bar:40.cyan/blue}] {pos}/{len} {msg}")
                    .unwrap()
                    .progress_chars("#>-"),
            );

            for file_path in &ripped_files {
                let file_name = file_path.file_name().unwrap().to_str().unwrap();

                // Skip cover art if it's not the first disc (already uploaded)
                if file_name == "folder.jpg" && current_disc > starting_disc {
                    pb.inc(1);
                    continue;
                }

                pb.set_message(format!("Uploading: {}", file_name));

                let contents = tokio::fs::read(file_path).await?;
                let dest_path = album_dir.join(file_name);

                storage.write_file(&dest_path, &contents).await?;

                if file_name == "folder.jpg" {
                    println!("  ✓ Uploaded cover art");
                }

                pb.inc(1);
            }

            pb.finish_with_message(format!("Disc {} complete!", current_disc));

            // Update total tracks count for next disc
            if !just_cover {
                total_tracks_so_far += album.tracks.len() as i32;
            }

            // Move to next disc
            current_disc += 1;
        }

        // Final summary
        if skip_remaining {
            println!(
                "\n{} Imported {} of {} discs for: {} - {}",
                "✓".bright_green(),
                current_disc - starting_disc,
                total_discs,
                base_album.artist,
                base_album.album_title
            );
        } else {
            println!(
                "\n{} Successfully imported all {} disc(s): {} - {}",
                "✓".bright_green(),
                total_discs,
                base_album.artist,
                base_album.album_title
            );
        }

        Ok(())
    }
}
