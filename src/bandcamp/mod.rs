//! Bandcamp API client and download functionality
//!
//! Handles fetching collection data and downloading music files from Bandcamp.

pub mod client;
pub mod download;
pub mod models;

pub use client::BandcampClient;
pub use download::DownloadManager;
