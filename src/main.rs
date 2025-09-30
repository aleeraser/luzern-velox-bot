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
use teloxide::{dptree, prelude::*, types::Message, utils::command::BotCommands};
use tokio::sync::RwLock;
use tokio::time::interval;

const STATE_FILE_PATH: &str = "known_cameras.json";
const SUBSCRIBERS_FILE_PATH: &str = "subscribers.json";
const CAMERA_LIST_URL: &str = "https://polizei.lu.ch/organisation/sicherheit_verkehrspolizei/verkehrspolizei/spezialversorgung/verkehrssicherheit/Aktuelle_Tempomessungen";
const CAMERA_SELECTOR: &str = "#radarList li > a";
const CHECK_INTERVAL_MINUTES: u64 = 30;
const DOWNTIME_START_HOUR: u8 = 2;
const DOWNTIME_END_HOUR: u8 = 7;

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
        let mut sorted_cameras: Vec<String> = cameras.iter().cloned().collect();
        sorted_cameras.sort_unstable();
        format!(
            "Current known speed cameras:\n{}",
            sorted_cameras
                .iter()
                .map(|c| format!("- {c}"))
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
        /status \\- Show bot status\n\
        /help \\- Show this help message\n\n\
        *Features:*\n\
        • Automatic checks every {} minutes\n\
        • No automatic checks between {}:00\\-{}:00\n\
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

    bot.send_message(msg.chat.id, "🔄 Starting manual camera check...")
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

            // Compare and notify if there are new cameras
            if let Err(e) =
                compare_and_notify(bot.clone(), state.clone(), &current_cameras, &known_cameras)
                    .await
            {
                log::error!("Failed to compare and notify during manual update: {e}");
            }

            // Update state file with current cameras
            if let Err(e) = update_state_file(&current_cameras, &known_cameras) {
                log::error!("Failed to update state file during manual update: {e}");
            }

            // Send summary to the user who requested the update
            let new_count = current_cameras.difference(&known_cameras).count();
            let total_count = current_cameras.len();

            let summary = if new_count > 0 {
                format!(
                    "✅ Manual check complete!\n📊 Found {} new cameras out of {} total",
                    new_count, total_count
                )
            } else {
                format!(
                    "✅ Manual check complete!\n📊 No new cameras found ({} total cameras)",
                    total_count
                )
            };

            bot.send_message(msg.chat.id, summary).await?;
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
        🔄 *Current Status:*\n\
        {}\n\n\
        📡 *Data Source:*\n\
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

// Subscriber data with preferences
#[derive(Serialize, Deserialize, Clone, Debug)]
struct SubscriberData {
    notify_no_updates: bool,
}

impl Default for SubscriberData {
    fn default() -> Self {
        Self {
            notify_no_updates: false, // Default to not sending "no updates" notifications
        }
    }
}

// Shared application state
struct AppState {
    subscribers: RwLock<HashMap<i64, SubscriberData>>,
}

// Load known cameras from JSON file
fn load_known_cameras(path: &str) -> Result<HashSet<String>> {
    match fs::read_to_string(path) {
        Ok(content) => {
            if content.is_empty() {
                Ok(HashSet::new())
            } else {
                serde_json::from_str(&content)
                    .with_context(|| format!("Failed to parse JSON from {path}"))
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

// Save known cameras to JSON file
fn save_known_cameras(path: &str, cameras: &HashSet<String>) -> Result<()> {
    let mut sorted_cameras: Vec<String> = cameras.iter().cloned().collect();
    sorted_cameras.sort_unstable();

    let content = serde_json::to_string_pretty(&sorted_cameras)
        .with_context(|| "Failed to serialize camera list to JSON")?;
    fs::write(path, content).with_context(|| format!("Failed to write state file {path}"))
}

// Fetch the webpage and parse out the current camera locations
async fn fetch_and_parse_cameras() -> Result<HashSet<String>> {
    log::info!("Fetching URL: {}", CAMERA_LIST_URL);
    let response = reqwest::get(CAMERA_LIST_URL)
        .await
        .with_context(|| format!("Failed to send GET request to {}", CAMERA_LIST_URL))?;

    if !response.status().is_success() {
        log::error!(
            "Failed to fetch URL {}: Status {}",
            CAMERA_LIST_URL,
            response.status()
        );
        anyhow::bail!("HTTP request failed with status: {}", response.status());
    }

    let body = response
        .text()
        .await
        .with_context(|| format!("Failed to read response body from {}", CAMERA_LIST_URL))?;
    log::info!("Successfully fetched HTML content online.");

    let document = Html::parse_document(&body);
    let selector = Selector::parse(CAMERA_SELECTOR).map_err(|e| {
        anyhow::anyhow!(
            "Failed to parse CSS selector '{}': {:?}",
            CAMERA_SELECTOR,
            e
        )
    })?;

    log::info!(
        "Extracting current camera locations using selector '{}'...",
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
        if !text.is_empty() && text != "Kantonsübersicht zurücksetzen" {
            log::debug!("- Found: {}", text);
            current_cameras.insert(text);
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

// Compare current cameras with known ones and send Telegram notification to all subscribers
async fn compare_and_notify(
    bot: Bot,
    state: Arc<AppState>,
    current_cameras: &HashSet<String>,
    known_cameras: &HashSet<String>,
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
                    match bot
                        .send_message(chat_id, no_update_message)
                        .parse_mode(teloxide::types::ParseMode::MarkdownV2)
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
                                "Failed to send 'no updates' message to {}: {}",
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
        let mut message_text = String::from("🚨 Neue Blitzerstandorte in Luzern:\n");
        for camera in &new_cameras {
            log::info!("- {}", camera);
            message_text.push_str(&format!("- {}\n", camera));
        }

        // Get subscriber list (read lock)
        let subscribers = state.subscribers.read().await;
        if subscribers.is_empty() {
            log::warn!("New cameras detected but no subscribers to notify.");
            return Ok(());
        }

        log::info!(
            "Sending notification to {} subscribers...",
            subscribers.len()
        );
        let mut success_count = 0;
        let mut error_count = 0;

        for (chat_id_val, _) in subscribers.iter() {
            let chat_id = ChatId(*chat_id_val);
            match bot.send_message(chat_id, message_text.clone()).await {
                Ok(_) => {
                    log::debug!("Successfully sent notification to chat ID {}", chat_id.0);
                    success_count += 1;
                }
                Err(e) => {
                    log::error!("Failed to send Telegram message to {}: {}", chat_id.0, e);
                    error_count += 1;
                }
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
    current_cameras: &HashSet<String>,
    known_cameras: &HashSet<String>,
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
