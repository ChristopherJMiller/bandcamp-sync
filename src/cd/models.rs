//! CD metadata models and structures

use serde::{Deserialize, Serialize};

/// CD album metadata from physical disc and MusicBrainz
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CDAlbum {
    pub disc_id: String,
    pub artist: String,
    pub album_title: String,
    pub release_date: Option<String>,
    pub label: Option<String>,
    pub catalog_number: Option<String>,
    pub barcode: Option<String>,
    pub tracks: Vec<CDTrack>,
    pub genres: Vec<String>,
    pub total_duration: f64,

    // MusicBrainz identifiers
    pub mb_release_id: Option<String>,
    pub mb_release_group_id: Option<String>,
    pub mb_artist_id: Option<String>,

    // Cover art
    pub cover_art_url: Option<String>,
    pub cover_art_available: bool,

    // Physical media info
    pub disc_number: Option<i32>,
    pub total_discs: Option<i32>,
    pub media_format: String, // "CD", "CD-R", etc
}

/// Individual track metadata from CD
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CDTrack {
    pub track_num: i32,
    pub title: String,
    pub artist: Option<String>, // For compilations/various artists
    pub duration: f64,          // Duration in seconds
    pub isrc: Option<String>,
    pub mb_recording_id: Option<String>,

    // CD-specific info
    pub start_offset: i32,   // Start frame offset on disc
    pub end_offset: i32,     // End frame offset on disc
    pub pregap: Option<f64>, // Pregap duration if any
}

/// Table of Contents from physical CD
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CDToc {
    pub first_track: u8,
    pub last_track: u8,
    pub leadout_offset: i32,
    pub track_offsets: Vec<i32>,
}

impl CDToc {
    /// Calculate MusicBrainz disc ID from TOC
    /// Based on: https://musicbrainz.org/doc/Disc_ID_Calculation
    pub fn calculate_disc_id(&self) -> String {
        use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
        use sha1::{Digest, Sha1};

        let mut hasher = Sha1::new();

        // First byte: first track number (hex)
        hasher.update([self.first_track]);

        // Second byte: last track number (hex)
        hasher.update([self.last_track]);

        // Next 4 bytes: leadout offset
        let leadout_bytes = (self.leadout_offset as u32).to_be_bytes();
        hasher.update(leadout_bytes);

        // For each track up to 99: 4 bytes offset
        for i in 0..99 {
            if i < self.track_offsets.len() {
                let offset_bytes = (self.track_offsets[i] as u32).to_be_bytes();
                hasher.update(offset_bytes);
            } else {
                hasher.update([0u8; 4]);
            }
        }

        let result = hasher.finalize();
        URL_SAFE_NO_PAD
            .encode(&result[..])
            .replace('_', ".")
            .replace('-', "_")
    }
}

/// Result from MusicBrainz lookup
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MusicBrainzRelease {
    pub id: String,
    pub title: String,
    pub artist: String,
    pub date: Option<String>,
    pub label: Option<String>,
    pub catalog_number: Option<String>,
    pub barcode: Option<String>,
    pub country: Option<String>,
    pub status: Option<String>,
    pub disambiguation: Option<String>,
    pub tracks: Vec<MusicBrainzTrack>,
    pub cover_art_archive: CoverArtArchive,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MusicBrainzTrack {
    pub position: i32,
    pub title: String,
    pub length: Option<i32>, // Duration in milliseconds
    pub recording_id: String,
    pub artist_credit: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CoverArtArchive {
    pub artwork: bool,
    pub count: i32,
    pub front: bool,
    pub back: bool,
}

/// CD ripping progress info
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RipProgress {
    pub current_track: i32,
    pub total_tracks: i32,
    pub track_progress: f32,   // 0.0 to 1.0
    pub overall_progress: f32, // 0.0 to 1.0
    pub status: String,
}
