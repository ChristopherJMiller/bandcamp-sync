use serde::{Deserialize, Deserializer, Serialize};
use serde_json::Value;

// Helper function to deserialize bool from either bool or int
fn bool_from_int<'de, D>(deserializer: D) -> Result<Option<bool>, D::Error>
where
    D: Deserializer<'de>,
{
    let value: Option<Value> = Option::deserialize(deserializer)?;
    Ok(value.and_then(|v| match v {
        Value::Bool(b) => Some(b),
        Value::Number(n) => n.as_i64().map(|i| i != 0),
        _ => None,
    }))
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CollectionItem {
    pub fan_id: i64,
    pub item_id: i64,
    pub item_type: String,
    pub item_title: String,
    pub item_url: String,
    pub band_id: i64,
    pub band_name: String,
    pub band_url: String,
    pub album_id: Option<i64>,
    pub album_title: Option<String>,
    pub tralbum_id: i64,
    pub tralbum_type: String,
    
    // Dates
    pub added: String,
    pub updated: String,
    pub purchased: Option<String>,
    
    // Art
    pub item_art_id: Option<i64>,
    pub item_art_url: Option<String>,
    pub band_image_id: Option<i64>,
    
    // Track info
    pub featured_track: Option<i64>,
    pub featured_track_title: Option<String>,
    pub featured_track_duration: Option<f64>,
    pub featured_track_url: Option<String>,
    pub num_streamable_tracks: Option<i32>,
    
    // Purchase info
    pub sale_item_id: Option<i64>,
    pub sale_item_type: Option<String>,
    pub price: Option<Value>,
    pub currency: Option<String>,
    pub discount: Option<f64>,
    
    // Metadata
    pub also_collected_count: i32,
    pub genre_id: Option<i64>,
    pub band_location: Option<String>,
    pub label: Option<String>,
    pub label_id: Option<i64>,
    
    // Flags
    #[serde(deserialize_with = "bool_from_int")]
    pub download_available: Option<bool>,
    #[serde(deserialize_with = "bool_from_int")]
    pub has_digital_download: Option<bool>,
    #[serde(deserialize_with = "bool_from_int")]
    pub is_preorder: Option<bool>,
    #[serde(deserialize_with = "bool_from_int")]
    pub is_private: Option<bool>,
    #[serde(deserialize_with = "bool_from_int")]
    pub is_purchasable: Option<bool>,
    #[serde(deserialize_with = "bool_from_int")]
    pub is_giftable: Option<bool>,
    #[serde(deserialize_with = "bool_from_int")]
    pub is_subscriber_only: Option<bool>,
    #[serde(deserialize_with = "bool_from_int")]
    pub is_subscription_item: Option<bool>,
    #[serde(deserialize_with = "bool_from_int")]
    pub is_set_price: Option<bool>,
    
    // Other fields we don't need but might be in response
    #[serde(skip_serializing_if = "Option::is_none", deserialize_with = "bool_from_int")]
    pub hidden: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub index: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub why: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub token: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url_hints: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub releases: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub merch_ids: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub merch_snapshot: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub package_details: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub item_art_ids: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub variant_id: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub release_count: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message_count: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none", deserialize_with = "bool_from_int")]
    pub merch_sold_out: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none", deserialize_with = "bool_from_int")]
    pub require_email: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none", deserialize_with = "bool_from_int")]
    pub licensed_item: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none", deserialize_with = "bool_from_int")]
    pub featured_track_is_custom: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub featured_track_number: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub featured_track_license_id: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub featured_track_encodings_id: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gift_id: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gift_sender_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gift_recipient_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gift_sender_note: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub listen_in_app_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub service_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub service_url_fragment: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub item_art: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CollectionResponse {
    pub items: Vec<CollectionItem>,
    pub more_available: bool,
    pub last_token: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FanData {
    pub fan_id: i64,
    pub username: String,
    pub url: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PageData {
    pub fan_data: Option<FanData>,
    pub item_cache: Option<ItemCache>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ItemCache {
    pub collection: Vec<CollectionItem>,
}