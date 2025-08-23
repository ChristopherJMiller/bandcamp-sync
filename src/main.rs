//! Bandcamp Sync - Download and sync your Bandcamp music collection
//!
//! This tool provides automated downloading and syncing of your Bandcamp
//! purchases to local storage or WebDAV servers, with support for incremental
//! updates, parallel downloads, and flexible filtering options.

mod bandcamp;
mod cli;
mod storage;
mod utils;

use anyhow::Result;
use clap::Parser;
use tracing_subscriber::{EnvFilter, fmt, prelude::*};

#[tokio::main]
async fn main() -> Result<()> {
    let args = cli::Cli::parse();

    // Configure logging based on verbosity flag
    if args.verbose {
        tracing_subscriber::registry()
            .with(
                fmt::layer()
                    .with_target(false)
                    .with_thread_ids(false)
                    .with_thread_names(false)
                    .with_file(false)
                    .with_line_number(false)
                    .with_level(true),
            )
            .with(EnvFilter::new("debug"))
            .init();
    } else {
        tracing_subscriber::registry()
            .with(
                fmt::layer()
                    .with_target(false)
                    .with_thread_ids(false)
                    .with_thread_names(false)
                    .with_file(false)
                    .with_line_number(false)
                    .with_level(false)
                    .without_time(),
            )
            .with(EnvFilter::new("warn"))
            .init();
    }

    cli::commands::CommandHandler::handle(args.command).await?;

    Ok(())
}
