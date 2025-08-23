use anyhow::{Context, Result};
use colored::Colorize;
use dialoguer::{Input, Password};
use keyring::Entry;
use std::time::Duration;
use thirtyfour::prelude::*;
use tracing::{debug, info};

use super::BrowserDriver;

const KEYRING_SERVICE: &str = "bandcamp-sync";

pub struct AuthManager;

impl AuthManager {
    pub async fn authenticate_bandcamp(
        headless: bool,
        driver: BrowserDriver,
        driver_port: Option<u16>,
        username: Option<String>,
        password: Option<String>,
        cookie: Option<String>,
        force: bool,
    ) -> Result<String> {
        // If cookie is provided directly, use it
        if let Some(cookie) = cookie {
            debug!("Using provided Bandcamp cookie");
            Self::store_bandcamp_cookie(&cookie)?;
            return Ok(cookie);
        }

        // Check keyring for existing cookie (unless forced)
        if !force {
            if let Ok(stored_cookie) = Self::get_bandcamp_cookie() {
                debug!("Found existing Bandcamp authentication in keyring");
                return Ok(stored_cookie);
            }
        } else {
            debug!("Force flag set, skipping keyring check");
        }

        // Launch browser for login
        debug!("Launching browser for Bandcamp login...");
        let cookie = Self::browser_login(driver, driver_port, username, password, headless).await?;

        // Store in keyring
        debug!("Storing cookie in keyring...");
        Self::store_bandcamp_cookie(&cookie)?;
        debug!("Bandcamp authentication successful - cookie saved");

        Ok(cookie)
    }

    pub async fn authenticate_webdav(
        url: &str,
        username: Option<String>,
        password: Option<String>,
    ) -> Result<(String, String)> {
        // Parse URL to get host for keyring key
        let parsed = url::Url::parse(url).context("Invalid WebDAV URL")?;
        let host = parsed
            .host_str()
            .ok_or_else(|| anyhow::anyhow!("No host in URL"))?;

        // Check keyring first using just the host
        if let (Ok(stored_user), Ok(stored_pass)) = (
            Self::get_webdav_username(host),
            Self::get_webdav_password(host),
        ) {
            info!("Found existing WebDAV credentials in keyring for {}", host);
            return Ok((stored_user, stored_pass));
        }

        info!("No stored credentials found for {}", host);

        // Get credentials
        let username = username.unwrap_or_else(|| {
            Input::new()
                .with_prompt("WebDAV username")
                .interact_text()
                .expect("Failed to read username")
        });

        let password = password.unwrap_or_else(|| {
            Password::new()
                .with_prompt("WebDAV password")
                .interact()
                .expect("Failed to read password")
        });

        // Verify credentials work
        Self::verify_webdav_credentials(url, &username, &password).await?;

        // Store in keyring using just the host
        Self::store_webdav_credentials(host, &username, &password)?;
        info!(
            "WebDAV authentication successful - credentials stored for {}",
            host
        );

        Ok((username, password))
    }

    async fn browser_login(
        browser: BrowserDriver,
        driver_port: Option<u16>,
        username: Option<String>,
        password: Option<String>,
        headless: bool,
    ) -> Result<String> {
        // Get the port to use
        let port = driver_port.unwrap_or_else(|| browser.default_port());
        
        // Connect to WebDriver - create driver differently based on browser type
        let url = format!("http://localhost:{}", port);
        debug!("Connecting to {} on port {}", browser.driver_name(), port);
        
        let driver = match browser {
            BrowserDriver::Chrome => {
                let mut caps = DesiredCapabilities::chrome();
                if headless {
                    caps.add_arg("--headless")?;
                }
                caps.add_arg("--disable-blink-features=AutomationControlled")?;
                caps.add_arg(
                    "--user-agent=Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36",
                )?;
                WebDriver::new(&url, caps).await
            }
            BrowserDriver::Firefox => {
                let mut caps = DesiredCapabilities::firefox();
                if headless {
                    caps.add_arg("-headless")?;
                }
                WebDriver::new(&url, caps).await
            }
            BrowserDriver::Safari => {
                let caps = DesiredCapabilities::safari();
                WebDriver::new(&url, caps).await
            }
        }
        .with_context(|| format!(
            "Failed to connect to {}. Please run: {} --port={}",
            browser.driver_name(),
            browser.driver_name(),
            port
        ))?;

        // Navigate to Bandcamp login
        driver
            .goto("https://bandcamp.com/login")
            .await
            .context("Failed to navigate to login page")?;

        // Wait for page to load
        tokio::time::sleep(Duration::from_secs(2)).await;

        // Only fill in credentials if provided via flags
        if let Some(username) = username
            && let Ok(username_field) = driver.find(By::Id("username-field")).await {
                username_field.send_keys(&username).await?;
                debug!("Filled in username");
            }

        if let Some(password) = password
            && let Ok(password_field) = driver.find(By::Id("password-field")).await {
                password_field.send_keys(&password).await?;
                debug!("Filled in password");
            }

        // Let user handle login
        println!();
        println!(
            "{}",
            "═══════════════════════════════════════════════════════".cyan()
        );
        println!(
            "{}",
            "Browser opened to Bandcamp login page.".green().bold()
        );
        println!("{}", "Please:".yellow());
        println!("{}", "1. Enter your credentials".yellow());
        println!("{}", "2. Solve the reCAPTCHA if present".yellow());
        println!("{}", "3. Click the 'Log in' button".yellow());
        println!(
            "{}",
            "═══════════════════════════════════════════════════════".cyan()
        );
        println!();
        println!("{}", "Waiting for login to complete...".blue());

        // Poll for successful login by checking for identity cookie
        let mut attempts = 0;
        let max_attempts = 120; // 2 minutes timeout
        
        // Pre-compile regexes used in the loop
        let fan_id_regex = regex::Regex::new(r#""fanId":(\d+)"#)?;
        let app_data_regex = regex::Regex::new(r#"<div[^>]+id="DiscoverApp"[^>]+data-blob="([^"]+)""#)?;

        loop {
            tokio::time::sleep(Duration::from_secs(1)).await;

            // Check if we have the identity cookie
            let cookies = driver.get_all_cookies().await?;
            if let Some(identity_cookie) = cookies.iter().find(|c| c.name == "identity") {
                let cookie_value = identity_cookie.value.to_string();

                // Check if we're already on the discover page after login redirect
                let current_url = driver.current_url().await?;
                debug!("Current URL after login: {}", current_url);
                
                if !current_url.as_str().contains("/discover") {
                    // Navigate to discover page if we're not already there
                    debug!("Navigating to /discover page for fan_id extraction");
                    driver.goto("https://bandcamp.com/discover").await?;
                    tokio::time::sleep(Duration::from_secs(2)).await;
                } else {
                    debug!("Already on /discover page, skipping navigation");
                    // Just a small delay to ensure page is fully loaded
                    tokio::time::sleep(Duration::from_millis(500)).await;
                }

                // Get page source to extract fan_id
                let page_source = driver.source().await?;

                // Extract fan_id from the page
                let fan_id = if let Some(captures) = fan_id_regex.captures(&page_source)
                {
                    captures.get(1).map(|m| m.as_str().to_string())
                } else {
                    None
                };

                if fan_id.is_none() {
                    // Try to get it from the DiscoverApp data
                    debug!("Trying to extract fan_id from DiscoverApp data");
                    let fan_id = if let Some(captures) = app_data_regex.captures(&page_source) {
                        if let Some(data_blob) = captures.get(1) {
                            let decoded = html_escape::decode_html_entities(data_blob.as_str());
                            // Reuse the same regex from above
                            if let Some(fan_captures) = fan_id_regex.captures(&decoded)
                            {
                                fan_captures.get(1).map(|m| m.as_str().to_string())
                            } else {
                                None
                            }
                        } else {
                            None
                        }
                    } else {
                        None
                    };

                    // Close browser and fail if still no fan_id
                    if fan_id.is_none() {
                        driver.quit().await?;
                        anyhow::bail!(
                            "Failed to extract fan_id from page after login. The page structure may have changed."
                        );
                    }

                    println!("{}", "✓ Login successful!".green().bold());
                    debug!("Successfully extracted cookie and fan_id: {:?}", fan_id);
                    // Store both cookie and fan_id together
                    return Ok(format!("{}:{}", cookie_value, fan_id.unwrap()));
                } else {
                    // Close browser
                    driver.quit().await?;

                    println!("{}", "✓ Login successful!".green().bold());
                    debug!(
                        "Successfully extracted cookie and fan_id: {}",
                        fan_id.as_ref().unwrap()
                    );
                    // Store both cookie and fan_id together
                    return Ok(format!("{}:{}", cookie_value, fan_id.unwrap()));
                }
            }

            attempts += 1;
            if attempts >= max_attempts {
                driver.quit().await?;
                anyhow::bail!("Login timeout - no identity cookie found after 2 minutes");
            }

            // Show progress
            if attempts % 10 == 0 {
                debug!("Still waiting for login... ({}/{})", attempts, max_attempts);
            }
        }
    }

    async fn verify_webdav_credentials(url: &str, username: &str, password: &str) -> Result<()> {
        // Simple PROPFIND request to verify credentials
        let client = reqwest::Client::new();
        let response = client
            .request(reqwest::Method::from_bytes(b"PROPFIND")?, url)
            .basic_auth(username, Some(password))
            .header("Depth", "0")
            .send()
            .await
            .context("Failed to connect to WebDAV server")?;

        if response.status().is_success() || response.status() == 207 {
            Ok(())
        } else if response.status() == 401 {
            anyhow::bail!("Invalid WebDAV credentials")
        } else {
            anyhow::bail!("WebDAV server returned status: {}", response.status())
        }
    }

    // Keyring helpers
    pub fn get_bandcamp_cookie() -> Result<String> {
        let entry = Entry::new(KEYRING_SERVICE, "bandcamp:cookie")?;
        let stored = entry
            .get_password()
            .context("No Bandcamp cookie in keyring")?;

        // Parse stored format: "timestamp:cookie" or "timestamp:cookie:fan_id"
        let parts: Vec<&str> = stored.splitn(3, ':').collect();

        if parts.len() >= 2
            && let Ok(timestamp) = parts[0].parse::<i64>() {
                let now = chrono::Utc::now().timestamp();
                let age_seconds = now - timestamp;

                // Expire after 10 minutes (600 seconds)
                if age_seconds > 600 {
                    debug!(
                        "Cookie expired (age: {}s), need to re-authenticate",
                        age_seconds
                    );
                    // Delete expired cookie
                    let _ = entry.delete_credential();
                    anyhow::bail!("Cookie expired, please re-authenticate");
                }

                debug!("Cookie is still valid (age: {}s)", age_seconds);

                // Return cookie:fan_id if we have fan_id, otherwise just cookie
                if parts.len() == 3 {
                    return Ok(format!("{}:{}", parts[1], parts[2])); // cookie:fan_id
                } else {
                    return Ok(parts[1].to_string()); // just cookie
                }
            }

        // Invalid format, treat as expired
        let _ = entry.delete_credential();
        anyhow::bail!("Cookie format invalid, please re-authenticate")
    }

    fn store_bandcamp_cookie(cookie: &str) -> Result<()> {
        let entry = Entry::new(KEYRING_SERVICE, "bandcamp:cookie")?;
        // Store with timestamp
        // Format: timestamp:cookie:fan_id (if fan_id is included in cookie)
        let timestamp = chrono::Utc::now().timestamp();
        let stored_value = format!("{}:{}", timestamp, cookie);
        entry
            .set_password(&stored_value)
            .context("Failed to store cookie in keyring")?;
        debug!("Cookie stored in keyring with 10 minute expiry");
        Ok(())
    }

    fn get_webdav_username(url: &str) -> Result<String> {
        let key = format!("webdav:{}:username", url);
        let entry = Entry::new(KEYRING_SERVICE, &key)?;
        entry
            .get_password()
            .context("No WebDAV username in keyring")
    }

    fn get_webdav_password(url: &str) -> Result<String> {
        let key = format!("webdav:{}:password", url);
        let entry = Entry::new(KEYRING_SERVICE, &key)?;
        entry
            .get_password()
            .context("No WebDAV password in keyring")
    }

    fn store_webdav_credentials(url: &str, username: &str, password: &str) -> Result<()> {
        let user_key = format!("webdav:{}:username", url);
        let pass_key = format!("webdav:{}:password", url);

        let user_entry = Entry::new(KEYRING_SERVICE, &user_key)?;
        user_entry.set_password(username)?;

        let pass_entry = Entry::new(KEYRING_SERVICE, &pass_key)?;
        pass_entry.set_password(password)?;

        Ok(())
    }
}
