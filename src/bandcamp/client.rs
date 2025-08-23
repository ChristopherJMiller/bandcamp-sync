use anyhow::{Context, Result};
use regex::Regex;
use reqwest::{Client, header};
use scraper::{Html, Selector};
use serde_json::json;
use tracing::{debug, info};

use super::models::{CollectionItem, CollectionResponse, PageData};

pub struct BandcampClient {
    client: Client,
    cookie: String,
    fan_id: Option<i64>,
}

impl BandcampClient {
    pub fn new(cookie_data: String) -> Result<Self> {
        // Parse cookie_data which might be "cookie" or "cookie:fan_id"
        let (cookie, fan_id) = if let Some((cookie_part, fan_id_str)) = cookie_data.split_once(':') {
            let fan_id = fan_id_str.parse::<i64>().ok();
            (cookie_part.to_string(), fan_id)
        } else {
            (cookie_data, None)
        };
        
        debug!("Initializing BandcampClient with fan_id: {:?}", fan_id);
        
        let mut headers = header::HeaderMap::new();
        headers.insert(
            header::USER_AGENT,
            header::HeaderValue::from_static("Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36"),
        );
        
        let client = Client::builder()
            .default_headers(headers)
            .cookie_store(true)
            .build()?;
        
        Ok(Self { client, cookie, fan_id })
    }

    pub async fn get_fan_id(&self) -> Result<i64> {
        // If we already have the fan_id, use it
        if let Some(fan_id) = self.fan_id {
            debug!("Using stored fan_id: {}", fan_id);
            return Ok(fan_id);
        }
        
        debug!("Fan_id not stored, fetching from Bandcamp profile");
        
        // Try to get the username/fan_id from the homepage when logged in
        let homepage_response = self.client
            .get("https://bandcamp.com/")
            .header(header::COOKIE, format!("identity={}", self.cookie))
            .send()
            .await
            .context("Failed to fetch Bandcamp homepage")?;
        
        let homepage_html = homepage_response.text().await?;
        
        // Look for the username in the header (usually in a link like href="/username")
        let username_regex = Regex::new(r#"href="/([^/"]+)(?:/collection|/wishlist|")"#)?;
        let username = if let Some(captures) = username_regex.captures(&homepage_html) {
            captures.get(1).map(|m| m.as_str().to_string())
        } else {
            None
        };
        
        debug!("Found username: {:?}", username);
        
        // Build list of URLs to try
        let mut urls = vec![
            "https://bandcamp.com/".to_string(),
        ];
        
        if let Some(ref user) = username {
            urls.push(format!("https://bandcamp.com/{}", user));
            urls.push(format!("https://bandcamp.com/{}/collection", user));
        }
        
        for url in &urls {
            debug!("Trying to get fan_id from {}", url);
            let response = self.client
                .get(url.as_str())
                .header(header::COOKIE, format!("identity={}", self.cookie))
                .send()
                .await
                .context("Failed to fetch page")?;
            
            if !response.status().is_success() {
                debug!("Response status: {}", response.status());
                continue;
            }
            
            let html = response.text().await?;
            
            // Debug: save HTML to file for inspection if in verbose mode
            if url.contains("cmiller548") {
                debug!("HTML length: {} chars", html.len());
                if html.len() < 1000 {
                    debug!("Short response, might be redirect or error: {}", &html[..html.len().min(500)]);
                }
            }
            
            // Try to find fan_id in the HTML directly using regex
            let fan_id_regex = Regex::new(r#""fan_id"\s*:\s*(\d+)"#)?;
            if let Some(captures) = fan_id_regex.captures(&html) {
                if let Some(fan_id_str) = captures.get(1) {
                    let fan_id: i64 = fan_id_str.as_str().parse()?;
                    debug!("Found fan_id via regex: {}", fan_id);
                    return Ok(fan_id);
                }
            }
            
            // Also try parsing pagedata
            let document = Html::parse_document(&html);
            let selector = Selector::parse("#pagedata").unwrap();
            if let Some(element) = document.select(&selector).next() {
                if let Some(data_blob) = element.value().attr("data-blob") {
                    if let Ok(page_data) = serde_json::from_str::<PageData>(data_blob) {
                        if let Some(fan_data) = page_data.fan_data {
                            debug!("Found fan_id in pagedata: {}", fan_data.fan_id);
                            return Ok(fan_data.fan_id);
                        }
                    }
                }
            }
        }
        
        // If we still don't have fan_id, try to get it from the collection page directly
        debug!("Trying to get fan_id from collection API without it");
        
        // We can try using a placeholder and see if the API gives us the real one
        anyhow::bail!("Could not find fan_id. The cookie might be expired. Try: bandcamp-sync auth bandcamp")
    }

    pub async fn get_collection(&self, fan_id: i64) -> Result<Vec<CollectionItem>> {
        info!("Fetching Bandcamp collection for fan_id: {}", fan_id);
        
        let mut all_items = Vec::new();
        let mut older_than_token: Option<String> = None;
        let mut page = 0;
        
        loop {
            page += 1;
            debug!("Fetching collection page {}", page);
            
            let token = older_than_token.clone()
                .unwrap_or_else(|| format!("{}::a::", chrono::Utc::now().timestamp()));
            
            let payload = json!({
                "fan_id": fan_id,
                "older_than_token": token,
                "count": 100
            });
            
            let response = self.client
                .post("https://bandcamp.com/api/fancollection/1/collection_items")
                .header(header::COOKIE, format!("identity={}", self.cookie))
                .json(&payload)
                .send()
                .await
                .context("Failed to fetch collection items")?;
            
            if !response.status().is_success() {
                anyhow::bail!("API returned status: {}", response.status());
            }
            
            let response_text = response.text().await?;
            
            // Save to file for debugging
            std::fs::write("/tmp/bandcamp_response.json", &response_text).ok();
            debug!("API response saved to /tmp/bandcamp_response.json");
            debug!("Response length: {} bytes", response_text.len());
            
            let collection_response: CollectionResponse = serde_json::from_str(&response_text)
                .context("Failed to parse collection response")?;
            
            let items_count = collection_response.items.len();
            debug!("Fetched {} items", items_count);
            
            all_items.extend(collection_response.items);
            
            if !collection_response.more_available || items_count == 0 {
                break;
            }
            
            older_than_token = collection_response.last_token;
        }
        
        info!("Total collection items fetched: {}", all_items.len());
        Ok(all_items)
    }

    pub async fn fetch_collection(&self) -> Result<Vec<CollectionItem>> {
        let fan_id = self.get_fan_id().await?;
        self.get_collection(fan_id).await
    }
}