use anyhow::{Context, Result};
use dotenvy;
use log;
use pretty_env_logger;
use reqwest;
use scraper::{Html, Selector};
use serde::{Deserialize, Serialize};
use serde_json;
use std::collections::{HashMap, HashSet};
use std::fs;
use std::io::ErrorKind;
use std::sync::Arc;
use std::time::Duration;
use teloxide::error_handlers::LoggingErrorHandler;
use teloxide::{
    dptree,
    prelude::*,
    types::{InputFile, Message},
    utils::command::BotCommands,
};
use tokio::sync::RwLock;
use tokio::time::interval;

const STATE_FILE_PATH: &str = "known_cameras.json";
const SUBSCRIBERS_FILE_PATH: &str = "subscribers.json";
const CAMERA_LIST_URL: &str = "https://polizei.lu.ch/organisation/sicherheit_verkehrspolizei/verkehrspolizei/spezialversorgung/verkehrssicherheit/Aktuelle_Tempomessungen";
const CAMERA_SELECTOR: &str = "#radarList li > a";
const CHECK_INTERVAL_MINUTES: u64 = 30;
const DOWNTIME_START_HOUR: u8 = 2;
const DOWNTIME_END_HOUR: u8 = 7;

// Google Maps Static API configuration
const GOOGLE_MAPS_BASE_URL: &str = "https://maps.googleapis.com/maps/api/staticmap";
const MAP_ZOOM_LEVEL: u8 = 15;
const MAP_WIDTH: u16 = 400 * 2;
const MAP_HEIGHT: u16 = 300 * 2;

// Map caching configuration
const CACHED_MAPS_DIR: &str = "cached_maps";

// Retry configuration for network operations
const MAX_RETRY_ATTEMPTS: u32 = 3;
const RETRY_DELAY_SECONDS: u64 = 5;

// Define the commands the bot understands
#[derive(BotCommands, Clone, Debug)]
#[command(
    rename_rule = "snake_case",
    description = "These commands are supported:"
)]
enum Command {
    #[command(description = "Subscribe to receive speed camera notifications.")]
    Start,
    #[command(description = "Show the current list of known speed cameras.")]
    CurrentList,
    #[command(description = "Unsubscribe from speed camera notifications.")]
    Unsubscribe,
    #[command(description = "Show help message with available commands.")]
    Help,
    #[command(description = "Force an immediate camera check.")]
    ManualUpdate,
    #[command(description = "Show bot status and last check information.")]
    Status,
    #[command(description = "Toggle notifications for checks with no updates.")]
    NotifyNoUpdates,
    #[command(description = "Toggle inclusion of maps in camera notifications.")]
    ToggleMaps,
}

// Command handler for /start
async fn start_command(
    bot: Bot,
    msg: Message,
    state: Arc<AppState>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let chat_id = msg.chat.id.0;
    log::info!("Received /start command from chat ID: {chat_id}");

    let mut subscribers = state.subscribers.write().await;
    let newly_added = subscribers
        .insert(chat_id, SubscriberData::default())
        .is_none();

    if newly_added {
        log::info!("New subscriber added: {chat_id}");
        drop(subscribers);

        let subscribers_data = {
            let guard = state.subscribers.read().await;
            guard.clone()
        };

        match save_subscribers(SUBSCRIBERS_FILE_PATH, &subscribers_data) {
            Ok(_) => log::info!("Successfully saved updated subscriber list."),
            Err(e) => {
                log::error!("Failed to save subscriber list: {e}");
            }
        }
        bot.send_message(
            msg.chat.id,
            "Subscription successful! You will now receive notifications about new speed cameras.",
        )
        .await?;
    } else {
        log::info!("User {chat_id} is already subscribed.");
        bot.send_message(msg.chat.id, "You are already subscribed.")
            .await?;
    }

    Ok(())
}

// Command handler for /current_list
async fn current_list_command(
    bot: Bot,
    msg: Message,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    log::info!(
        "Received /current_list command from chat ID: {}",
        msg.chat.id.0
    );
    log::debug!("Loading cameras from: {STATE_FILE_PATH}");

    let cameras = match load_known_cameras(STATE_FILE_PATH) {
        Ok(cameras) => {
            log::debug!("Loaded {} cameras from file", cameras.len());
            cameras
        }
        Err(e) => {
            log::error!("Failed to load cameras: {e}");
            bot.send_message(
                msg.chat.id,
                "Sorry, I couldn't load the camera list right now.",
            )
            .await?;
            return Err(e.into());
        }
    };

    let response_text = if cameras.is_empty() {
        log::info!("No cameras found, sending empty list message");
        "No known speed cameras currently listed.".to_string()
    } else {
        log::info!("Formatting {} cameras for response", cameras.len());
        let mut sorted_cameras: Vec<CameraData> = cameras.iter().cloned().collect();
        sorted_cameras.sort_unstable_by(|a, b| a.name.cmp(&b.name));
        format!(
            "Current known speed cameras:\n{}",
            sorted_cameras
                .iter()
                .map(|c| format!("- {}", c.name))
                .collect::<Vec<_>>()
                .join("\n")
        )
    };

    log::debug!(
        "Sending response message of length: {}",
        response_text.len()
    );
    match bot.send_message(msg.chat.id, response_text).await {
        Ok(_) => log::info!("Successfully sent camera list response"),
        Err(e) => {
            log::error!("Failed to send response: {e}");
            return Err(e.into());
        }
    }

    Ok(())
}

// Command handler for /unsubscribe
async fn unsubscribe_command(
    bot: Bot,
    msg: Message,
    state: Arc<AppState>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let chat_id = msg.chat.id.0;
    log::info!("Received /unsubscribe command from chat ID: {chat_id}");

    let mut subscribers = state.subscribers.write().await;
    let was_subscribed = subscribers.remove(&chat_id).is_some();

    if was_subscribed {
        log::info!("User {chat_id} unsubscribed successfully");
        drop(subscribers);

        let subscribers_data = {
            let guard = state.subscribers.read().await;
            guard.clone()
        };

        match save_subscribers(SUBSCRIBERS_FILE_PATH, &subscribers_data) {
            Ok(_) => log::info!("Successfully saved updated subscriber list after unsubscribe"),
            Err(e) => {
                log::error!("Failed to save subscriber list after unsubscribe: {e}");
            }
        }

        bot.send_message(
            msg.chat.id,
            "You have been unsubscribed from speed camera notifications.",
        )
        .await?;
    } else {
        log::info!("User {chat_id} was not subscribed");
        bot.send_message(msg.chat.id, "You are not currently subscribed.")
            .await?;
    }

    Ok(())
}

// Command handler for /help
async fn help_command(
    bot: Bot,
    msg: Message,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    log::info!("Received /help command from chat ID: {}", msg.chat.id.0);

    let help_text = format!(
        "🚗 *Luzern Speed Camera Bot* 🚗\n\n\
        This bot monitors speed cameras in Luzern, Switzerland and notifies you when new ones are detected\\.\n\n\
        *Available Commands:*\n\
        /start \\- Subscribe to notifications\n\
        /unsubscribe \\- Stop receiving notifications\n\
        /current\\_list \\- Show all known cameras\n\
        /manual\\_update \\- Force immediate check\n\
        /notify\\_no\\_updates \\- Toggle no\\-update notifications\n\
        /toggle\\_maps \\- Toggle map images in notifications\n\
        /status \\- Show bot status\n\
        /help \\- Show this help message\n\n\
        *Features:*\n\
        • Automatic checks every {} minutes\n\
        • No automatic checks between {}:00\\-{}:00\n\
        • Map images with location overview \\(when available\\)\n\
        • Data sourced from Luzern Police website\n\n\
        Questions? Contact @aleeraser",
        CHECK_INTERVAL_MINUTES,
        DOWNTIME_START_HOUR,
        DOWNTIME_END_HOUR
    );

    bot.send_message(msg.chat.id, help_text)
        .parse_mode(teloxide::types::ParseMode::MarkdownV2)
        .await?;

    Ok(())
}

// Command handler for /manual_update
async fn manual_update_command(
    bot: Bot,
    msg: Message,
    state: Arc<AppState>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let chat_id = msg.chat.id.0;
    log::info!("Received /manual_update command from chat ID: {chat_id}");

    // Get user preferences for maps
    let subscribers = state.subscribers.read().await;
    let user_prefs = subscribers.get(&chat_id).cloned().unwrap_or_default();
    let include_maps = user_prefs.include_maps;
    drop(subscribers);

    bot.send_message(msg.chat.id, "Starting manual camera check...")
        .await?;

    // Load current known cameras
    let known_cameras = match load_known_cameras(STATE_FILE_PATH) {
        Ok(cameras) => cameras,
        Err(e) => {
            log::error!("Failed to load known cameras during manual update: {e}");
            bot.send_message(msg.chat.id, "❌ Failed to load current camera data.")
                .await?;
            return Ok(());
        }
    };

    // Fetch current cameras from website
    match fetch_and_parse_cameras().await {
        Ok(current_cameras) => {
            log::info!(
                "Manual update: fetched {} cameras from website",
                current_cameras.len()
            );

            // For manual updates, we check for new cameras and send maps to the requesting user
            let new_cameras: Vec<CameraData> = current_cameras
                .difference(&known_cameras)
                .cloned()
                .collect();

            // Update state file with current cameras
            if let Err(e) = update_state_file(&current_cameras, &known_cameras) {
                log::error!("Failed to update state file during manual update: {e}");
            }

            // Send summary to the user who requested the update
            let new_count = new_cameras.len();
            let total_count = current_cameras.len();

            if new_count > 0 {
                // Send header message
                let header_message = format!("Found {} new camera(s):", new_count);
                bot.send_message(msg.chat.id, header_message).await?;

                // Get Google Maps API key from environment
                let google_maps_api_key = std::env::var("GOOGLE_MAPS_API_KEY").ok();
                if google_maps_api_key.is_none() {
                    log::warn!("GOOGLE_MAPS_API_KEY not found in environment. Map images will not be included in manual update.");
                }

                // Send individual messages with maps for each new camera
                for camera in &new_cameras {
                    let camera_message = format!("📍 {}", camera.name);

                    match send_message_with_map(
                        &bot,
                        msg.chat.id,
                        &camera_message,
                        camera,
                        google_maps_api_key.as_deref(),
                        include_maps,
                    )
                    .await
                    {
                        Ok(_) => {
                            log::debug!("Successfully sent camera map for: {}", camera);
                        }
                        Err(e) => {
                            log::error!("Failed to send camera map for {}: {}", camera, e);
                        }
                    }

                    // Small delay between messages to avoid rate limiting
                    tokio::time::sleep(Duration::from_millis(500)).await;
                }
            } else {
                let summary = format!("No new cameras found ({} total cameras)", total_count);
                bot.send_message(msg.chat.id, summary).await?;
            }
        }
        Err(e) => {
            log::error!("Failed to fetch cameras during manual update: {e}");
            bot.send_message(
                msg.chat.id,
                "❌ Failed to fetch camera data from website. Please try again later.",
            )
            .await?;
        }
    }

    Ok(())
}

// Command handler for /status
async fn status_command(
    bot: Bot,
    msg: Message,
    state: Arc<AppState>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    log::info!("Received /status command from chat ID: {}", msg.chat.id.0);

    // Get subscriber count
    let subscriber_count = {
        let subscribers = state.subscribers.read().await;
        subscribers.len()
    };

    // Get known camera count
    let camera_count = match load_known_cameras(STATE_FILE_PATH) {
        Ok(cameras) => cameras.len(),
        Err(_) => 0,
    };

    // Check if we're in downtime
    let downtime_status = if is_downtime() {
        "🌙 In downtime \\(checks paused\\)"
    } else {
        "🔄 Active monitoring"
    };

    let status_text = format!(
        "🤖 *Bot Status*\n\n\
        📊 *Statistics:*\n\
        • Known cameras: {}\n\
        • Active subscribers: {}\n\
        • Check interval: {} minutes\n\
        • Downtime: {}:00\\-{}:00\n\n\
        *Current Status:*\n\
        {}\n\n\
        *Data Source:*\n\
        [Luzern Police Official Website]({})",
        camera_count,
        subscriber_count,
        CHECK_INTERVAL_MINUTES,
        DOWNTIME_START_HOUR,
        DOWNTIME_END_HOUR,
        downtime_status,
        CAMERA_LIST_URL
    );

    bot.send_message(msg.chat.id, status_text)
        .parse_mode(teloxide::types::ParseMode::MarkdownV2)
        .await?;

    Ok(())
}

// Command handler for /notify_no_updates
async fn notify_no_updates_command(
    bot: Bot,
    msg: Message,
    state: Arc<AppState>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let chat_id = msg.chat.id.0;
    log::info!("Received /notify_no_updates command from chat ID: {chat_id}");

    // Get current preference
    let mut subscribers = state.subscribers.write().await;
    let current_prefs = subscribers
        .entry(chat_id)
        .or_insert_with(SubscriberData::default);

    // Toggle the preference
    current_prefs.notify_no_updates = !current_prefs.notify_no_updates;
    let new_setting = current_prefs.notify_no_updates;

    // Save preferences to file
    let subscribers_copy = subscribers.clone();
    drop(subscribers);

    match save_subscribers(SUBSCRIBERS_FILE_PATH, &subscribers_copy) {
        Ok(_) => log::info!("Successfully saved subscriber preferences after toggle"),
        Err(e) => {
            log::error!("Failed to save user preferences: {e}");
        }
    }

    // Send confirmation message
    let message = if new_setting {
        "✅ You will now receive notifications when camera checks find no updates\\."
    } else {
        "❌ You will no longer receive notifications when camera checks find no updates\\."
    };

    bot.send_message(msg.chat.id, message)
        .parse_mode(teloxide::types::ParseMode::MarkdownV2)
        .await?;

    log::info!("User {chat_id} toggled notify_no_updates to: {new_setting}");
    Ok(())
}

// Command handler for /toggle_maps
async fn toggle_maps_command(
    bot: Bot,
    msg: Message,
    state: Arc<AppState>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let chat_id = msg.chat.id.0;
    log::info!("Received /toggle_maps command from chat ID: {chat_id}");

    // Get current preference
    let mut subscribers = state.subscribers.write().await;
    let current_prefs = subscribers
        .entry(chat_id)
        .or_insert_with(SubscriberData::default);

    // Toggle the preference
    current_prefs.include_maps = !current_prefs.include_maps;
    let new_setting = current_prefs.include_maps;

    // Save preferences to file
    let subscribers_copy = subscribers.clone();
    drop(subscribers);

    match save_subscribers(SUBSCRIBERS_FILE_PATH, &subscribers_copy) {
        Ok(_) => log::info!("Successfully saved subscriber preferences after toggle"),
        Err(e) => {
            log::error!("Failed to save user preferences: {e}");
        }
    }

    // Send confirmation message
    let message = if new_setting {
        "✅ Maps will now be included with camera notifications\\."
    } else {
        "❌ Maps will no longer be included with camera notifications\\. You'll receive text\\-only messages\\."
    };

    bot.send_message(msg.chat.id, message)
        .parse_mode(teloxide::types::ParseMode::MarkdownV2)
        .await?;

    log::info!("User {chat_id} toggled include_maps to: {new_setting}");
    Ok(())
}

// Camera data with coordinates
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
struct CameraData {
    name: String,
    latitude: f64,
    longitude: f64,
}

impl std::fmt::Display for CameraData {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.name)
    }
}

impl std::hash::Hash for CameraData {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.name.hash(state);
        // Convert f64 to bits for hashing
        self.latitude.to_bits().hash(state);
        self.longitude.to_bits().hash(state);
    }
}

impl Eq for CameraData {}

impl Ord for CameraData {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.name.cmp(&other.name)
    }
}

impl PartialOrd for CameraData {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

// Subscriber data with preferences
#[derive(Serialize, Deserialize, Clone, Debug)]
struct SubscriberData {
    notify_no_updates: bool,
    include_maps: bool,
}

impl Default for SubscriberData {
    fn default() -> Self {
        Self {
            notify_no_updates: false, // Default to not sending "no updates" notifications
            include_maps: true,       // Default to including maps in notifications
        }
    }
}

// Shared application state
struct AppState {
    subscribers: RwLock<HashMap<i64, SubscriberData>>,
}

// Load known cameras from JSON file
fn load_known_cameras(path: &str) -> Result<HashSet<CameraData>> {
    match fs::read_to_string(path) {
        Ok(content) => {
            if content.is_empty() {
                Ok(HashSet::new())
            } else {
                // Try to load new format first, fallback to old format for migration
                if let Ok(camera_data) = serde_json::from_str::<HashSet<CameraData>>(&content) {
                    Ok(camera_data)
                } else {
                    // Legacy format migration: convert old string-based data
                    log::warn!(
                        "Converting legacy camera data format to new coordinate-based format"
                    );
                    let old_cameras: HashSet<String> = serde_json::from_str(&content)
                        .with_context(|| format!("Failed to parse JSON from {path}"))?;

                    // Convert old format to new format (without coordinates for now)
                    let new_cameras: HashSet<CameraData> = old_cameras
                        .into_iter()
                        .map(|name| CameraData {
                            name,
                            latitude: 0.0,  // Will be updated on next fetch
                            longitude: 0.0, // Will be updated on next fetch
                        })
                        .collect();

                    Ok(new_cameras)
                }
            }
        }
        Err(e) if e.kind() == ErrorKind::NotFound => Ok(HashSet::new()),
        Err(e) => {
            Err(anyhow::Error::from(e)).with_context(|| format!("Failed to read state file {path}"))
        }
    }
}

// Load subscribed chat IDs and preferences from JSON file
fn load_subscribers(path: &str) -> Result<HashMap<i64, SubscriberData>> {
    match fs::read_to_string(path) {
        Ok(content) => {
            if content.is_empty() {
                Ok(HashMap::new())
            } else {
                // Try to parse as new format first
                if let Ok(subscribers) =
                    serde_json::from_str::<HashMap<i64, SubscriberData>>(&content)
                {
                    return Ok(subscribers);
                }

                // Try to parse as intermediate format (map with old SubscriberData without include_maps)
                #[derive(Deserialize)]
                struct OldSubscriberData {
                    notify_no_updates: bool,
                }

                if let Ok(old_map) =
                    serde_json::from_str::<HashMap<i64, OldSubscriberData>>(&content)
                {
                    log::info!(
                        "Migrating {} subscribers from intermediate format to new format",
                        old_map.len()
                    );
                    let mut new_subscribers = HashMap::new();
                    for (chat_id, old_data) in old_map {
                        new_subscribers.insert(
                            chat_id,
                            SubscriberData {
                                notify_no_updates: old_data.notify_no_updates,
                                include_maps: true, // Default to including maps
                            },
                        );
                    }
                    return Ok(new_subscribers);
                }

                // Fall back to old format (array of IDs) and migrate
                let old_subscribers: Vec<i64> = serde_json::from_str(&content)
                    .with_context(|| format!("Failed to parse subscriber JSON from {path}"))?;

                log::info!(
                    "Migrating {} subscribers from old format to new format",
                    old_subscribers.len()
                );
                let mut new_subscribers = HashMap::new();
                for chat_id in old_subscribers {
                    new_subscribers.insert(chat_id, SubscriberData::default());
                }
                Ok(new_subscribers)
            }
        }
        Err(e) if e.kind() == ErrorKind::NotFound => Ok(HashMap::new()),
        Err(e) => Err(anyhow::Error::from(e))
            .with_context(|| format!("Failed to read subscriber file {path}")),
    }
}

// Save subscribed chat IDs and preferences to JSON file
fn save_subscribers(path: &str, subscribers: &HashMap<i64, SubscriberData>) -> Result<()> {
    let content = serde_json::to_string_pretty(subscribers)
        .with_context(|| "Failed to serialize subscriber data to JSON")?;
    fs::write(path, content).with_context(|| format!("Failed to write subscriber file {path}"))
}

// Send message with retry logic for network failures
async fn send_message_with_retry(
    bot: &Bot,
    chat_id: teloxide::types::ChatId,
    text: String,
) -> Result<()> {
    send_message_with_retry_and_parse_mode(bot, chat_id, text, None).await
}

// Send message with retry logic and optional parse mode
async fn send_message_with_retry_and_parse_mode(
    bot: &Bot,
    chat_id: teloxide::types::ChatId,
    text: String,
    parse_mode: Option<teloxide::types::ParseMode>,
) -> Result<()> {
    use teloxide::requests::Requester;

    let mut last_error = None;

    for attempt in 1..=MAX_RETRY_ATTEMPTS {
        let mut request = bot.send_message(chat_id, text.clone());
        if let Some(mode) = parse_mode {
            request = request.parse_mode(mode);
        }

        match request.await {
            Ok(_) => {
                if attempt > 1 {
                    log::info!(
                        "Successfully sent message to {} after {} attempts",
                        chat_id,
                        attempt
                    );
                }
                return Ok(());
            }
            Err(e) => {
                last_error = Some(anyhow::anyhow!("Telegram API error: {}", e));
                log::warn!(
                    "Attempt {}/{} to send message to {} failed: {}",
                    attempt,
                    MAX_RETRY_ATTEMPTS,
                    chat_id,
                    e
                );

                if attempt < MAX_RETRY_ATTEMPTS {
                    log::info!(
                        "Retrying message send in {} seconds...",
                        RETRY_DELAY_SECONDS
                    );
                    tokio::time::sleep(Duration::from_secs(RETRY_DELAY_SECONDS)).await;
                } else {
                    log::error!(
                        "All {} attempts to send message to {} failed",
                        MAX_RETRY_ATTEMPTS,
                        chat_id
                    );
                }
            }
        }
    }

    Err(last_error.unwrap())
}

// Save known cameras to JSON file
fn save_known_cameras(path: &str, cameras: &HashSet<CameraData>) -> Result<()> {
    let mut sorted_cameras: Vec<CameraData> = cameras.iter().cloned().collect();
    sorted_cameras.sort_by(|a, b| a.name.cmp(&b.name));

    let content = serde_json::to_string_pretty(&sorted_cameras)
        .with_context(|| "Failed to serialize camera list to JSON")?;
    fs::write(path, content).with_context(|| format!("Failed to write state file {path}"))
}

// Generate a map image URL using Google Maps Static API with coordinates
fn generate_map_url_with_coordinates(camera: &CameraData, api_key: &str) -> String {
    format!(
        "{}?center={},{}&zoom={}&size={}x{}&maptype=roadmap&markers=color:red|label:C|{},{}&key={}",
        GOOGLE_MAPS_BASE_URL,
        camera.latitude,
        camera.longitude,
        MAP_ZOOM_LEVEL,
        MAP_WIDTH,
        MAP_HEIGHT,
        camera.latitude,
        camera.longitude,
        api_key
    )
}

// Download map image from Google Maps Static API using coordinates (with caching)
async fn download_map_image_with_coordinates(
    camera: &CameraData,
    api_key: &str,
) -> Result<bytes::Bytes> {
    // First, try to load from cache
    match load_map_from_cache(camera).await {
        Ok(cached_bytes) => {
            log::debug!(
                "Using cached map image for {} ({} bytes)",
                camera.name,
                cached_bytes.len()
            );
            return Ok(cached_bytes);
        }
        Err(_) => {
            log::debug!(
                "No cached map found for {}, downloading from API",
                camera.name
            );
        }
    }

    // If not in cache, download from Google Maps API
    let url = generate_map_url_with_coordinates(camera, api_key);
    log::debug!("Downloading map image for {} from: {}", camera.name, url);

    let client = reqwest::Client::new();
    let response = client
        .get(&url)
        .timeout(Duration::from_secs(30))
        .send()
        .await
        .with_context(|| "Failed to send request to Google Maps API")?;

    if !response.status().is_success() {
        return Err(anyhow::anyhow!(
            "Google Maps API returned error status: {}",
            response.status()
        ));
    }

    let image_bytes = response
        .bytes()
        .await
        .with_context(|| "Failed to download image bytes")?;

    log::debug!(
        "Downloaded {} bytes for map image for {}",
        image_bytes.len(),
        camera.name
    );

    // Save to cache for future use (don't fail if caching fails)
    if let Err(e) = save_map_to_cache(camera, &image_bytes).await {
        log::warn!("Failed to cache map for {}: {}", camera.name, e);
        // Continue anyway - we still have the image data
    }

    Ok(image_bytes)
}

// Generate cache filename for a map image
fn generate_cache_filename(camera: &CameraData) -> String {
    // Clean the camera name: remove " - " patterns, parentheses, and replace spaces with underscores
    let cleaned_name = camera
        .name
        .replace(" - ", "-") // Remove spaces around dashes
        .replace('(', "") // Remove opening parentheses
        .replace(')', "") // Remove closing parentheses
        .replace(' ', "_"); // Replace remaining spaces with underscores

    format!(
        "{}-{}-{}-{}-{}x{}.png",
        cleaned_name, camera.latitude, camera.longitude, MAP_ZOOM_LEVEL, MAP_WIDTH, MAP_HEIGHT
    )
}

// Check if a cached map exists and return its path
fn get_cached_map_path(camera: &CameraData) -> (std::path::PathBuf, bool) {
    let filename = generate_cache_filename(camera);
    let path = std::path::Path::new(CACHED_MAPS_DIR).join(filename);
    let exists = path.exists();
    (path, exists)
}

// Save map image to cache
async fn save_map_to_cache(camera: &CameraData, image_bytes: &bytes::Bytes) -> Result<()> {
    // Ensure cache directory exists
    if let Err(e) = std::fs::create_dir_all(CACHED_MAPS_DIR) {
        log::warn!(
            "Failed to create cache directory {}: {}",
            CACHED_MAPS_DIR,
            e
        );
        return Err(anyhow::anyhow!("Failed to create cache directory: {}", e));
    }

    let (cache_path, _) = get_cached_map_path(camera);

    match std::fs::write(&cache_path, image_bytes) {
        Ok(_) => {
            log::debug!(
                "Saved map image for {} to cache: {}",
                camera.name,
                cache_path.display()
            );
            Ok(())
        }
        Err(e) => {
            log::warn!(
                "Failed to save map image for {} to cache: {}",
                camera.name,
                e
            );
            Err(anyhow::anyhow!("Failed to save map to cache: {}", e))
        }
    }
}

// Load map image from cache
async fn load_map_from_cache(camera: &CameraData) -> Result<bytes::Bytes> {
    let (cache_path, exists) = get_cached_map_path(camera);

    if !exists {
        return Err(anyhow::anyhow!("Cached map not found"));
    }

    match std::fs::read(&cache_path) {
        Ok(data) => {
            log::debug!(
                "Loaded map image for {} from cache: {} ({} bytes)",
                camera.name,
                cache_path.display(),
                data.len()
            );
            Ok(bytes::Bytes::from(data))
        }
        Err(e) => {
            log::warn!("Failed to read cached map for {}: {}", camera.name, e);
            Err(anyhow::anyhow!("Failed to read cached map: {}", e))
        }
    }
}

// Create a temporary file for the map image
async fn create_temp_map_file(image_bytes: bytes::Bytes) -> Result<tempfile::NamedTempFile> {
    use std::io::Write;

    let mut temp_file =
        tempfile::NamedTempFile::new().with_context(|| "Failed to create temporary file")?;

    temp_file
        .write_all(&image_bytes)
        .with_context(|| "Failed to write image data to temporary file")?;

    temp_file
        .flush()
        .with_context(|| "Failed to flush temporary file")?;

    Ok(temp_file)
}

// Fetch the webpage and parse out the current camera locations
async fn fetch_and_parse_cameras() -> Result<HashSet<CameraData>> {
    fetch_and_parse_cameras_with_retry().await
}

// Fetch cameras with retry logic for network failures
async fn fetch_and_parse_cameras_with_retry() -> Result<HashSet<CameraData>> {
    let mut last_error = None;

    for attempt in 1..=MAX_RETRY_ATTEMPTS {
        log::info!(
            "Fetching URL (attempt {}/{}): {}",
            attempt,
            MAX_RETRY_ATTEMPTS,
            CAMERA_LIST_URL
        );

        match fetch_cameras_once().await {
            Ok(cameras) => {
                if attempt > 1 {
                    log::info!("Successfully recovered after {} attempts", attempt);
                }
                return Ok(cameras);
            }
            Err(e) => {
                last_error = Some(e);
                log::warn!(
                    "Attempt {}/{} failed: {}",
                    attempt,
                    MAX_RETRY_ATTEMPTS,
                    last_error.as_ref().unwrap()
                );

                if attempt < MAX_RETRY_ATTEMPTS {
                    log::info!("Retrying in {} seconds...", RETRY_DELAY_SECONDS);
                    tokio::time::sleep(Duration::from_secs(RETRY_DELAY_SECONDS)).await;
                } else {
                    log::error!("All {} attempts failed", MAX_RETRY_ATTEMPTS);
                }
            }
        }
    }

    Err(last_error.unwrap())
}

// Single attempt to fetch and parse cameras with coordinates
async fn fetch_cameras_once() -> Result<HashSet<CameraData>> {
    let response = reqwest::get(CAMERA_LIST_URL)
        .await
        .with_context(|| format!("Failed to send GET request to {}", CAMERA_LIST_URL))?;

    if !response.status().is_success() {
        anyhow::bail!("HTTP request failed with status: {}", response.status());
    }

    let body = response
        .text()
        .await
        .with_context(|| format!("Failed to read response body from {}", CAMERA_LIST_URL))?;
    log::debug!("Successfully fetched HTML content online.");

    let document = Html::parse_document(&body);
    let selector = Selector::parse(CAMERA_SELECTOR).map_err(|e| {
        anyhow::anyhow!(
            "Failed to parse CSS selector '{}': {:?}",
            CAMERA_SELECTOR,
            e
        )
    })?;

    log::debug!(
        "Extracting current camera locations with coordinates using selector '{}'...",
        CAMERA_SELECTOR
    );
    let mut current_cameras = HashSet::new();
    let mut found_any_cameras = false;

    for element in document.select(&selector) {
        let text = element
            .text()
            .collect::<Vec<_>>()
            .join(" ")
            .trim()
            .to_string();

        // Specific filter for the Luzern page: Exclude the "reset filter" link text
        if text.is_empty() || text == "Kantonsübersicht zurücksetzen" {
            continue;
        }

        // Extract coordinates from onclick attribute
        if let Some(onclick) = element.value().attr("onclick") {
            if let Some((lat, lng)) = extract_coordinates_from_onclick(onclick) {
                let camera_data = CameraData {
                    name: text.clone(),
                    latitude: lat,
                    longitude: lng,
                };

                log::debug!("- Found: {} at coordinates ({}, {})", text, lat, lng);
                current_cameras.insert(camera_data);
                found_any_cameras = true;
            } else {
                log::warn!("Could not extract coordinates from onclick for: {}", text);
                // Still add camera without coordinates for compatibility
                let camera_data = CameraData {
                    name: text.clone(),
                    latitude: 0.0,
                    longitude: 0.0,
                };
                current_cameras.insert(camera_data);
                found_any_cameras = true;
            }
        } else {
            log::warn!("No onclick attribute found for camera: {}", text);
            // Still add camera without coordinates for compatibility
            let camera_data = CameraData {
                name: text.clone(),
                latitude: 0.0,
                longitude: 0.0,
            };
            current_cameras.insert(camera_data);
            found_any_cameras = true;
        }
    }

    if !found_any_cameras {
        log::warn!("No camera data found on the page using selector '{}'. Check selector or page structure.", CAMERA_SELECTOR);
        return Ok(HashSet::new());
    }

    log::info!("Found {} cameras on the website", current_cameras.len());
    Ok(current_cameras)
}

// Extract latitude and longitude from onclick attribute
// Expects format like: "map.flyTo([47.0357531942389, 8.16251290734679], 16,{animate: false})"
fn extract_coordinates_from_onclick(onclick: &str) -> Option<(f64, f64)> {
    use regex::Regex;

    // Create regex to match coordinates in flyTo call
    let re = Regex::new(r"map\.flyTo\(\[([0-9.-]+),\s*([0-9.-]+)\]").ok()?;

    if let Some(captures) = re.captures(onclick) {
        let lat_str = captures.get(1)?.as_str();
        let lng_str = captures.get(2)?.as_str();

        let lat: f64 = lat_str.parse().ok()?;
        let lng: f64 = lng_str.parse().ok()?;

        Some((lat, lng))
    } else {
        None
    }
}

// Send message with map image for a speed camera location
async fn send_message_with_map(
    bot: &Bot,
    chat_id: ChatId,
    message_text: &str,
    camera_data: &CameraData,
    google_maps_api_key: Option<&str>,
    include_maps: bool,
) -> Result<()> {
    match (google_maps_api_key, include_maps) {
        (Some(api_key), true) => {
            // Try to send with map image
            match send_message_with_map_image(bot, chat_id, message_text, camera_data, api_key)
                .await
            {
                Ok(_) => {
                    log::debug!(
                        "Successfully sent message with map to chat ID {}",
                        chat_id.0
                    );
                    Ok(())
                }
                Err(e) => {
                    log::warn!(
                        "Failed to send message with map to {}: {}. Falling back to text-only.",
                        chat_id.0,
                        e
                    );
                    // Fall back to text-only message
                    send_message_with_retry(bot, chat_id, message_text.to_string()).await
                }
            }
        }
        (None, _) | (_, false) => {
            // Send text-only message if no API key is available or user doesn't want maps
            send_message_with_retry(bot, chat_id, message_text.to_string()).await
        }
    }
}

// Send message with map image using Google Maps Static API
async fn send_message_with_map_image(
    bot: &Bot,
    chat_id: ChatId,
    message_text: &str,
    camera_data: &CameraData,
    api_key: &str,
) -> Result<()> {
    // Download map image
    let image_bytes = download_map_image_with_coordinates(camera_data, api_key)
        .await
        .with_context(|| "Failed to download map image")?;

    // Create temporary file
    let temp_file = create_temp_map_file(image_bytes)
        .await
        .with_context(|| "Failed to create temporary file")?;

    // Create InputFile from the temp file path
    let input_file = InputFile::file(temp_file.path());

    // Send photo with caption
    bot.send_photo(chat_id, input_file)
        .caption(message_text)
        .await
        .with_context(|| "Failed to send photo message")?;

    log::debug!("Successfully sent photo message to chat ID {}", chat_id.0);
    Ok(())
}

// Compare current cameras with known ones and send Telegram notification to all subscribers
async fn compare_and_notify(
    bot: Bot,
    state: Arc<AppState>,
    current_cameras: &HashSet<CameraData>,
    known_cameras: &HashSet<CameraData>,
) -> Result<()> {
    log::info!(
        "Comparing current cameras ({}) with known cameras ({})",
        current_cameras.len(),
        known_cameras.len()
    );
    let mut new_cameras = Vec::new();
    for camera in current_cameras {
        if !known_cameras.contains(camera) {
            new_cameras.push(camera.clone());
        }
    }

    if new_cameras.is_empty() {
        log::info!("No new cameras detected.");

        // Send "no updates" notifications to users who have opted in
        let subscribers = state.subscribers.read().await;

        if !subscribers.is_empty() {
            let mut no_update_subscribers = Vec::new();
            for (chat_id, subscriber_data) in subscribers.iter() {
                if subscriber_data.notify_no_updates {
                    no_update_subscribers.push(*chat_id);
                }
            }

            if !no_update_subscribers.is_empty() {
                log::info!(
                    "Sending 'no updates' notification to {} subscribers",
                    no_update_subscribers.len()
                );
                let no_update_message =
                    "ℹ️ Camera check completed: No new speed cameras detected\\.";

                for chat_id_val in no_update_subscribers {
                    let chat_id = ChatId(chat_id_val);
                    match send_message_with_retry_and_parse_mode(
                        &bot,
                        chat_id,
                        no_update_message.to_string(),
                        Some(teloxide::types::ParseMode::MarkdownV2),
                    )
                    .await
                    {
                        Ok(_) => {
                            log::debug!(
                                "Successfully sent 'no updates' notification to chat ID {}",
                                chat_id.0
                            );
                        }
                        Err(e) => {
                            log::error!(
                                "Failed to send 'no updates' message to {} after retries: {}",
                                chat_id.0,
                                e
                            );
                        }
                    }
                }
            }
        }
    } else {
        log::info!("New cameras detected:");
        new_cameras.sort_unstable();

        // Get Google Maps API key from environment
        let google_maps_api_key = std::env::var("GOOGLE_MAPS_API_KEY").ok();
        if google_maps_api_key.is_none() {
            log::warn!(
                "GOOGLE_MAPS_API_KEY not found in environment. Map images will not be included."
            );
        }

        // Get subscriber list (read lock)
        let subscribers = state.subscribers.read().await;
        if subscribers.is_empty() {
            log::warn!("New cameras detected but no subscribers to notify.");
            return Ok(());
        }

        log::info!(
            "Sending notification to {} subscribers for {} new cameras...",
            subscribers.len(),
            new_cameras.len()
        );
        let mut success_count = 0;
        let mut error_count = 0;

        for (chat_id_val, subscriber_data) in subscribers.iter() {
            let chat_id = ChatId(*chat_id_val);

            // Send a header message first
            let header_message = format!("🚨 {} new speed camera(s):", new_cameras.len());
            match send_message_with_retry(&bot, chat_id, header_message).await {
                Ok(_) => log::debug!("Sent header message to chat ID {}", chat_id.0),
                Err(e) => log::error!("Failed to send header message to {}: {}", chat_id.0, e),
            }

            // Send individual messages with maps for each camera
            for camera in &new_cameras {
                log::info!("Sending notification for camera: {}", camera.name);
                let camera_message = format!("📍 {}", camera.name);

                match send_message_with_map(
                    &bot,
                    chat_id,
                    &camera_message,
                    camera,
                    google_maps_api_key.as_deref(),
                    subscriber_data.include_maps,
                )
                .await
                {
                    Ok(_) => {
                        log::debug!(
                            "Successfully sent camera notification to chat ID {}",
                            chat_id.0
                        );
                        success_count += 1;
                    }
                    Err(e) => {
                        log::error!(
                            "Failed to send camera notification to {} after retries: {}",
                            chat_id.0,
                            e
                        );
                        error_count += 1;
                    }
                }

                // Small delay between messages to avoid rate limiting
                tokio::time::sleep(Duration::from_millis(500)).await;
            }
        }

        log::info!(
            "Finished sending notifications. Success: {}, Errors: {}",
            success_count,
            error_count
        );
    }
    Ok(())
}

// Update the state file if the current camera list differs from the known one
fn update_state_file(
    current_cameras: &HashSet<CameraData>,
    known_cameras: &HashSet<CameraData>,
) -> Result<()> {
    if known_cameras != current_cameras {
        log::info!(
            "Changes detected. Updating state file {}...",
            STATE_FILE_PATH
        );
        save_known_cameras(STATE_FILE_PATH, current_cameras)?;
        log::info!("State file updated successfully.");
    } else {
        log::info!("No changes in camera list, state file not updated.");
    }
    Ok(())
}

// Check if current time is within downtime hours (2 AM - 7 AM local time)
fn is_downtime() -> bool {
    use chrono::prelude::*;
    let now = Local::now();
    let hour = now.hour() as u8;
    hour >= DOWNTIME_START_HOUR && hour < DOWNTIME_END_HOUR
}

// Background task to periodically check for camera updates
async fn camera_monitoring_task(bot: Bot, state: Arc<AppState>) {
    let mut interval = interval(Duration::from_secs(CHECK_INTERVAL_MINUTES * 60));

    // Skip the first tick to avoid duplicate check immediately after startup
    log::info!(
        "Camera monitoring task started. Next check in {} minutes.",
        CHECK_INTERVAL_MINUTES
    );
    interval.tick().await;

    loop {
        interval.tick().await;

        if is_downtime() {
            log::info!(
                "Skipping camera check during downtime hours ({}-{} local time)",
                DOWNTIME_START_HOUR,
                DOWNTIME_END_HOUR
            );
            continue;
        }

        log::info!("Starting periodic camera check...");

        // Load current known cameras
        let known_cameras = match load_known_cameras(STATE_FILE_PATH) {
            Ok(cameras) => cameras,
            Err(e) => {
                log::error!("Failed to load known cameras: {e}");
                continue;
            }
        };

        // Fetch current cameras from website
        match fetch_and_parse_cameras().await {
            Ok(current_cameras) => {
                log::info!("Fetched {} cameras from website", current_cameras.len());

                // Compare and notify if there are new cameras
                if let Err(e) =
                    compare_and_notify(bot.clone(), state.clone(), &current_cameras, &known_cameras)
                        .await
                {
                    log::error!("Failed to compare and notify: {e}");
                }

                // Update state file with current cameras
                if let Err(e) = update_state_file(&current_cameras, &known_cameras) {
                    log::error!("Failed to update state file: {e}");
                }

                log::info!("Periodic camera check completed successfully");
            }
            Err(e) => {
                log::error!("Failed to fetch cameras during periodic check: {e}");
                // Continue to next check rather than crashing
            }
        }
    }
}

// Initialize logging and load .env
fn init_logging() -> Result<()> {
    match dotenvy::dotenv() {
        Ok(path) => log::info!("Loaded .env file from path: {}", path.display()),
        Err(e) if e.not_found() => {
            log::info!(".env file not found, using system environment variables.")
        }
        Err(e) => log::warn!("Failed to load .env file: {e}"),
    }

    pretty_env_logger::init();
    log::info!("Starting bot...");

    Ok(())
}

// Command handler logic - routes commands to specific functions
async fn handle_commands(
    bot: Bot,
    msg: Message,
    cmd: Command,
    state: Arc<AppState>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    log::debug!("Handling command: {cmd:?}");
    log::info!("Command handler called with: {cmd:?}");
    match cmd {
        Command::Start => {
            log::debug!("Routing to start_command");
            start_command(bot, msg, state).await
        }
        Command::CurrentList => {
            log::debug!("Routing to current_list_command");
            current_list_command(bot, msg).await
        }
        Command::Unsubscribe => {
            log::debug!("Routing to unsubscribe_command");
            unsubscribe_command(bot, msg, state).await
        }
        Command::Help => {
            log::debug!("Routing to help_command");
            help_command(bot, msg).await
        }
        Command::ManualUpdate => {
            log::debug!("Routing to manual_update_command");
            manual_update_command(bot, msg, state).await
        }
        Command::Status => {
            log::debug!("Routing to status_command");
            status_command(bot, msg, state).await
        }
        Command::NotifyNoUpdates => {
            log::debug!("Routing to notify_no_updates_command");
            notify_no_updates_command(bot, msg, state).await
        }
        Command::ToggleMaps => {
            log::debug!("Routing to toggle_maps_command");
            toggle_maps_command(bot, msg, state).await
        }
    }
}

// Default handler for any message that isn't a recognized command
async fn default_handler(msg: Message) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    log::debug!("Unhandled message received: {msg:?}");

    // Check if this looks like a command that should have been handled
    if let teloxide::types::MessageKind::Common(common) = &msg.kind {
        if let teloxide::types::MediaKind::Text(text) = &common.media_kind {
            if text.text.starts_with('/') {
                log::warn!(
                    "Command '{}' was not recognized by the command handler",
                    text.text
                );
            }
        }
    }

    Ok(())
}

#[tokio::main]
async fn main() -> Result<()> {
    // Initialize logging
    init_logging()?;

    // Initialize the bot
    let bot = Bot::from_env();
    log::info!("Bot instance created.");

    // Test bot connection by getting bot info
    match bot.get_me().await {
        Ok(me) => log::info!("Bot authenticated successfully: @{}", me.username()),
        Err(e) => {
            log::error!("Failed to authenticate bot: {e}");
            log::error!("Please check your TELOXIDE_TOKEN environment variable");
            return Err(anyhow::anyhow!("Bot authentication failed: {e}"));
        }
    }

    // Load initial subscribers
    let initial_subscribers = load_subscribers(SUBSCRIBERS_FILE_PATH)?;
    log::info!(
        "Loaded {} initial subscribers from {}",
        initial_subscribers.len(),
        SUBSCRIBERS_FILE_PATH
    );

    // Create the shared state
    let app_state = Arc::new(AppState {
        subscribers: RwLock::new(initial_subscribers),
    });

    // Perform initial camera check
    log::info!("Performing initial camera check...");
    let known_cameras = load_known_cameras(STATE_FILE_PATH)?;
    log::info!(
        "Loaded {} known cameras from state file",
        known_cameras.len()
    );

    match fetch_and_parse_cameras().await {
        Ok(current_cameras) => {
            log::info!(
                "Successfully fetched {} cameras from website",
                current_cameras.len()
            );

            // Compare and notify if there are new cameras
            if let Err(e) = compare_and_notify(
                bot.clone(),
                app_state.clone(),
                &current_cameras,
                &known_cameras,
            )
            .await
            {
                log::error!("Failed to compare and notify: {e}");
            }

            // Update state file with current cameras
            if let Err(e) = update_state_file(&current_cameras, &known_cameras) {
                log::error!("Failed to update state file: {e}");
            }
        }
        Err(e) => {
            log::error!("Failed to fetch cameras from website on startup: {e}");
            log::warn!("Bot will continue with existing camera data");
        }
    }

    // Start the background camera monitoring task
    log::info!(
        "Starting background camera monitoring task (interval: {} minutes)",
        CHECK_INTERVAL_MINUTES
    );
    let monitoring_bot = bot.clone();
    let monitoring_state = app_state.clone();
    tokio::spawn(async move {
        camera_monitoring_task(monitoring_bot, monitoring_state).await;
    });

    // Build the handler chain
    let handler = Update::filter_message()
        .branch(
            dptree::entry()
                .filter_command::<Command>()
                .endpoint(handle_commands),
        )
        .branch(dptree::endpoint(default_handler));

    // Build and start the dispatcher
    log::info!("Starting dispatcher...");
    Dispatcher::builder(bot, handler)
        .dependencies(dptree::deps![app_state])
        .enable_ctrlc_handler()
        .error_handler(LoggingErrorHandler::with_custom_text(
            "An error occurred in the dispatcher",
        ))
        .build()
        .dispatch()
        .await;

    log::info!("Bot shutdown complete.");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cache_filename_generation() {
        let camera = CameraData {
            name: "Test Camera - Location".to_string(),
            latitude: 47.0502,
            longitude: 8.3093,
        };

        let filename = generate_cache_filename(&camera);
        assert_eq!(
            filename,
            "Test_Camera-Location-47.0502-8.3093-15-800x600.png"
        );

        let camera_with_spaces = CameraData {
            name: "Camera With Spaces".to_string(),
            latitude: 46.1234,
            longitude: 7.5678,
        };

        let filename_spaces = generate_cache_filename(&camera_with_spaces);
        assert_eq!(
            filename_spaces,
            "Camera_With_Spaces-46.1234-7.5678-15-800x600.png"
        );

        let camera_with_parentheses = CameraData {
            name: "Camera (Test Location)".to_string(),
            latitude: 45.9876,
            longitude: 6.4321,
        };

        let filename_parens = generate_cache_filename(&camera_with_parentheses);
        assert_eq!(
            filename_parens,
            "Camera_Test_Location-45.9876-6.4321-15-800x600.png"
        );
    }
}
