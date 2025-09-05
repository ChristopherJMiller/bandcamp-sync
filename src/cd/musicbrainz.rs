//! MusicBrainz API client for CD metadata lookup

use anyhow::{Context, Result};
use musicbrainz_rs::Browse;
use musicbrainz_rs::entity::discid::Discid;
use musicbrainz_rs::entity::release::Release;
use musicbrainz_rs::prelude::*;
use serde::Deserialize;
use tracing::{debug, info, warn};

use super::models::{CDAlbum, CDToc, CDTrack};

/// Cover Art Archive response structure
#[derive(Debug, Deserialize)]
struct CoverArtResponse {
    images: Vec<CoverArtImage>,
}

#[derive(Debug, Deserialize)]
struct CoverArtImage {
    front: bool,
    image: String,
}

/// Client for MusicBrainz metadata lookups
pub struct MusicBrainzClient;

impl MusicBrainzClient {
    pub fn new() -> Self {
        Self
    }

    /// Look up release by disc ID
    pub async fn lookup_by_disc_id(&self, disc_id: &str) -> Result<Vec<CDAlbum>> {
        info!("Looking up disc ID in MusicBrainz: {}", disc_id);

        // Fetch disc ID with releases
        let disc_result = Discid::fetch().id(disc_id).execute().await;

        let discid_response = match disc_result {
            Ok(discid) => discid,
            Err(e) => {
                debug!("Disc ID lookup failed: {}", e);
                return Ok(Vec::new());
            }
        };

        let releases = discid_response.releases.unwrap_or_default();

        let mut albums = Vec::new();

        for release in releases {
            debug!(
                "Found release: {} by {}",
                release.title.clone(),
                release
                    .artist_credit
                    .as_ref()
                    .and_then(|ac| ac.first())
                    .map(|a| a.name.clone())
                    .unwrap_or_else(|| "Unknown Artist".to_string())
            );

            // Fetch full release details with recordings
            let full_release = Release::fetch()
                .id(&release.id)
                .with_recordings()
                .with_artist_credits()
                .with_labels()
                .with_release_groups() // Need this for alternative cover art search
                .execute()
                .await
                .context("Failed to fetch release details")?;

            albums.push(
                self.convert_release_to_album(disc_id, full_release, None)
                    .await?,
            );
        }

        if albums.is_empty() {
            warn!("No releases found for disc ID: {}", disc_id);
        } else {
            info!("Found {} release(s) for disc ID", albums.len());
        }

        Ok(albums)
    }

    /// Get a specific disc/medium from a multi-disc release
    /// This is used when importing subsequent discs of a multi-disc album
    pub async fn get_release_disc(
        &self,
        release_id: &str,
        disc_id: &str,
        disc_number: i32,
    ) -> Result<Option<CDAlbum>> {
        info!("Fetching disc {} from release {}", disc_number, release_id);

        // Fetch the full release with all media/discs
        let release = Release::fetch()
            .id(release_id)
            .with_recordings()
            .with_artist_credits()
            .with_labels()
            .with_release_groups()
            .execute()
            .await
            .context("Failed to fetch release details")?;

        // Find the specific medium/disc and extract tracks first
        let mut disc_tracks = Vec::new();
        let mut found_disc = false;

        if let Some(media_list) = &release.media {
            let disc_index = (disc_number - 1) as usize;

            if let Some(medium) = media_list.get(disc_index) {
                info!(
                    "Found disc {} with {} tracks",
                    disc_number, medium.track_count
                );
                found_disc = true;

                // Extract tracks from the specific disc
                if let Some(track_list) = &medium.tracks {
                    for track in track_list {
                        let track_duration =
                            track.length.map(|ms| ms as f64 / 1000.0).unwrap_or(0.0);

                        disc_tracks.push(CDTrack {
                            track_num: track.position as i32,
                            title: track.title.clone(),
                            artist: track
                                .recording
                                .as_ref()
                                .and_then(|r| r.artist_credit.as_ref())
                                .and_then(|ac| ac.first())
                                .map(|a| a.name.clone()),
                            duration: track_duration,
                            isrc: track
                                .recording
                                .as_ref()
                                .and_then(|r| r.isrcs.as_ref())
                                .and_then(|i| i.first())
                                .cloned(),
                            mb_recording_id: track.recording.as_ref().map(|r| r.id.clone()),
                            start_offset: 0,
                            end_offset: 0,
                            pregap: None,
                        });
                    }
                }
            } else {
                warn!(
                    "Disc {} not found in release (only {} discs)",
                    disc_number,
                    media_list.len()
                );
            }
        }

        if found_disc {
            // Now convert the release to album (this consumes the release)
            let mut album = self
                .convert_release_to_album(disc_id, release, None)
                .await?;

            // Override with the specific disc's tracks
            album.tracks = disc_tracks;
            album.disc_number = Some(disc_number);
            album.disc_id = disc_id.to_string();

            Ok(Some(album))
        } else {
            Ok(None)
        }
    }

    /// Look up releases by submitting TOC data
    /// This works even when the disc ID is not in the database
    pub async fn lookup_by_toc(&self, toc: &CDToc) -> Result<Vec<CDAlbum>> {
        info!("Looking up releases by TOC submission");

        // Build the TOC string for submission
        let mut toc_parts = vec![
            toc.first_track.to_string(),
            toc.last_track.to_string(),
            toc.leadout_offset.to_string(),
        ];
        for offset in &toc.track_offsets {
            toc_parts.push(offset.to_string());
        }
        let toc_string = toc_parts.join(" ");

        debug!("TOC string: {}", toc_string);

        // Use the MusicBrainz web service to submit TOC
        // This endpoint returns releases that match the TOC
        let url = format!(
            "https://musicbrainz.org/ws/2/discid/-?toc={}&fmt=json",
            urlencoding::encode(&toc_string)
        );

        debug!("TOC lookup URL: {}", url);

        // MusicBrainz requires a user agent
        let client = reqwest::Client::builder()
            .user_agent("bandcamp-sync/0.3.0 (https://github.com/chris-miller/bandcamp-sync)")
            .build()?;

        let response = client.get(&url).send().await?;

        if !response.status().is_success() {
            debug!("TOC lookup failed with status: {}", response.status());
            return Ok(Vec::new());
        }

        // Get the response text
        let response_text = response.text().await?;
        debug!("TOC response length: {} bytes", response_text.len());

        #[derive(Deserialize, Debug)]
        struct SimplifiedRelease {
            id: String,
            title: String,
            country: Option<String>,
        }

        #[derive(Deserialize, Debug)]
        #[allow(dead_code)]
        struct TocResponse {
            releases: Option<Vec<SimplifiedRelease>>,
            #[serde(rename = "release-count")]
            release_count: Option<usize>,
        }

        let toc_response: TocResponse = match serde_json::from_str(&response_text) {
            Ok(resp) => resp,
            Err(e) => {
                warn!("Failed to parse TOC response: {}", e);
                debug!(
                    "Response preview: {}",
                    &response_text[..response_text.len().min(500)]
                );
                return Ok(Vec::new());
            }
        };

        let mut albums = Vec::new();
        let disc_id = toc.calculate_disc_id();

        let releases = toc_response.releases.unwrap_or_default();
        info!("Found {} releases via TOC submission", releases.len());

        for simple_release in releases {
            debug!(
                "Found release via TOC: {} - {} ({})",
                simple_release.id,
                simple_release.title,
                simple_release.country.as_deref().unwrap_or("unknown")
            );

            // Fetch full release details using the ID
            let full_release = Release::fetch()
                .id(&simple_release.id)
                .with_recordings()
                .with_artist_credits()
                .with_labels()
                .with_release_groups() // Need this for alternative cover art search
                .execute()
                .await?;

            // Debug: Check if release_group is present after fetch
            if let Some(rg) = &full_release.release_group {
                debug!(
                    "TOC release {} fetched with release_group: {}",
                    simple_release.id, rg.id
                );
            } else {
                warn!(
                    "TOC release {} fetched but NO release_group!",
                    simple_release.id
                );
            }

            albums.push(
                self.convert_release_to_album(&disc_id, full_release, Some(toc))
                    .await?,
            );
        }

        if albums.is_empty() {
            info!("No releases found via TOC submission");
        } else {
            info!("Found {} release(s) via TOC submission", albums.len());
        }

        Ok(albums)
    }

    /// Convert MusicBrainz Release to our CDAlbum format
    /// For multi-disc releases, matches the TOC to find the correct disc
    async fn convert_release_to_album(
        &self,
        disc_id: &str,
        release: Release,
        toc: Option<&CDToc>,
    ) -> Result<CDAlbum> {
        // Debug: Check if release_group is present
        if let Some(rg) = &release.release_group {
            debug!("Release {} has release_group: {}", release.id, rg.id);
        } else {
            debug!("Release {} has NO release_group", release.id);
        }

        let artist = release
            .artist_credit
            .as_ref()
            .and_then(|ac| ac.first())
            .map(|a| a.name.clone())
            .unwrap_or_else(|| "Unknown Artist".to_string());

        let album_title = release.title.clone();

        // Extract label info
        let (label, catalog_number) =
            if let Some(label_info) = release.label_info.as_ref().and_then(|li| li.first()) {
                (
                    label_info.label.as_ref().map(|l| l.name.clone()),
                    label_info.catalog_number.clone(),
                )
            } else {
                (None, None)
            };

        // Convert tracks - handle multi-disc releases
        let mut tracks = Vec::new();
        let mut total_duration = 0.0;
        let mut disc_number = 1i32;
        let mut total_discs = 1i32;

        if let Some(media_list) = &release.media {
            total_discs = media_list.len() as i32;

            // If we have a TOC and multiple discs, try to match the correct disc
            let mut matched_medium = None;
            if let Some(toc) = toc {
                if total_discs > 1 {
                    // Try to match by track count first
                    let toc_track_count = toc.track_offsets.len();
                    for (idx, medium) in media_list.iter().enumerate() {
                        if medium.track_count == toc_track_count as u32 {
                            matched_medium = Some((idx, medium));
                            disc_number = (idx + 1) as i32;
                            info!(
                                "Matched TOC to disc {} of {} by track count",
                                disc_number, total_discs
                            );
                            break;
                        }
                    }

                    if matched_medium.is_none() {
                        warn!(
                            "Could not match TOC to specific disc in multi-disc release, using disc 1"
                        );
                        matched_medium = media_list.first().map(|m| (0, m));
                    }
                } else {
                    matched_medium = media_list.first().map(|m| (0, m));
                }
            } else {
                // No TOC provided, just use first disc
                matched_medium = media_list.first().map(|m| (0, m));
            }

            // Extract tracks from the matched disc only
            if let Some((_, medium)) = matched_medium
                && let Some(track_list) = &medium.tracks
            {
                for track in track_list {
                    let track_duration = track.length.map(|ms| ms as f64 / 1000.0).unwrap_or(0.0);

                    total_duration += track_duration;

                    tracks.push(CDTrack {
                        track_num: track.position as i32,
                        title: track.title.clone(),
                        artist: track
                            .recording
                            .as_ref()
                            .and_then(|r| r.artist_credit.as_ref())
                            .and_then(|ac| ac.first())
                            .map(|a| a.name.clone()),
                        duration: track_duration,
                        isrc: track
                            .recording
                            .as_ref()
                            .and_then(|r| r.isrcs.as_ref())
                            .and_then(|i| i.first())
                            .cloned(),
                        mb_recording_id: track.recording.as_ref().map(|r| r.id.clone()),
                        start_offset: 0, // Will be filled from actual TOC
                        end_offset: 0,
                        pregap: None,
                    });
                }
            }
        }

        // Get cover art URL - first try this release, then try to find any release with cover art
        let (mut cover_art_url, mut cover_art_available) =
            self.get_cover_art_url(&release.id).await;

        // If no cover art, try to find it from other releases of the same album
        if cover_art_url.is_none() {
            info!(
                "No cover art for release {}, searching alternatives",
                release.id
            );
            if let Some(alt_cover_url) = self.find_alternative_cover_art(&release).await {
                info!("Found alternative cover art!");
                cover_art_url = Some(alt_cover_url);
                cover_art_available = true;
            } else {
                warn!("Could not find any alternative cover art");
            }
        }

        // Extract genres from tags
        let genres = release
            .tags
            .unwrap_or_default()
            .into_iter()
            .map(|tag| tag.name)
            .collect();

        Ok(CDAlbum {
            disc_id: disc_id.to_string(),
            artist,
            album_title,
            release_date: release.date.map(|d| d.0),
            label,
            catalog_number,
            barcode: release.barcode,
            tracks,
            genres,
            total_duration,
            mb_release_id: Some(release.id.clone()),
            mb_release_group_id: release.release_group.map(|rg| rg.id),
            mb_artist_id: release
                .artist_credit
                .and_then(|mut ac| ac.pop())
                .map(|a| a.artist.id),
            cover_art_url,
            cover_art_available,
            disc_number: Some(disc_number),
            total_discs: Some(total_discs),
            media_format: "CD".to_string(),
        })
    }

    /// Get cover art URL from Cover Art Archive
    async fn get_cover_art_url(&self, release_id: &str) -> (Option<String>, bool) {
        // Use direct HTTP request to Cover Art Archive
        // The API may redirect to archive.org, reqwest will follow redirects
        let url = format!("https://coverartarchive.org/release/{}", release_id);

        match reqwest::get(&url).await {
            Ok(response) => {
                // Check if we got a successful response (2xx) after following any redirects
                if response.status().is_success() {
                    // Parse the JSON response
                    if let Ok(coverart) = response.json::<CoverArtResponse>().await {
                        // Prefer front cover, fall back to first image
                        let image_url = coverart
                            .images
                            .iter()
                            .find(|img| img.front)
                            .or_else(|| coverart.images.first())
                            .map(|img| img.image.clone());

                        if image_url.is_some() {
                            debug!("Found cover art for release {}", release_id);
                            (image_url, true)
                        } else {
                            debug!("No images in cover art response for release {}", release_id);
                            (None, false)
                        }
                    } else {
                        debug!("Failed to parse cover art JSON for release {}", release_id);
                        (None, false)
                    }
                } else if response.status() == reqwest::StatusCode::NOT_FOUND {
                    debug!("No cover art exists for release {} (404)", release_id);
                    (None, false)
                } else {
                    debug!(
                        "Cover art request failed with status {} for release {}",
                        response.status(),
                        release_id
                    );
                    (None, false)
                }
            }
            Err(e) => {
                debug!("Error fetching cover art for release {}: {}", release_id, e);
                (None, false)
            }
        }
    }

    /// Try to find cover art from other releases in the same release group
    async fn find_alternative_cover_art(&self, release: &Release) -> Option<String> {
        info!(
            "Searching for alternative cover art for release {}",
            release.id
        );

        // First try release group if available
        if let Some(release_group) = &release.release_group {
            info!("Release has release group: {}", release_group.id);
            let release_group_id = release_group.id.clone();

            // Fetch the release group to get all releases
            // Note: We need to browse releases by release group, not search
            if let Ok(rg_releases) = Release::browse()
                .by_release_group(&release_group_id)
                .execute()
                .await
            {
                let release_count = rg_releases.entities.len();
                info!("Found {} releases in release group", release_count);

                // Try each release to find cover art
                for other_release in rg_releases.entities {
                    // Skip the current release
                    if other_release.id == release.id {
                        debug!("Skipping current release {}", other_release.id);
                        continue;
                    }

                    info!(
                        "Checking release {} - {} for cover art",
                        other_release.id, other_release.title
                    );

                    // Try to get cover art from this release
                    if let (Some(cover_url), true) = self.get_cover_art_url(&other_release.id).await
                    {
                        info!(
                            "✓ Found cover art in release: {} - {}",
                            other_release.title, other_release.id
                        );
                        return Some(cover_url);
                    } else {
                        debug!("No cover art in release {}", other_release.id);
                    }
                }
                warn!(
                    "No cover art found in any of {} release group releases",
                    release_count
                );
            } else {
                warn!(
                    "Failed to browse releases by release group {}",
                    release_group_id
                );
            }
        } else {
            warn!("Release has no release_group, cannot search for alternatives");
        }

        // Always try a broader search by artist and album as fallback
        info!("Trying broader search by artist and album name");
        if let Some(artist_credit) = &release.artist_credit
            && let Some(artist) = artist_credit.first()
        {
            // Search by artist and release title
            let broad_search = format!(
                "artist:\"{}\" AND release:\"{}\"",
                artist.name, release.title
            );

            info!("Searching with query: {}", broad_search);

            if let Ok(search_result) = Release::search(broad_search).execute().await {
                info!(
                    "Found {} alternative releases to check",
                    search_result.entities.len()
                );

                for other_release in search_result.entities.into_iter().take(20) {
                    // Skip the current release
                    if other_release.id == release.id {
                        continue;
                    }

                    info!(
                        "Checking alternative release: {} - {}",
                        other_release.title, other_release.id
                    );

                    if let (Some(cover_url), true) = self.get_cover_art_url(&other_release.id).await
                    {
                        info!(
                            "Found cover art in alternative release: {} - {}",
                            other_release.title, other_release.id
                        );
                        return Some(cover_url);
                    }
                }
            }
        }

        warn!("No cover art found in any related releases after checking all alternatives");
        None
    }

    /// Fallback: Search by artist and album name when disc ID fails
    pub async fn search_by_metadata(
        &self,
        artist: &str,
        album: &str,
        num_tracks: usize,
    ) -> Result<Vec<CDAlbum>> {
        info!("Searching MusicBrainz by metadata: {} - {}", artist, album);

        // Use direct search query
        let query = format!("artist:\"{}\" AND release:\"{}\"", artist, album);

        let search_result = Release::search(query)
            .execute()
            .await
            .context("Failed to search MusicBrainz by metadata")?;

        let releases = search_result.entities;

        let mut albums = Vec::new();

        for release in releases {
            // Fetch full details to check track count
            let full_release = Release::fetch()
                .id(&release.id)
                .with_recordings()
                .with_artist_credits()
                .with_release_groups() // Need this for alternative cover art search
                .execute()
                .await?;

            // Check if track count roughly matches
            let release_tracks = full_release
                .media
                .as_ref()
                .and_then(|m| m.first())
                .map(|m| m.track_count)
                .unwrap_or(0) as usize;

            if (release_tracks as i32 - num_tracks as i32).abs() <= 2 {
                albums.push(
                    self.convert_release_to_album("manual-search", full_release, None)
                        .await?,
                );
            }
        }

        Ok(albums)
    }
}
