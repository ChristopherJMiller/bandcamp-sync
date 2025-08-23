//! Album and track downloading from Bandcamp

use anyhow::{Context, Result};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::path::Path;
use tracing::{debug, info, warn};

use crate::utils::sanitize_filename;
use image::ImageFormat;
use lofty::config::{ParseOptions, WriteOptions};
use lofty::prelude::*;
use lofty::probe::Probe;
use lofty::tag::{ItemKey, ItemValue, Tag, TagItem, TagType};

/// Album metadata from Bandcamp pages
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TralbumData {
    pub artist: Option<String>,
    pub album_title: Option<String>,
    pub item_type: String,
    pub item_id: i64,
    pub tracks: Option<Vec<TrackData>>,
    pub download_pref: Option<i32>,
    #[serde(rename = "freeDownloadPage")]
    pub free_download_page: Option<String>,
}

/// Individual track metadata
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrackData {
    pub track_num: Option<i32>,
    pub title: Option<String>,
    pub duration: Option<f64>,
    pub file: Option<TrackFile>,
}

/// Track file download URLs
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrackFile {
    #[serde(rename = "mp3-128")]
    pub mp3_128: Option<String>,
}

/// Manages album downloads from Bandcamp
pub struct DownloadManager {
    client: Client,
    cookie: String,
}

impl DownloadManager {
    pub fn new(cookie: String) -> Self {
        let client = Client::builder()
            .user_agent("Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36")
            .build()
            .unwrap();

        Self { client, cookie }
    }

    /// Download tracks directly from album page (like bandcamp-dl does)
    pub async fn download_tracks_from_album_page(
        &self,
        album_url: &str,
        output_dir: &Path,
    ) -> Result<Vec<String>> {
        debug!("Fetching album page for track downloads: {}", album_url);

        // Fetch the album page
        let response = self
            .client
            .get(album_url)
            .header("Cookie", format!("identity={}", self.cookie))
            .send()
            .await
            .context("Failed to fetch album page")?;

        if !response.status().is_success() {
            anyhow::bail!("Failed to fetch album page: {}", response.status());
        }

        let html = response.text().await?;

        // Extract TralbumData from the page - it's in a data-tralbum attribute
        let tralbum_regex = regex::Regex::new(r#"data-tralbum="([^"]+)""#)?;

        if let Some(captures) = tralbum_regex.captures(&html)
            && let Some(json_str) = captures.get(1)
        {
            // HTML-decode the JSON string
            let decoded = html_escape::decode_html_entities(json_str.as_str());

            // Parse the TralbumData
            let data: serde_json::Value =
                serde_json::from_str(&decoded).context("Failed to parse TralbumData")?;

            // Create output directory
            std::fs::create_dir_all(output_dir)?;

            let mut downloaded_files = Vec::new();

            // Download album art if available
            // Bandcamp stores art_id, not art_url - we need to construct the URL
            let art_url = if let Some(art_id) = data.get("art_id").and_then(|v| v.as_i64()) {
                // Use size 10 for good quality (roughly 1200x1200)
                Some(format!("https://f4.bcbits.com/img/a{:010}_{}.jpg", art_id, 10))
            } else if let Some(art_id) = data.get("current")
                .and_then(|c| c.get("art_id"))
                .and_then(|v| v.as_i64()) {
                // Sometimes art_id is in the current object
                Some(format!("https://f4.bcbits.com/img/a{:010}_{}.jpg", art_id, 10))
            } else {
                // Fallback: check if there's a direct art_url field (rare)
                data.get("art_url")
                    .and_then(|v| v.as_str())
                    .map(|url| {
                        if url.starts_with("http") {
                            url.to_string()
                        } else {
                            format!("https:{}", url)
                        }
                    })
            };

            if let Some(art_url) = art_url {
                info!("Downloading album art from: {}", art_url);
                let cover_path = output_dir.join("folder.jpg");
                
                match self.client.get(&art_url).send().await {
                    Ok(art_response) => {
                        if art_response.status().is_success() {
                            let bytes = art_response.bytes().await?;
                            
                            // Load the image and convert to JPEG if needed
                            match image::load_from_memory(&bytes) {
                                Ok(img) => {
                                    // Save as JPEG with reasonable quality
                                    img.save_with_format(&cover_path, ImageFormat::Jpeg)
                                        .context("Failed to save cover image as JPEG")?;
                                    info!("Successfully saved album cover as folder.jpg");
                                    downloaded_files.push("folder.jpg".to_string());
                                }
                                Err(e) => {
                                    // If image processing fails, try saving raw bytes
                                    // (in case it's already a valid JPEG)
                                    warn!("Could not process image with image crate, saving raw: {}", e);
                                    std::fs::write(&cover_path, bytes)?;
                                    info!("Saved raw image data as folder.jpg");
                                    downloaded_files.push("folder.jpg".to_string());
                                }
                            }
                        } else {
                            warn!("Failed to download album art, HTTP status: {}", art_response.status());
                        }
                    }
                    Err(e) => {
                        warn!("Failed to fetch album art: {}", e);
                    }
                }
            } else {
                warn!("No art_id or art_url found in album data");
                // Log what fields we do have for debugging
                if let Some(obj) = data.as_object() {
                    let keys: Vec<_> = obj.keys().take(10).cloned().collect();
                    debug!("Available fields in data: {:?}", keys);
                }
            }

            // Get the trackinfo array
            if let Some(trackinfo) = data.get("trackinfo").and_then(|v| v.as_array()) {
                for (index, track) in trackinfo.iter().enumerate() {
                    let track_num = index + 1;
                    let title = track.get("title")
                        .and_then(|v| v.as_str())
                        .unwrap_or("Unknown");

                    // Get the mp3-128 URL from the file object
                    if let Some(file) = track.get("file").and_then(|v| v.as_object())
                        && let Some(mp3_url) = file.get("mp3-128").and_then(|v| v.as_str())
                    {
                        // Construct full URL
                        let download_url = if mp3_url.starts_with("http") {
                            mp3_url.to_string()
                        } else {
                            format!("https:{}", mp3_url)
                        };

                        // Download the track
                        let filename =
                            format!("{:02} - {}.mp3", track_num, sanitize_filename(title));
                        let track_path = output_dir.join(&filename);

                        info!("Downloading track: {}", filename);

                        let track_response = self.client.get(&download_url).send().await?;

                        if track_response.status().is_success() {
                            let bytes = track_response.bytes().await?;
                            std::fs::write(&track_path, &bytes)?;
                            
                            // Tag the MP3 file with metadata
                            if let Err(e) = self.tag_audio_file(
                                &track_path,
                                &data,
                                track,
                                track_num as u32,
                                trackinfo.len() as u32,
                            ) {
                                warn!("Failed to tag audio file: {}", e);
                            }
                            
                            downloaded_files.push(filename);
                        } else {
                            warn!("Failed to download track: {}", title);
                        }
                    }
                }
            }

            return Ok(downloaded_files);
        }

        anyhow::bail!("Could not find TralbumData in album page")
    }

    /// Tag an audio file with metadata from Bandcamp
    fn tag_audio_file(
        &self,
        file_path: &Path,
        album_data: &serde_json::Value,
        track_data: &serde_json::Value,
        track_num: u32,
        total_tracks: u32,
    ) -> Result<()> {
        // Parse the audio file
        let mut tagged_file = Probe::open(file_path)?
            .options(ParseOptions::new().read_properties(false))
            .read()?;

        // Get or create the primary tag
        let tag = match tagged_file.primary_tag_mut() {
            Some(tag) => tag,
            None => {
                // Create a new ID3v2 tag for MP3 files
                tagged_file.insert_tag(Tag::new(TagType::Id3v2));
                tagged_file.primary_tag_mut()
                    .ok_or_else(|| anyhow::anyhow!("Failed to create tag"))?
            }
        };

        // Clear existing tags to ensure clean metadata
        tag.clear();

        // Extract metadata from the album data (safe access to avoid panics)
        let artist = album_data.get("artist")
            .and_then(|v| v.as_str())
            .or_else(|| album_data.get("albumArtist").and_then(|v| v.as_str()))
            .unwrap_or("Unknown Artist");
        
        let album = album_data.get("current")
            .and_then(|c| c.get("title"))
            .and_then(|v| v.as_str())
            .or_else(|| album_data.get("album_title").and_then(|v| v.as_str()))
            .or_else(|| album_data.get("current")
                .and_then(|c| c.get("album_title"))
                .and_then(|v| v.as_str()))
            .unwrap_or("Unknown Album");

        let track_title = track_data.get("title")
            .and_then(|v| v.as_str())
            .unwrap_or("Unknown Track");

        // Set basic metadata
        tag.set_artist(String::from(artist));
        tag.set_album(String::from(album));
        tag.set_title(String::from(track_title));
        tag.set_track(track_num);
        // Note: lofty doesn't have a direct set_total_tracks method
        // We'll add it as a custom TagItem for formats that support it
        tag.insert(TagItem::new(
            ItemKey::TrackTotal,
            ItemValue::Text(total_tracks.to_string())
        ));

        // Set album artist (important for compilation albums)
        tag.insert(TagItem::new(
            ItemKey::AlbumArtist,
            ItemValue::Text(String::from(artist))
        ));

        // Extract and set year if available
        if let Some(release_date) = album_data.get("album_release_date")
            .and_then(|v| v.as_str()) {
            // Try to parse year from date string
            if let Some(year) = release_date.split('-').next() {
                if let Ok(year_num) = year.parse::<u32>() {
                    tag.set_year(year_num);
                }
            }
        } else if let Some(year) = album_data.get("current")
            .and_then(|c| c.get("release_date"))
            .and_then(|v| v.as_i64()) {
            tag.set_year(year as u32);
        }

        // Extract and set genre/tags if available
        if let Some(tags) = album_data.get("tags")
            .and_then(|v| v.as_array()) {
            let genres: Vec<String> = tags
                .iter()
                .filter_map(|t| {
                    // Handle both string tags and object tags with "name" field
                    t.as_str()
                        .map(String::from)
                        .or_else(|| t.get("name").and_then(|n| n.as_str()).map(String::from))
                })
                .collect();
            
            if !genres.is_empty() {
                // For gonic multi-value support, join with semicolon
                let genre_string = genres.join(";");
                tag.set_genre(genre_string);
            }
        } else if let Some(genre) = album_data.get("genre")
            .and_then(|v| v.as_str()) {
            tag.set_genre(String::from(genre));
        }

        // Add comment with Bandcamp URL if available
        if let Some(url) = album_data.get("url")
            .and_then(|v| v.as_str()) {
            tag.set_comment(format!("Bandcamp: {}", url));
        } else if let Some(url) = album_data.get("current")
            .and_then(|c| c.get("url"))
            .and_then(|v| v.as_str()) {
            tag.set_comment(format!("Bandcamp: {}", url));
        }

        // Save the tags to the file
        tag.save_to_path(file_path, WriteOptions::default())?;
        
        debug!("Tagged {} - {} - {}", artist, album, track_title);
        
        Ok(())
    }
}
