mod cli;
mod bandcamp;
mod storage;
mod utils;

use anyhow::Result;
use clap::Parser;
use tracing_subscriber::{fmt, prelude::*, EnvFilter};

#[tokio::main]
async fn main() -> Result<()> {
    // Parse CLI arguments
    let args = cli::Cli::parse();

    // Set up logging - only show logs in verbose mode
    if args.verbose {
        tracing_subscriber::registry()
            .with(
                fmt::layer()
                    .with_target(false)
                    .with_thread_ids(false)
                    .with_thread_names(false)
                    .with_file(false)
                    .with_line_number(false)
                    .with_level(true)
            )
            .with(EnvFilter::new("debug"))
            .init();
    } else {
        // In normal mode, only show warnings and errors
        tracing_subscriber::registry()
            .with(
                fmt::layer()
                    .with_target(false)
                    .with_thread_ids(false)
                    .with_thread_names(false)
                    .with_file(false)
                    .with_line_number(false)
                    .with_level(false)
                    .without_time()
            )
            .with(EnvFilter::new("warn"))
            .init();
    }

    // Handle commands
    cli::commands::CommandHandler::handle(args.command).await?;

    Ok(())
}
