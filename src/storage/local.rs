use anyhow::{Context, Result};
use async_trait::async_trait;
use std::path::{Path, PathBuf};
use tokio::fs;
use tokio::io::AsyncWriteExt;
use tracing::{debug, info};

use super::{StorageBackend, StorageItem};

pub struct LocalStorage {
    root_path: PathBuf,
}

impl LocalStorage {
    pub fn new(root_path: impl AsRef<Path>) -> Result<Self> {
        let root_path = root_path.as_ref().to_path_buf();
        
        // Ensure the path is absolute
        let root_path = if root_path.is_absolute() {
            root_path
        } else {
            std::env::current_dir()
                .context("Failed to get current directory")?
                .join(root_path)
        };
        
        debug!("Initialized LocalStorage with root: {:?}", root_path);
        
        Ok(Self { root_path })
    }
    
    fn full_path(&self, path: &Path) -> PathBuf {
        if path.is_absolute() {
            path.to_path_buf()
        } else {
            self.root_path.join(path)
        }
    }
}

#[async_trait]
impl StorageBackend for LocalStorage {
    async fn list_directory(&self, path: &Path) -> Result<Vec<StorageItem>> {
        let full_path = self.full_path(path);
        debug!("Listing directory: {:?}", full_path);
        
        if !full_path.exists() {
            return Ok(Vec::new());
        }
        
        let mut items = Vec::new();
        let mut entries = fs::read_dir(&full_path).await
            .with_context(|| format!("Failed to read directory: {:?}", full_path))?;
        
        while let Some(entry) = entries.next_entry().await? {
            let metadata = entry.metadata().await?;
            let path = entry.path();
            let relative_path = path.strip_prefix(&self.root_path)
                .unwrap_or(&path)
                .to_path_buf();
            
            items.push(StorageItem {
                path: relative_path,
                is_directory: metadata.is_dir(),
                size: if metadata.is_file() {
                    Some(metadata.len())
                } else {
                    None
                },
                modified: metadata.modified()
                    .ok()
                    .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                    .map(|d| chrono::DateTime::from_timestamp(d.as_secs() as i64, 0))
                    .flatten(),
            });
        }
        
        Ok(items)
    }
    
    async fn exists(&self, path: &Path) -> Result<bool> {
        let full_path = self.full_path(path);
        Ok(full_path.exists())
    }
    
    async fn create_directory(&self, path: &Path) -> Result<()> {
        let full_path = self.full_path(path);
        debug!("Creating directory: {:?}", full_path);
        
        fs::create_dir_all(&full_path).await
            .with_context(|| format!("Failed to create directory: {:?}", full_path))?;
        
        Ok(())
    }
    
    async fn write_file(&self, path: &Path, data: &[u8]) -> Result<()> {
        let full_path = self.full_path(path);
        debug!("Writing file: {:?} ({} bytes)", full_path, data.len());
        
        // Ensure parent directory exists
        if let Some(parent) = full_path.parent() {
            fs::create_dir_all(parent).await
                .with_context(|| format!("Failed to create parent directory: {:?}", parent))?;
        }
        
        let mut file = fs::File::create(&full_path).await
            .with_context(|| format!("Failed to create file: {:?}", full_path))?;
        
        file.write_all(data).await
            .with_context(|| format!("Failed to write file: {:?}", full_path))?;
        
        file.flush().await?;
        
        info!("Wrote file: {:?}", full_path);
        Ok(())
    }
    
    async fn read_file(&self, path: &Path) -> Result<Vec<u8>> {
        let full_path = self.full_path(path);
        debug!("Reading file: {:?}", full_path);
        
        let data = fs::read(&full_path).await
            .with_context(|| format!("Failed to read file: {:?}", full_path))?;
        
        Ok(data)
    }
    
    async fn delete_file(&self, path: &Path) -> Result<()> {
        let full_path = self.full_path(path);
        debug!("Deleting file: {:?}", full_path);
        
        fs::remove_file(&full_path).await
            .with_context(|| format!("Failed to delete file: {:?}", full_path))?;
        
        Ok(())
    }
    
    fn storage_type(&self) -> &str {
        "Local Filesystem"
    }
    
    fn root_path(&self) -> String {
        self.root_path.display().to_string()
    }
}