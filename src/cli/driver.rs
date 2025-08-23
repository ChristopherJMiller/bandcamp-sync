//! WebDriver lifecycle management
//!
//! Automatically starts and stops WebDriver processes (geckodriver, chromedriver, etc.)
//! to avoid manual driver management by users.

use anyhow::{Context, Result};
use std::process::{Child, Command, Stdio};
use std::time::Duration;
use tokio::time::sleep;
use tracing::{debug, info, warn};

use super::BrowserDriver;

/// Manages WebDriver process lifecycle
pub struct DriverManager {
    driver: BrowserDriver,
    port: u16,
    process: Option<Child>,
}

impl DriverManager {
    pub fn new(driver: BrowserDriver, port: Option<u16>) -> Self {
        let port = port.unwrap_or_else(|| driver.default_port());
        Self {
            driver,
            port,
            process: None,
        }
    }

    /// Check if a WebDriver is already running on the specified port
    pub async fn is_running(&self) -> bool {
        let url = format!("http://localhost:{}/status", self.port);

        match reqwest::get(&url).await {
            Ok(response) => {
                if response.status().is_success() {
                    debug!(
                        "{} is already running on port {}",
                        self.driver.driver_name(),
                        self.port
                    );
                    true
                } else {
                    false
                }
            }
            Err(_) => false,
        }
    }

    /// Start the WebDriver if it's not already running
    pub async fn ensure_running(&mut self) -> Result<()> {
        if self.is_running().await {
            info!(
                "{} is already running on port {}",
                self.driver.driver_name(),
                self.port
            );
            return Ok(());
        }

        info!(
            "Starting {} on port {}...",
            self.driver.driver_name(),
            self.port
        );
        self.start_driver()?;

        // Wait for driver to be ready
        self.wait_for_ready().await?;

        Ok(())
    }

    /// Start the WebDriver process
    fn start_driver(&mut self) -> Result<()> {
        let driver_name = self.driver.driver_name();

        // Check if the driver executable exists
        if !self.driver_exists() {
            anyhow::bail!(
                "{} not found. Please install it:\n  - Firefox: brew install geckodriver\n  - Chrome: brew install chromedriver\n  - Safari: safaridriver --enable",
                driver_name
            );
        }

        let mut cmd = Command::new(driver_name);

        // Add port argument based on driver type
        match self.driver {
            BrowserDriver::Chrome => {
                cmd.arg(format!("--port={}", self.port));
            }
            BrowserDriver::Firefox => {
                cmd.args(["--port", &self.port.to_string()]);
            }
            BrowserDriver::Safari => {
                cmd.args(["--port", &self.port.to_string()]);
                // Safari driver requires additional setup
                cmd.arg("--enable");
            }
        }

        // Start the process in the background
        let child = cmd
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .with_context(|| format!("Failed to start {}", driver_name))?;

        self.process = Some(child);
        debug!(
            "Started {} process with PID: {:?}",
            driver_name,
            self.process.as_ref().map(|p| p.id())
        );

        Ok(())
    }

    /// Check if the driver executable exists in PATH
    fn driver_exists(&self) -> bool {
        Command::new("which")
            .arg(self.driver.driver_name())
            .output()
            .map(|output| output.status.success())
            .unwrap_or(false)
    }

    /// Wait for the WebDriver to be ready
    async fn wait_for_ready(&self) -> Result<()> {
        let url = format!("http://localhost:{}/status", self.port);
        let max_attempts = 30; // 30 seconds timeout

        for attempt in 1..=max_attempts {
            if let Ok(response) = reqwest::get(&url).await
                && response.status().is_success()
            {
                info!(
                    "{} is ready on port {}",
                    self.driver.driver_name(),
                    self.port
                );
                return Ok(());
            }

            if attempt < max_attempts {
                debug!(
                    "Waiting for {} to start... ({}/{})",
                    self.driver.driver_name(),
                    attempt,
                    max_attempts
                );
                sleep(Duration::from_secs(1)).await;
            }
        }

        anyhow::bail!(
            "{} failed to start on port {} after {} seconds",
            self.driver.driver_name(),
            self.port,
            max_attempts
        )
    }

    /// Stop the WebDriver if we started it
    pub fn stop(&mut self) {
        if let Some(mut child) = self.process.take() {
            debug!("Stopping {} process", self.driver.driver_name());
            match child.kill() {
                Ok(_) => info!("{} stopped", self.driver.driver_name()),
                Err(e) => warn!("Failed to stop {}: {}", self.driver.driver_name(), e),
            }
        }
    }

    /// Get the URL for connecting to the WebDriver
    pub fn url(&self) -> String {
        format!("http://localhost:{}", self.port)
    }
}

impl Drop for DriverManager {
    fn drop(&mut self) {
        // Clean up the process if we started it
        self.stop();
    }
}

// Note: with_driver helper removed as it's not suitable for long-running browser sessions
// The DriverManager should be kept alive for the duration of WebDriver usage
