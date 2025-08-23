use anyhow::{Context, Result};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::path::Path;
use tracing::{debug, info, warn};

/// Represents the data needed to download an album
#[derive(Debug, Clone)]
pub struct AlbumDownload {
    pub item_id: i64,
    pub item_type: String,
    pub item_url: String,
    pub artist: String,
    pub album: String,
    pub sale_item_id: Option<i64>,
    pub sale_item_type: Option<String>,
}

/// The TralbumData embedded in album pages
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TralbumData {
    pub artist: Option<String>,
    pub album_title: Option<String>,
    pub item_type: String,
    pub item_id: i64,
    pub tracks: Option<Vec<TrackData>>,
    pub download_pref: Option<i32>,
    pub freeDownloadPage: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrackData {
    pub track_num: Option<i32>,
    pub title: Option<String>,
    pub duration: Option<f64>,
    pub file: Option<TrackFile>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrackFile {
    #[serde(rename = "mp3-128")]
    pub mp3_128: Option<String>,
}

/// Downloads manager for fetching albums from Bandcamp
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
    
    /// Fetch album page and extract TralbumData
    pub async fn fetch_album_data(&self, album_url: &str) -> Result<TralbumData> {
        debug!("Fetching album page: {}", album_url);
        
        let response = self.client
            .get(album_url)
            .header("Cookie", format!("identity={}", self.cookie))
            .send()
            .await
            .context("Failed to fetch album page")?;
        
        if !response.status().is_success() {
            anyhow::bail!("Failed to fetch album page: {}", response.status());
        }
        
        let html = response.text().await?;
        
        // Extract TralbumData from the page
        // It's embedded as: var TralbumData = { ... };
        // Use a more robust regex that handles nested objects
        let tralbum_regex = regex::Regex::new(r"var TralbumData = (\{[\s\S]*?\});[\s]*\n")?;
        
        if let Some(captures) = tralbum_regex.captures(&html) {
            if let Some(json_str) = captures.get(1) {
                let data: TralbumData = serde_json::from_str(json_str.as_str())
                    .context("Failed to parse TralbumData")?;
                return Ok(data);
            }
        }
        
        anyhow::bail!("Could not find TralbumData in album page")
    }
    
    /// Download tracks directly from album page (like bandcamp-dl does)
    pub async fn download_tracks_from_album_page(&self, album_url: &str, output_dir: &Path) -> Result<Vec<String>> {
        debug!("Fetching album page for track downloads: {}", album_url);
        
        // Fetch the album page
        let response = self.client
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
        
        if let Some(captures) = tralbum_regex.captures(&html) {
            if let Some(json_str) = captures.get(1) {
                // HTML-decode the JSON string
                let decoded = html_escape::decode_html_entities(json_str.as_str());
                
                // Parse the TralbumData
                let data: serde_json::Value = serde_json::from_str(&decoded)
                    .context("Failed to parse TralbumData")?;
                
                // Create output directory
                std::fs::create_dir_all(output_dir)?;
                
                let mut downloaded_files = Vec::new();
                
                // Download album art if available
                if let Some(art_url) = data["art_url"].as_str() {
                    let art_url = if art_url.starts_with("http") {
                        art_url.to_string()
                    } else {
                        format!("https:{}", art_url)
                    };
                    
                    let cover_path = output_dir.join("cover.jpg");
                    let art_response = self.client.get(&art_url).send().await?;
                    if art_response.status().is_success() {
                        let bytes = art_response.bytes().await?;
                        std::fs::write(&cover_path, bytes)?;
                        downloaded_files.push("cover.jpg".to_string());
                    }
                }
                
                // Get the trackinfo array
                if let Some(trackinfo) = data["trackinfo"].as_array() {
                    for (index, track) in trackinfo.iter().enumerate() {
                        let track_num = index + 1;
                        let title = track["title"].as_str().unwrap_or("Unknown");
                        
                        // Get the mp3-128 URL from the file object
                        if let Some(file) = track["file"].as_object() {
                            if let Some(mp3_url) = file["mp3-128"].as_str() {
                                // Construct full URL
                                let download_url = if mp3_url.starts_with("http") {
                                    mp3_url.to_string()
                                } else {
                                    format!("https:{}", mp3_url)
                                };
                                
                                // Download the track
                                let filename = format!("{:02} - {}.mp3", track_num, sanitize_filename(title));
                                let track_path = output_dir.join(&filename);
                                
                                info!("Downloading track: {}", filename);
                                
                                let track_response = self.client
                                    .get(&download_url)
                                    .send()
                                    .await?;
                                
                                if track_response.status().is_success() {
                                    let bytes = track_response.bytes().await?;
                                    std::fs::write(&track_path, bytes)?;
                                    downloaded_files.push(filename);
                                } else {
                                    warn!("Failed to download track: {}", title);
                                }
                            }
                        }
                    }
                }
                
                return Ok(downloaded_files);
            }
        }
        
        anyhow::bail!("Could not find TralbumData in album page")
    }
    
    /// Download an album to a file
    pub async fn download_album(&self, download_url: &str, output_path: &Path) -> Result<()> {
        info!("Downloading album to: {:?}", output_path);
        
        let response = self.client
            .get(download_url)
            .header("Cookie", format!("identity={}", self.cookie))
            .send()
            .await
            .context("Failed to start download")?;
        
        if !response.status().is_success() {
            anyhow::bail!("Download failed: {}", response.status());
        }
        
        // Get the content length if available
        let content_length = response.content_length();
        debug!("Download size: {:?} bytes", content_length);
        
        // Stream the download to a file
        let bytes = response.bytes().await
            .context("Failed to download album")?;
        
        // Ensure parent directory exists
        if let Some(parent) = output_path.parent() {
            std::fs::create_dir_all(parent)
                .context("Failed to create output directory")?;
        }
        
        // Write to file
        std::fs::write(output_path, bytes)
            .context("Failed to write album file")?;
        
        info!("Download complete: {:?}", output_path);
        Ok(())
    }
    
    /// Extract a zip file to a directory
    pub async fn extract_album(&self, zip_path: &Path, output_dir: &Path) -> Result<Vec<String>> {
        debug!("Extracting {:?} to {:?}", zip_path, output_dir);
        
        // Create output directory
        std::fs::create_dir_all(output_dir)
            .context("Failed to create output directory")?;
        
        // Open the zip file
        let file = std::fs::File::open(zip_path)
            .context("Failed to open zip file")?;
        
        let mut archive = zip::ZipArchive::new(file)
            .context("Failed to read zip archive")?;
        
        let mut extracted_files = Vec::new();
        
        // Extract all files
        for i in 0..archive.len() {
            let mut file = archive.by_index(i)?;
            let file_name = file.name().to_string();
            
            // Skip directories
            if file.is_dir() {
                continue;
            }
            
            let output_path = output_dir.join(&file_name);
            
            // Create parent directories if needed
            if let Some(parent) = output_path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            
            // Extract file
            let mut output_file = std::fs::File::create(&output_path)
                .context("Failed to create output file")?;
            
            std::io::copy(&mut file, &mut output_file)
                .context("Failed to extract file")?;
            
            extracted_files.push(file_name);
        }
        
        debug!("Extracted {} files", extracted_files.len());
        Ok(extracted_files)
    }
}

/// Sanitize filename for safe filesystem usage
fn sanitize_filename(name: &str) -> String {
    name.chars()
        .map(|c| match c {
            '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' => '_',
            _ => c,
        })
        .collect()
}