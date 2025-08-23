pub mod auth;
pub mod commands;
pub mod completions;

use clap::{Parser, Subcommand, ValueEnum};
use clap_complete::Shell;

#[derive(Parser, Debug)]
#[command(
    name = "bandcamp-sync",
    about = "Sync Bandcamp purchases to WebDAV music library",
    version,
    author
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Commands,

    /// Verbose output
    #[arg(short, long, global = true)]
    pub verbose: bool,
}

#[derive(Subcommand, Debug)]
pub enum Commands {
    /// Authenticate with services
    Auth {
        #[command(subcommand)]
        service: AuthService,
    },

    /// List Bandcamp collection
    List {
        /// Filter by artist name (include only matching artists)
        #[arg(long, conflicts_with = "exclude_artist")]
        artist_filter: Option<String>,
        
        /// Exclude artists matching this pattern
        #[arg(long, conflicts_with = "artist_filter")]
        exclude_artist: Option<String>,

        /// Filter by album name
        #[arg(long)]
        album_filter: Option<String>,

        /// Output format
        #[arg(short, long, default_value = "table")]
        format: OutputFormat,
    },

    /// Scan destination library
    Scan {
        /// WebDAV URL (mutually exclusive with --local-path)
        #[arg(long, env = "WEBDAV_URL", conflicts_with = "local_path")]
        webdav_url: Option<String>,

        /// Local folder path (mutually exclusive with --webdav-url)
        #[arg(long, env = "LOCAL_PATH", conflicts_with = "webdav_url")]
        local_path: Option<String>,

        /// Show detailed information
        #[arg(short, long)]
        detailed: bool,
    },

    /// Show differences between Bandcamp and destination
    Diff {
        /// WebDAV URL (mutually exclusive with --local-path)
        #[arg(long, env = "WEBDAV_URL", conflicts_with = "local_path")]
        webdav_url: Option<String>,

        /// Local folder path (mutually exclusive with --webdav-url)
        #[arg(long, env = "LOCAL_PATH", conflicts_with = "webdav_url")]
        local_path: Option<String>,

        /// Show only missing albums
        #[arg(long)]
        missing_only: bool,
        
        /// Filter by artist name (include only matching artists)
        #[arg(long, conflicts_with = "exclude_artist")]
        artist_filter: Option<String>,
        
        /// Exclude artists matching this pattern
        #[arg(long, conflicts_with = "artist_filter")]
        exclude_artist: Option<String>,
    },

    /// Sync missing albums from Bandcamp to destination
    Sync {
        /// WebDAV URL (mutually exclusive with --local-path)
        #[arg(long, env = "WEBDAV_URL", conflicts_with = "local_path")]
        webdav_url: Option<String>,

        /// Local folder path (mutually exclusive with --webdav-url)
        #[arg(long, env = "LOCAL_PATH", conflicts_with = "webdav_url")]
        local_path: Option<String>,

        /// Dry run - show what would be synced without doing it
        #[arg(long)]
        dry_run: bool,

        /// Preferred audio format
        #[arg(short, long, default_value = "aac")]
        format: AudioFormat,

        /// Number of parallel downloads (0 = auto, max 6)
        #[arg(short, long, default_value = "0")]
        parallel: usize,

        /// Disable parallel downloads (sequential mode)
        #[arg(long, conflicts_with = "parallel")]
        no_parallel: bool,

        /// Skip album cover art
        #[arg(long)]
        no_cover: bool,

        /// Filter by artist name (include only matching artists)
        #[arg(long, conflicts_with = "exclude_artist")]
        artist_filter: Option<String>,
        
        /// Exclude artists matching this pattern
        #[arg(long, conflicts_with = "artist_filter")]
        exclude_artist: Option<String>,

        /// Filter by album name
        #[arg(long)]
        album_filter: Option<String>,
    },

    /// Generate shell completions
    Completion {
        /// Shell to generate completions for
        shell: Shell,
    },

    /// Check authentication status (debug)
    Status,
}

#[derive(Subcommand, Debug)]
pub enum AuthService {
    /// Authenticate with Bandcamp (opens browser for login)
    Bandcamp {
        /// Use headless browser
        #[arg(long)]
        headless: bool,

        /// Bandcamp username/email
        #[arg(short, long, env = "BANDCAMP_USER")]
        username: Option<String>,

        /// Bandcamp password
        #[arg(short, long, env = "BANDCAMP_PASS")]
        password: Option<String>,

        /// Skip browser login and use provided cookie directly
        #[arg(long, env = "BANDCAMP_COOKIE")]
        cookie: Option<String>,

        /// Force re-authentication even if valid cookie exists
        #[arg(long)]
        force: bool,
    },

    /// Authenticate with WebDAV
    Webdav {
        /// WebDAV URL
        #[arg(long, env = "WEBDAV_URL")]
        url: String,

        /// Username
        #[arg(short, long, env = "WEBDAV_USER")]
        username: Option<String>,

        /// Password
        #[arg(short, long, env = "WEBDAV_PASS")]
        password: Option<String>,
    },
}

#[derive(ValueEnum, Clone, Debug)]
pub enum AudioFormat {
    Aac,
    Mp3,
    Flac,
    Wav,
}

#[derive(ValueEnum, Clone, Debug)]
pub enum OutputFormat {
    Table,
    Json,
    Csv,
}
