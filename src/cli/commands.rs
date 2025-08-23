use anyhow::Result;
use colored::Colorize;
use indicatif::{ProgressBar, ProgressStyle};
use keyring::Entry;
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
            } => Self::handle_diff(webdav_url, local_path, missing_only, artist_filter, exclude_artist).await,
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
                };
                Self::handle_sync(options).await
            }
            Commands::Completion { shell } => {
                Self::handle_completion(shell);
                Ok(())
            }
            Commands::Status => Self::handle_status().await,
        }
    }

    async fn handle_auth(service: AuthService) -> Result<()> {
        match service {
            AuthService::Bandcamp {
                headless,
                username,
                password,
                cookie,
                force,
            } => {
                println!("{}", "Authenticating with Bandcamp...".blue());
                let _cookie =
                    AuthManager::authenticate_bandcamp(headless, username, password, cookie, force)
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
                parallel_downloads: 0,  // Not needed for scan
                skip_cover_art: false,
                audio_format: crate::storage::AudioFormat::Aac,
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
                parallel_downloads: 0,  // Not needed for scan
                skip_cover_art: false,
                audio_format: crate::storage::AudioFormat::Aac,
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
        let storage = Self::get_storage(options.storage.webdav_url, options.storage.local_path).await?;

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
            0  // Disabled
        } else if options.parallel > 0 {
            options.parallel.min(6)  // User specified, cap at 6
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
}
