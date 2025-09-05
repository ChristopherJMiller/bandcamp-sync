//! CD device reader for extracting TOC and disc information

use anyhow::{Context, Result};
use std::path::Path;
use tracing::{debug, info};

use super::models::CDToc;

/// Reads CD information from physical drives
pub struct CDReader {
    device_path: String,
}

impl CDReader {
    /// Create a new CD reader for the specified device
    pub fn new(device_path: impl Into<String>) -> Self {
        Self {
            device_path: device_path.into(),
        }
    }

    /// Auto-detect CD device (tries common paths)
    pub fn auto_detect() -> Result<Self> {
        let common_paths = [
            "/dev/cdrom",
            "/dev/sr0",
            "/dev/sr1",
            "/dev/dvd",
            "/dev/cd0",   // BSD
            "/dev/disk1", // macOS
        ];

        for path in &common_paths {
            if Path::new(path).exists() {
                info!("Found CD device at: {}", path);
                return Ok(Self::new(*path));
            }
        }

        anyhow::bail!("No CD device found. Please specify device path manually.")
    }

    /// Read Table of Contents from CD
    pub async fn read_toc(&self) -> Result<CDToc> {
        info!("Reading TOC from device: {}", self.device_path);

        // Try cd-discid first (simplest)
        if let Ok(output) = tokio::process::Command::new("cd-discid")
            .arg(&self.device_path)
            .output()
            .await
            && output.status.success()
        {
            let stdout = String::from_utf8_lossy(&output.stdout);
            if let Ok(toc) = self.parse_discid_output(&stdout) {
                return Ok(toc);
            }
        }

        // Fallback to cdparanoia -Q
        let output = tokio::process::Command::new("cdparanoia")
            .args(["-d", &self.device_path, "-Q"])
            .output()
            .await
            .context("Failed to run cdparanoia. Please install cdparanoia.")?;

        // cdparanoia outputs to stderr for queries
        let stderr = String::from_utf8_lossy(&output.stderr);
        self.parse_cdparanoia_output(&stderr)
    }

    /// Check if a disc is present in the drive
    pub async fn has_disc(&self) -> Result<bool> {
        // Try cdparanoia quick check
        let output = tokio::process::Command::new("cdparanoia")
            .args(["-d", &self.device_path, "-Q"])
            .stderr(std::process::Stdio::piped())
            .output()
            .await;

        if let Ok(output) = output {
            let stderr = String::from_utf8_lossy(&output.stderr);
            // Check for disc presence indicators
            if stderr.contains("Table of contents") || stderr.contains("track") {
                return Ok(true);
            }
            if stderr.contains("Unable to open") || stderr.contains("No such") {
                return Ok(false);
            }
        }

        // Fallback: try to read TOC
        match self.read_toc().await {
            Ok(_) => Ok(true),
            Err(_) => Ok(false),
        }
    }

    /// Parse cd-discid output format
    fn parse_discid_output(&self, output: &str) -> Result<CDToc> {
        // cd-discid output format:
        // <disc-id> <num-tracks> <track-offset-1> ... <track-offset-n> <leadout-offset>
        let parts: Vec<&str> = output.split_whitespace().collect();

        if parts.len() < 3 {
            anyhow::bail!("Invalid cd-discid output format");
        }

        let num_tracks: u8 = parts[1]
            .parse()
            .context("Failed to parse number of tracks")?;

        let mut track_offsets = Vec::new();
        for i in 2..(2 + num_tracks as usize) {
            if i >= parts.len() {
                anyhow::bail!("Missing track offset in cd-discid output");
            }
            let offset: i32 = parts[i].parse().context("Failed to parse track offset")?;
            track_offsets.push(offset);
        }

        let leadout_offset: i32 = parts[parts.len() - 1]
            .parse()
            .context("Failed to parse leadout offset")?;

        Ok(CDToc {
            first_track: 1,
            last_track: num_tracks,
            leadout_offset,
            track_offsets,
        })
    }

    /// Eject the disc from the drive
    pub async fn eject_disc(&self) -> Result<()> {
        info!("Ejecting disc from: {}", self.device_path);

        // Try eject command first
        let output = tokio::process::Command::new("eject")
            .arg(&self.device_path)
            .output()
            .await;

        match output {
            Ok(output) if output.status.success() => {
                info!("Disc ejected successfully");
                Ok(())
            }
            _ => {
                // Fallback to cdrecord -eject
                let output = tokio::process::Command::new("cdrecord")
                    .args(["-eject", &format!("dev={}", self.device_path)])
                    .output()
                    .await;

                if let Ok(output) = output
                    && output.status.success()
                {
                    info!("Disc ejected successfully using cdrecord");
                    return Ok(());
                }

                anyhow::bail!("Failed to eject disc. You may need to eject manually.")
            }
        }
    }

    /// Wait for a new disc to be inserted
    pub async fn wait_for_disc(&self) -> Result<()> {
        use std::time::Duration;
        use tokio::time::sleep;

        info!("Waiting for disc to be inserted...");

        loop {
            if self.has_disc().await? {
                // Wait a moment for the disc to stabilize
                sleep(Duration::from_secs(2)).await;
                info!("Disc detected");
                return Ok(());
            }
            sleep(Duration::from_millis(500)).await;
        }
    }

    /// Get CD-TEXT information if available
    pub async fn read_cd_text(&self) -> Result<Option<CDTextInfo>> {
        debug!(
            "Attempting to read CD-TEXT from device: {}",
            self.device_path
        );

        // Use cdrdao to read CD-TEXT
        let output = tokio::process::Command::new("cdrdao")
            .args(["read-cddb", "--device", &self.device_path, "-"])
            .output()
            .await;

        match output {
            Ok(output) if output.status.success() => {
                let stdout = String::from_utf8_lossy(&output.stdout);
                Ok(self.parse_cdtext(&stdout))
            }
            _ => Ok(None),
        }
    }

    fn parse_cdtext(&self, output: &str) -> Option<CDTextInfo> {
        let mut info = CDTextInfo::default();

        for line in output.lines() {
            if line.starts_with("Title:") {
                info.album_title = Some(line.strip_prefix("Title:").unwrap().trim().to_string());
            } else if line.starts_with("Performer:") {
                info.artist = Some(line.strip_prefix("Performer:").unwrap().trim().to_string());
            }
        }

        if info.album_title.is_some() || info.artist.is_some() {
            Some(info)
        } else {
            None
        }
    }

    /// Parse cdparanoia -Q output
    fn parse_cdparanoia_output(&self, output: &str) -> Result<CDToc> {
        let mut track_offsets = Vec::new();
        let mut leadout_offset = 0i32;
        let mut first_track = 1u8;
        let mut last_track = 0u8;

        for line in output.lines() {
            let line = line.trim();

            // Parse track lines like: " 1.    18295 [04:04.45]   0 [00:00.00]    0 copy OK"
            if line.starts_with(|c: char| c.is_ascii_digit())
                && let Some(dot_pos) = line.find('.')
                && let Ok(track_num) = line[..dot_pos].trim().parse::<u8>()
            {
                // Extract offset (second number)
                let parts: Vec<&str> = line[dot_pos + 1..].split_whitespace().collect();
                if !parts.is_empty() {
                    // Skip the length field and get the offset
                    if parts.len() >= 3
                        && let Ok(offset) = parts[2].parse::<i32>()
                    {
                        track_offsets.push(offset);
                        if track_num == 1 {
                            first_track = track_num;
                        }
                        last_track = track_num;
                    }
                }
            }

            // Parse TOTAL line for leadout
            if line.starts_with("TOTAL") {
                let parts: Vec<&str> = line.split_whitespace().collect();
                if parts.len() >= 2
                    && let Ok(offset) = parts[1].parse::<i32>()
                {
                    leadout_offset = offset;
                }
            }
        }

        if track_offsets.is_empty() {
            anyhow::bail!("No tracks found in CD");
        }

        Ok(CDToc {
            first_track,
            last_track,
            leadout_offset,
            track_offsets,
        })
    }
}

#[derive(Debug, Default, Clone)]
pub struct CDTextInfo {
    pub album_title: Option<String>,
    pub artist: Option<String>,
}
