use anyhow::{Context, Result};
use async_trait::async_trait;
use reqwest_dav::{Client, ClientBuilder, Auth, Depth};
use std::path::{Path, PathBuf};
use tracing::{debug, info};

use super::{StorageBackend, StorageItem};

pub struct WebDavStorage {
    client: Client,
    root_url: String,
}

impl WebDavStorage {
    pub async fn new(url: &str, username: Option<String>, password: Option<String>) -> Result<Self> {
        // Ensure URL is valid
        let _parsed = url::Url::parse(url)
            .with_context(|| format!("Invalid WebDAV URL: {}", url))?;
        
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
        })
    }
    
    fn make_url(&self, path: &Path) -> String {
        let path_str = path.to_str().unwrap_or("");
        if path_str.is_empty() || path_str == "/" {
            self.root_url.clone()
        } else {
            let clean_path = path_str.trim_start_matches('/');
            format!("{}/{}", self.root_url.trim_end_matches('/'), clean_path)
        }
    }
    
    fn make_path(&self, path: &Path) -> String {
        let path_str = path.to_str().unwrap_or("");
        if path_str.is_empty() {
            "/".to_string()
        } else {
            format!("/{}", path_str.trim_start_matches('/'))
        }
    }
}

#[async_trait]
impl StorageBackend for WebDavStorage {
    async fn list_directory(&self, path: &Path) -> Result<Vec<StorageItem>> {
        let webdav_path = self.make_path(path);
        debug!("Listing WebDAV directory: {}", webdav_path);
        
        // List with depth 1 to get immediate children
        let items = self.client.list(&webdav_path, Depth::Number(1)).await
            .with_context(|| format!("Failed to list WebDAV directory: {}", webdav_path))?;
        
        debug!("WebDAV returned {} items for path {}", items.len(), webdav_path);
        
        let mut storage_items = Vec::new();
        
        // Process the list items
        for (i, item) in items.into_iter().enumerate() {
            // Skip the first item which is usually the directory itself
            if i == 0 {
                debug!("Skipping first item (directory itself)");
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
                trimmed.split('/').last().unwrap_or(&href).to_string()
            } else {
                // Just use the last component of the path  
                let trimmed = href.trim_end_matches('/').trim_start_matches('/');
                trimmed.split('/').last().unwrap_or(trimmed).to_string()
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
                size: None, // Can't extract from debug output easily
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
                self.client.mkcol(&current_path).await
                    .with_context(|| format!("Failed to create WebDAV directory: {}", current_path))?;
            }
        }
        
        Ok(())
    }
    
    async fn write_file(&self, path: &Path, data: &[u8]) -> Result<()> {
        let webdav_path = self.make_path(path);
        debug!("Writing WebDAV file: {} ({} bytes)", webdav_path, data.len());
        
        // Ensure parent directory exists
        if let Some(parent) = path.parent() {
            if !parent.as_os_str().is_empty() {
                self.create_directory(parent).await?;
            }
        }
        
        self.client.put(&webdav_path, data.to_vec()).await
            .with_context(|| format!("Failed to write WebDAV file: {}", webdav_path))?;
        
        info!("Wrote WebDAV file: {}", webdav_path);
        Ok(())
    }
    
    async fn read_file(&self, path: &Path) -> Result<Vec<u8>> {
        let webdav_path = self.make_path(path);
        debug!("Reading WebDAV file: {}", webdav_path);
        
        let response = self.client.get(&webdav_path).await
            .with_context(|| format!("Failed to read WebDAV file: {}", webdav_path))?;
        
        // Extract bytes from response
        let data = response.bytes().await
            .with_context(|| format!("Failed to read response body for: {}", webdav_path))?;
        
        Ok(data.to_vec())
    }
    
    async fn delete_file(&self, path: &Path) -> Result<()> {
        let webdav_path = self.make_path(path);
        debug!("Deleting WebDAV file: {}", webdav_path);
        
        self.client.delete(&webdav_path).await
            .with_context(|| format!("Failed to delete WebDAV file: {}", webdav_path))?;
        
        Ok(())
    }
    
    fn storage_type(&self) -> &str {
        "WebDAV"
    }
    
    fn root_path(&self) -> String {
        self.root_url.clone()
    }
}