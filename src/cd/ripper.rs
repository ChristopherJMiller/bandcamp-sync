//! CD ripping using cdparanoia command and ffmpeg for transcoding

use anyhow::{Context, Result};
use indicatif::{ProgressBar, ProgressStyle};
use std::path::{Path, PathBuf};
use tracing::{debug, info, warn};

use super::models::{CDAlbum, CDTrack};
use crate::storage::AudioFormat;
use crate::utils::sanitize_filename;

/// CD ripper using cdparanoia and FFmpeg commands
pub struct CDRipper {
    device_path: String,
    output_format: AudioFormat,
}

impl CDRipper {
    pub fn new(device_path: impl Into<String>, output_format: AudioFormat) -> Self {
        Self {
            device_path: device_path.into(),
            output_format,
        }
    }

    /// Rip entire CD with metadata
    pub async fn rip_cd(&self, album: &CDAlbum, output_dir: &Path) -> Result<Vec<PathBuf>> {
        info!("Starting CD rip: {} - {}", album.artist, album.album_title);

        // Create output directory structure: Artist/Album/ (no subdirectories for multi-disc)
        let album_dir = output_dir
            .join(sanitize_filename(&album.artist))
            .join(sanitize_filename(&album.album_title));

        tokio::fs::create_dir_all(&album_dir)
            .await
            .context("Failed to create album directory")?;

        let mut ripped_files = Vec::new();

        // Download cover art if available
        if let Some(cover_url) = &album.cover_art_url {
            info!("Downloading cover art from: {}", cover_url);
            match Self::download_cover_art_static(cover_url, &album_dir).await {
                Ok(cover_path) => {
                    info!("Cover art downloaded successfully");
                    ripped_files.push(cover_path);
                }
                Err(e) => {
                    warn!("Failed to download cover art: {}", e);
                }
            }
        } else {
            warn!("No cover art URL available for this album");
        }

        // Setup progress bar
        let pb = ProgressBar::new(album.tracks.len() as u64);
        pb.set_style(
            ProgressStyle::default_bar()
                .template("{spinner:.green} [{bar:40.cyan/blue}] {pos}/{len} {msg}")
                .unwrap()
                .progress_chars("#>-"),
        );

        // Rip each track
        for track in album.tracks.iter() {
            pb.set_message(format!("Ripping: {}", track.title));

            let track_path = self.rip_track_native(track, album, &album_dir).await?;
            ripped_files.push(track_path);

            pb.inc(1);
        }

        pb.finish_with_message("CD rip complete!");

        info!("Successfully ripped {} tracks from CD", ripped_files.len());
        Ok(ripped_files)
    }

    /// Rip a single track using cdparanoia + ffmpeg-next transcoding
    async fn rip_track_native(
        &self,
        track: &CDTrack,
        album: &CDAlbum,
        output_dir: &Path,
    ) -> Result<PathBuf> {
        // Track numbers are already adjusted for multi-disc in the album metadata
        let filename = format!(
            "{:02} - {}.{}",
            track.track_num,
            sanitize_filename(&track.title),
            self.output_format.extension()
        );
        let output_path = output_dir.join(&filename);

        debug!(
            "Ripping track {} to: {}",
            track.track_num,
            output_path.display()
        );

        // Use original disc track number for cdparanoia (stored in start_offset)
        // This is the physical track number on the disc (1-n), not the continuous numbering
        let cdparanoia_track_num = if track.start_offset > 0 {
            track.start_offset
        } else {
            track.track_num
        };

        // Create a temporary WAV file first
        let temp_wav = output_dir.join(format!(".temp_track_{}.wav", track.track_num));

        // Step 1: Rip to WAV with cdparanoia
        let rip_output = tokio::process::Command::new("cdparanoia")
            .args([
                "-d",
                &self.device_path,
                &cdparanoia_track_num.to_string(),
                temp_wav.to_str().unwrap(),
            ])
            .output()
            .await
            .context("Failed to run cdparanoia")?;

        if !rip_output.status.success() {
            let stderr = String::from_utf8_lossy(&rip_output.stderr);
            if stderr.contains("Unable to open") {
                anyhow::bail!("CD drive not accessible or no disc present");
            }
            anyhow::bail!("cdparanoia failed: {}", stderr);
        }

        // Step 2: Transcode using ffmpeg-next
        self.transcode_audio(&temp_wav, &output_path).await?;

        // Clean up temp file
        let _ = tokio::fs::remove_file(&temp_wav).await;

        // Tag the file with metadata
        self.tag_audio_file(&output_path, track, album).await?;

        Ok(output_path)
    }

    /// Transcode audio file using ffmpeg-next
    async fn transcode_audio(&self, input_path: &Path, output_path: &Path) -> Result<()> {
        // For now, use a simpler command-based approach until we get the FFmpeg API working properly
        // The ffmpeg-next API is complex and has changed between versions
        let codec_args = match self.output_format {
            AudioFormat::Flac => vec!["-acodec", "flac", "-compression_level", "12"],
            AudioFormat::Mp3 => vec!["-acodec", "libmp3lame", "-ab", "320k"],
            AudioFormat::Aac => vec!["-acodec", "aac", "-ab", "256k"],
            AudioFormat::Wav => vec!["-acodec", "pcm_s16le"],
        };

        let mut ffmpeg_args = vec![
            "-i",
            input_path.to_str().unwrap(),
            "-y", // Overwrite output
        ];
        ffmpeg_args.extend_from_slice(&codec_args);
        ffmpeg_args.push(output_path.to_str().unwrap());

        let output = std::process::Command::new("ffmpeg")
            .args(&ffmpeg_args)
            .output()
            .context("Failed to run ffmpeg")?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            anyhow::bail!("FFmpeg conversion failed: {}", stderr);
        }

        Ok(())
    }

    /// Add metadata tags to audio file
    async fn tag_audio_file(
        &self,
        file_path: &Path,
        track: &CDTrack,
        album: &CDAlbum,
    ) -> Result<()> {
        use lofty::prelude::*;
        use lofty::probe::Probe;
        use lofty::tag::{Accessor, ItemKey};

        let mut tagged_file = Probe::open(file_path)?
            .options(lofty::config::ParseOptions::new().read_properties(false))
            .read()?;

        let tag = match tagged_file.primary_tag_mut() {
            Some(primary_tag) => primary_tag,
            None => {
                let tag_type = tagged_file.primary_tag_type();
                tagged_file.insert_tag(lofty::tag::Tag::new(tag_type));
                tagged_file.primary_tag_mut().unwrap()
            }
        };

        // Set standard tags (track numbers are already adjusted for multi-disc)
        tag.set_artist(album.artist.clone());
        tag.set_album(album.album_title.clone());
        tag.set_title(track.title.clone());
        tag.set_track(track.track_num as u32);

        // Set album artist for compilations
        if let Some(track_artist) = &track.artist {
            tag.set_artist(track_artist.clone());
            tag.insert_text(ItemKey::AlbumArtist, album.artist.clone());
        }

        // Add additional metadata
        if let Some(date) = &album.release_date {
            tag.set_year(
                date.split('-')
                    .next()
                    .and_then(|y| y.parse().ok())
                    .unwrap_or(0),
            );
        }

        if !album.genres.is_empty() {
            tag.set_genre(album.genres.join(", "));
        }

        if let Some(label) = &album.label {
            tag.insert_text(ItemKey::Publisher, label.clone());
        }

        if let Some(mb_id) = &album.mb_release_id {
            tag.insert_text(ItemKey::MusicBrainzReleaseId, mb_id.clone());
        }

        if let Some(mb_recording_id) = &track.mb_recording_id {
            tag.insert_text(ItemKey::MusicBrainzRecordingId, mb_recording_id.clone());
        }

        // Save tags
        tagged_file.save_to_path(file_path, lofty::config::WriteOptions::default())?;

        Ok(())
    }

    /// Download cover art (public static method so it can be used without ripping)
    pub async fn download_cover_art_static(url: &str, output_dir: &Path) -> Result<PathBuf> {
        let cover_path = output_dir.join("folder.jpg");

        debug!("Downloading cover art from: {}", url);

        let response = reqwest::get(url)
            .await
            .context("Failed to download cover art")?;

        if !response.status().is_success() {
            anyhow::bail!("Failed to download cover art: {}", response.status());
        }

        let bytes = response.bytes().await?;

        // Save cover art
        tokio::fs::write(&cover_path, &bytes).await?;

        info!("Downloaded cover art to: {}", cover_path.display());
        Ok(cover_path)
    }

    /// Check if required tools are installed
    pub async fn check_dependencies() -> Result<()> {
        let tools = ["cdparanoia", "ffmpeg"];
        let mut missing = Vec::new();

        for tool in &tools {
            let output = tokio::process::Command::new("which")
                .arg(tool)
                .output()
                .await;

            if output.is_err() || !output.unwrap().status.success() {
                missing.push(*tool);
            }
        }

        if !missing.is_empty() {
            anyhow::bail!(
                "Missing required tools: {}. Please install them first.\n\
                Install with: sudo apt install {} (Debian/Ubuntu) or brew install {} (macOS)",
                missing.join(", "),
                missing.join(" "),
                missing.join(" ")
            );
        }

        info!("All required tools found: cdparanoia and ffmpeg");
        Ok(())
    }
}
