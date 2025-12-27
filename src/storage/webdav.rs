use anyhow::{Context, Result};
use async_trait::async_trait;
use reqwest_dav::{Auth, Client, ClientBuilder, Depth};
use std::path::{Path, PathBuf};
use std::time::Duration;
use tracing::{debug, info, warn};

use super::{RetryConfig, StorageBackend, StorageItem};

pub struct WebDavStorage {
    client: Client,
    root_url: String,
    retry_config: RetryConfig,
}

impl WebDavStorage {
    pub async fn new(
        url: &str,
        username: Option<String>,
        password: Option<String>,
    ) -> Result<Self> {
        Self::with_retry_config(url, username, password, RetryConfig::default()).await
    }

    pub async fn with_retry_config(
        url: &str,
        username: Option<String>,
        password: Option<String>,
        retry_config: RetryConfig,
    ) -> Result<Self> {
        // Ensure URL is valid
        let _parsed =
            url::Url::parse(url).with_context(|| format!("Invalid WebDAV URL: {}", url))?;

        let auth = match (username, password) {
            (Some(user), Some(pass)) => Auth::Basic(user, pass),
            _ => Auth::Anonymous,
        };

        // Build the client with ClientBuilder
        let client = ClientBuilder::new()
            .set_host(url.to_string())
            .set_auth(auth)
            .build()
            .with_context(|| format!("Failed to create WebDAV client for {}", url))?;

        debug!("Initialized WebDavStorage with URL: {}", url);

        Ok(Self {
            client,
            root_url: url.to_string(),
            retry_config,
        })
    }

    fn make_path(&self, path: &Path) -> String {
        let path_str = path.to_str().unwrap_or("");
        if path_str.is_empty() {
            "/".to_string()
        } else {
            format!("/{}", path_str.trim_start_matches('/'))
        }
    }

    /// Check if an error is transient and worth retrying
    /// We retry on most errors EXCEPT clear permanent failures
    fn is_transient_error(err: &reqwest_dav::Error) -> bool {
        let err_str = format!("{:?}", err);

        // Don't retry on permanent client errors
        if err_str.contains("status: 401")
            || err_str.contains("status: 403")
            || err_str.contains("status: 404")
            || err_str.contains("Unauthorized")
            || err_str.contains("Forbidden")
        {
            return false;
        }

        // Retry on everything else - network issues, server errors, timeouts, etc.
        true
    }

    /// Calculate delay with jitter for retry attempt
    fn calculate_delay(&self, attempt: u32) -> Duration {
        use rand::Rng;

        let base_delay = self.retry_config.initial_delay_ms as f64
            * self.retry_config.backoff_factor.powi(attempt as i32 - 1);
        let capped_delay = base_delay.min(self.retry_config.max_delay_ms as f64);

        // Add ±10% jitter
        let jitter_range = capped_delay * 0.1;
        let jitter = rand::thread_rng().gen_range(-jitter_range..jitter_range);
        let final_delay = (capped_delay + jitter).max(0.0) as u64;

        Duration::from_millis(final_delay)
    }

    /// Execute a WebDAV PUT operation with retry
    async fn put_with_retry(&self, path: &str, data: Vec<u8>) -> Result<(), reqwest_dav::Error> {
        let mut last_error = None;

        for attempt in 1..=self.retry_config.max_attempts {
            match self.client.put(path, data.clone()).await {
                Ok(()) => return Ok(()),
                Err(e) => {
                    if attempt < self.retry_config.max_attempts && Self::is_transient_error(&e) {
                        let delay = self.calculate_delay(attempt);
                        warn!(
                            "WebDAV PUT {} failed (attempt {}/{}): {:?}. Retrying in {:?}...",
                            path, attempt, self.retry_config.max_attempts, e, delay
                        );
                        tokio::time::sleep(delay).await;
                        last_error = Some(e);
                    } else {
                        return Err(e);
                    }
                }
            }
        }

        Err(last_error.unwrap())
    }

    /// Execute a WebDAV MKCOL operation with retry
    async fn mkcol_with_retry(&self, path: &str) -> Result<(), reqwest_dav::Error> {
        let mut last_error = None;

        for attempt in 1..=self.retry_config.max_attempts {
            match self.client.mkcol(path).await {
                Ok(()) => return Ok(()),
                Err(e) => {
                    if attempt < self.retry_config.max_attempts && Self::is_transient_error(&e) {
                        let delay = self.calculate_delay(attempt);
                        warn!(
                            "WebDAV MKCOL {} failed (attempt {}/{}): {:?}. Retrying in {:?}...",
                            path, attempt, self.retry_config.max_attempts, e, delay
                        );
                        tokio::time::sleep(delay).await;
                        last_error = Some(e);
                    } else {
                        return Err(e);
                    }
                }
            }
        }

        Err(last_error.unwrap())
    }
}

#[async_trait]
impl StorageBackend for WebDavStorage {
    async fn list_directory(&self, path: &Path) -> Result<Vec<StorageItem>> {
        let webdav_path = self.make_path(path);
        debug!("Listing WebDAV directory: {}", webdav_path);

        // List with depth 1 to get immediate children
        let items = self
            .client
            .list(&webdav_path, Depth::Number(1))
            .await
            .with_context(|| format!("Failed to list WebDAV directory: {}", webdav_path))?;

        debug!(
            "WebDAV returned {} items for path {}",
            items.len(),
            webdav_path
        );

        let mut storage_items = Vec::new();

        // Process the list items
        // When listing root ("/"), all items are subdirectories
        // When listing a subdirectory, first item is the directory itself
        let skip_first = !path.as_os_str().is_empty();

        for (i, item) in items.into_iter().enumerate() {
            // Skip the directory itself when listing a subdirectory
            if skip_first && i == 0 {
                debug!("Skipping parent directory (first item in subdirectory listing)");
                continue;
            }

            // Since ListEntity is not public, we'll use debug output to determine type
            // This is a workaround - ideally reqwest_dav would export these types
            let item_debug = format!("{:?}", item);
            debug!("Processing item {}: {:?}", i, item_debug);
            let is_directory = item_debug.contains("Folder");

            // Extract href from debug output (not ideal but works)
            let href = if let Some(start) = item_debug.find("href: \"") {
                let start = start + 7;
                let end = item_debug[start..].find('"').unwrap_or(0) + start;
                item_debug[start..end].to_string()
            } else {
                continue;
            };

            // Extract the path from the href
            // The href might be a full URL or just a path
            let item_name = if href.starts_with("http") {
                // Parse URL and get the last path component
                let trimmed = href.trim_end_matches('/');
                trimmed.split('/').next_back().unwrap_or(&href).to_string()
            } else {
                // Just use the last component of the path
                let trimmed = href.trim_end_matches('/').trim_start_matches('/');
                trimmed
                    .split('/')
                    .next_back()
                    .unwrap_or(trimmed)
                    .to_string()
            };

            // URL decode the name
            let decoded_name = urlencoding::decode(&item_name)
                .map(|s| s.to_string())
                .unwrap_or_else(|_| item_name.clone());

            let item_path = if path.as_os_str().is_empty() {
                PathBuf::from(decoded_name)
            } else {
                path.join(decoded_name)
            };

            storage_items.push(StorageItem {
                path: item_path,
                is_directory,
                size: None,     // Can't extract from debug output easily
                modified: None, // Can't extract from debug output easily
            });
        }

        debug!("Found {} items in WebDAV directory", storage_items.len());
        Ok(storage_items)
    }

    async fn exists(&self, path: &Path) -> Result<bool> {
        let webdav_path = self.make_path(path);
        debug!("Checking WebDAV path exists: {}", webdav_path);

        // Try to list with depth 0 to check existence
        match self.client.list(&webdav_path, Depth::Number(0)).await {
            Ok(_) => Ok(true),
            Err(_) => Ok(false),
        }
    }

    async fn create_directory(&self, path: &Path) -> Result<()> {
        let webdav_path = self.make_path(path);
        debug!("Creating WebDAV directory: {}", webdav_path);

        // Create parent directories if needed
        let mut current = PathBuf::new();
        for component in path.components() {
            current.push(component);
            let current_path = self.make_path(&current);

            if !self.exists(&current).await? {
                self.mkcol_with_retry(&current_path)
                    .await
                    .with_context(|| {
                        format!("Failed to create WebDAV directory: {}", current_path)
                    })?;
            }
        }

        Ok(())
    }

    async fn write_file(&self, path: &Path, data: &[u8]) -> Result<()> {
        let webdav_path = self.make_path(path);
        debug!(
            "Writing WebDAV file: {} ({} bytes)",
            webdav_path,
            data.len()
        );

        // Ensure parent directory exists
        if let Some(parent) = path.parent()
            && !parent.as_os_str().is_empty()
        {
            self.create_directory(parent).await?;
        }

        self.put_with_retry(&webdav_path, data.to_vec())
            .await
            .with_context(|| format!("Failed to write WebDAV file: {}", webdav_path))?;

        info!("Wrote WebDAV file: {}", webdav_path);
        Ok(())
    }

    fn storage_type(&self) -> &str {
        "WebDAV"
    }

    fn root_path(&self) -> String {
        self.root_url.clone()
    }
}
