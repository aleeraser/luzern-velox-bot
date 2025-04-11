use anyhow::{Context, Result};
use clap::Parser; // Added for command-line argument parsing
use dotenvy;
use log;
use pretty_env_logger;
use reqwest;
use scraper::{Html, Selector};
use serde_json;
use std::collections::HashSet;
use std::env;
use std::fs;
use std::io::ErrorKind;
use teloxide::{prelude::*, dptree, types::{ChatId, Message}, utils::command::BotCommands, Bot}; // Added Message, dptree
use teloxide::dispatching::HandlerExt; // Removed UpdateHandler
// Removed DependencyMap import
use tokio;
use tokio::sync::RwLock; // Added for AppState
use std::sync::Arc; // Added for AppState
use futures::future; // More specific import for join_all

const STATE_FILE_PATH: &str = "known_cameras.json";
const SUBSCRIBERS_FILE_PATH: &str = "subscribers.json"; // Added for subscriber persistence
const CAMERA_LIST_URL: &str = "https://polizei.lu.ch/organisation/sicherheit_verkehrspolizei/verkehrspolizei/spezialversorgung/verkehrssicherheit/Aktuelle_Tempomessungen";
const OFFLINE_FILE_PATH: &str = "velox_page.html"; // Path to the local HTML file
const CAMERA_SELECTOR: &str = "#radarList li > a";

// Command-line arguments structure
#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Cli {
    /// Run in offline mode, reading from a local file instead of fetching the URL
    #[arg(short, long)]
    offline: bool,
}

// Define the commands the bot understands
#[derive(BotCommands, Clone, Debug)]
#[command(rename_rule = "lowercase", description = "These commands are supported:")]
enum Command {
    #[command(description = "Subscribe to receive speed camera notifications.")]
    Start,
    // Other commands will be added here later
}

// Command handler for /start
async fn start_command(bot: Bot, msg: Message, state: Arc<AppState>) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let chat_id = msg.chat.id.0; // Get the i64 chat ID
    log::info!("Received /start command from chat ID: {}", chat_id);

    let mut subscribers = state.subscribers.write().await; // Acquire write lock

    let newly_added = subscribers.insert(chat_id); // Add user, returns true if new

    if newly_added {
        log::info!("New subscriber added: {}", chat_id);
        // Save the updated list persistently
        // Release the write lock before saving to avoid holding it during potentially slow I/O
        drop(subscribers); // Explicitly drop the write guard

        // Clone the data out of the read lock to minimize lock duration and simplify borrowing
        let subscribers_data = { // Create a scope for the read lock
             let guard = state.subscribers.read().await;
             guard.clone() // Clone the HashSet<i64>
        }; // Read lock guard is dropped here

        match save_subscribers(SUBSCRIBERS_FILE_PATH, &subscribers_data) { // Pass reference to cloned data
             Ok(_) => log::info!("Successfully saved updated subscriber list."),
             Err(e) => {
                 log::error!("Failed to save subscriber list: {}", e);
                 // Decide how critical this is. Maybe notify admin? For now, just log.
                 // We could try to remove the user from the in-memory set if saving fails,
                 // but that adds complexity. Let's keep it simple for now.
             }
        }
        bot.send_message(msg.chat.id, "Subscription successful! You will now receive notifications about new speed cameras.").await?;
    } else {
        log::info!("User {} is already subscribed.", chat_id);
        bot.send_message(msg.chat.id, "You are already subscribed.").await?;
    }

    Ok(())
}


// Shared application state
struct AppState {
    subscribers: RwLock<HashSet<i64>>, // Store ChatId.0 (i64)
}

// Removed Config struct as it's no longer needed

fn load_known_cameras(path: &str) -> Result<HashSet<String>> {
    match fs::read_to_string(path) {
        Ok(content) => {
            if content.is_empty() {
                Ok(HashSet::new())
            } else {
                serde_json::from_str(&content)
                    .with_context(|| format!("Failed to parse JSON from {}", path))
            }
        }
        Err(e) if e.kind() == ErrorKind::NotFound => Ok(HashSet::new()),
        Err(e) => Err(anyhow::Error::from(e)).with_context(|| format!("Failed to read state file {}", path)),
    }
}

fn save_known_cameras(path: &str, cameras: &HashSet<String>) -> Result<()> {
    let mut sorted_cameras: Vec<String> = cameras.iter().cloned().collect();
    sorted_cameras.sort_unstable();

    let content = serde_json::to_string_pretty(&sorted_cameras)
        .with_context(|| "Failed to serialize camera list to JSON")?;
    fs::write(path, content)
        .with_context(|| format!("Failed to write state file {}", path))
}

// Load subscribed chat IDs from the JSON file
fn load_subscribers(path: &str) -> Result<HashSet<i64>> {
    match fs::read_to_string(path) {
        Ok(content) => {
            if content.is_empty() {
                Ok(HashSet::new())
            } else {
                serde_json::from_str(&content)
                    .with_context(|| format!("Failed to parse subscriber JSON from {}", path))
            }
        }
        Err(e) if e.kind() == ErrorKind::NotFound => Ok(HashSet::new()), // No file yet, return empty set
        Err(e) => Err(anyhow::Error::from(e)).with_context(|| format!("Failed to read subscriber file {}", path)),
    }
}

// Save subscribed chat IDs to the JSON file
fn save_subscribers(path: &str, subscribers: &HashSet<i64>) -> Result<()> {
    // Sort for consistent file output (optional but nice)
    let mut sorted_subscribers: Vec<i64> = subscribers.iter().cloned().collect();
    sorted_subscribers.sort_unstable();

    let content = serde_json::to_string_pretty(&sorted_subscribers)
        .with_context(|| "Failed to serialize subscriber list to JSON")?;
    fs::write(path, content)
        .with_context(|| format!("Failed to write subscriber file {}", path))
}


// Initialize logging and load .env
fn init_logging() -> Result<()> { // No longer async, just initializes logger
    match dotenvy::dotenv() {
        Ok(path) => log::info!("Loaded .env file from path: {}", path.display()),
        Err(e) if e.not_found() => log::info!(".env file not found, using system environment variables."),
        Err(e) => log::warn!("Failed to load .env file: {}", e),
    }

    pretty_env_logger::init();
    log::info!("Starting bot...");

    // Log variables specifically loaded from .env (optional but helpful for debugging)
    log::debug!("Variables loaded from .env file:");
    let env_vars_to_log = ["TELOXIDE_TOKEN", "TELEGRAM_CHAT_ID", "RUST_LOG"];
    for var_name in env_vars_to_log {
        match env::var(var_name) {
            Ok(value) => log::debug!("  {} = {}", var_name, value),
            Err(_) => log::warn!("  {} not found in environment", var_name),
        }
    }

    // Removed TELEGRAM_CHAT_ID loading logic
    // Bot initialization will happen in main

    Ok(())
}

// Fetch the webpage (or read from file) and parse out the current camera locations
async fn fetch_and_parse_cameras(offline_mode: bool) -> Result<HashSet<String>> {
    let body = if offline_mode {
        log::info!("Running in offline mode. Reading from file: {}", OFFLINE_FILE_PATH);
        fs::read_to_string(OFFLINE_FILE_PATH)
            .with_context(|| format!("Failed to read offline file {}", OFFLINE_FILE_PATH))?
    } else {
        log::info!("Fetching URL: {}", CAMERA_LIST_URL);
        let response = reqwest::get(CAMERA_LIST_URL).await
            .with_context(|| format!("Failed to send GET request to {}", CAMERA_LIST_URL))?;

        if !response.status().is_success() {
            log::error!("Failed to fetch URL {}: Status {}", CAMERA_LIST_URL, response.status());
            anyhow::bail!("HTTP request failed with status: {}", response.status());
        }

        let online_body = response.text().await
            .with_context(|| format!("Failed to read response body from {}", CAMERA_LIST_URL))?;
        log::info!("Successfully fetched HTML content online.");
        online_body
    };

    let document = Html::parse_document(&body);
    let selector = Selector::parse(CAMERA_SELECTOR)
        .map_err(|e| anyhow::anyhow!("Failed to parse CSS selector '{}': {:?}", CAMERA_SELECTOR, e))?;

    log::info!("Extracting current camera locations using selector '{}'...", CAMERA_SELECTOR);
    let mut current_cameras = HashSet::new();
    let mut found_any_cameras = false;
    for element in document.select(&selector) {
        let text = element.text().collect::<Vec<_>>().join(" ").trim().to_string();
        // Specific filter for the Luzern page: Exclude the "reset filter" link text
        if !text.is_empty() && text != "Kantonsübersicht zurücksetzen" {
            log::debug!("- Found: {}", text);
            current_cameras.insert(text);
            found_any_cameras = true;
        }
    }

     if !found_any_cameras {
        log::warn!("No camera data found on the page using selector '{}'. Check selector or page structure.", CAMERA_SELECTOR);
        // Return an empty set, main function will handle this.
        return Ok(HashSet::new());
    }

    Ok(current_cameras)
}

// Compare current cameras with known ones and send Telegram notification to all subscribers
async fn compare_and_notify( // Keep this function for later use in the scheduled task
    bot: Bot,
    state: Arc<AppState>,
    current_cameras: &HashSet<String>,
    known_cameras: &HashSet<String>,
) -> Result<()> {
    log::info!("Comparing current cameras ({}) with known cameras ({})", current_cameras.len(), known_cameras.len());
    let mut new_cameras = Vec::new();
    for camera in current_cameras {
        if !known_cameras.contains(camera) {
            new_cameras.push(camera.clone());
        }
    }

    if new_cameras.is_empty() {
        log::info!("No new cameras detected.");
    } else {
        log::info!("New cameras detected:");
        new_cameras.sort_unstable(); // Sort for consistent message order
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

        log::info!("Sending notification to {} subscribers...", subscribers.len());
        let mut send_futures = Vec::new();

        for chat_id_val in subscribers.iter() {
             let chat_id = ChatId(*chat_id_val);
             // Clone bot and message for each concurrent send task
             let bot_clone = bot.clone();
             let message_clone = message_text.clone();
             // Spawn a task for each message send
             send_futures.push(tokio::spawn(async move {
                 match bot_clone.send_message(chat_id, message_clone).await {
                     Ok(_) => {
                         log::debug!("Successfully sent notification to chat ID {}", chat_id.0);
                         Ok(()) // Indicate success for this specific send
                     }
                     Err(e) => {
                         log::error!("Failed to send Telegram message to {}: {}", chat_id.0, e);
                         Err(e) // Propagate the error for this specific send
                     }
                 }
             }));
        }

        // Wait for all send tasks to complete and log aggregate results
        let results = future::join_all(send_futures).await; // Use imported future module
        let success_count = results.iter().filter(|res| res.is_ok() && res.as_ref().unwrap().is_ok()).count();
        let error_count = results.len() - success_count;

        log::info!("Finished sending notifications. Success: {}, Errors: {}", success_count, error_count);
        // Decide if overall function should return an error if *any* send failed.
        // For now, just log errors and return Ok(()) overall.
    }
    Ok(())
}

// Update the state file if the current camera list differs from the known one
fn update_state_file(
    current_cameras: &HashSet<String>,
    known_cameras: &HashSet<String>,
) -> Result<()> {
    if known_cameras != current_cameras {
        log::info!("Changes detected. Updating state file {}...", STATE_FILE_PATH);
        save_known_cameras(STATE_FILE_PATH, current_cameras)?;
        log::info!("State file updated successfully.");
    } else {
        log::info!("No changes in camera list, state file not updated.");
    }
    Ok(())
}


// Command handler logic - routes commands to specific functions
async fn handle_commands(bot: Bot, msg: Message, cmd: Command, state: Arc<AppState>) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    match cmd {
        Command::Start => start_command(bot, msg, state).await?,
        // Add matches for other commands here later
    };
    Ok(())
}


#[tokio::main]
async fn main() -> Result<()> {
    // Initialize logging
    init_logging()?; // No longer async

    // Parse command-line arguments (e.g., for --offline mode, though not used in dispatcher yet)
    let _cli = Cli::parse(); // Keep parsing but maybe use it later

    // Initialize the bot directly here
    let bot = Bot::from_env();
    log::info!("Bot instance created.");

    // Load initial subscribers
    let initial_subscribers = load_subscribers(SUBSCRIBERS_FILE_PATH)?;
    log::info!("Loaded {} initial subscribers from {}", initial_subscribers.len(), SUBSCRIBERS_FILE_PATH);

    // Create the shared state
    let app_state = Arc::new(AppState {
        subscribers: RwLock::new(initial_subscribers),
    });

    // Build the handler chain directly
    let handler = Update::filter_message()
        .branch(
            dptree::entry()
                .filter_command::<Command>()
                .endpoint(handle_commands)
        );

    // Build the dispatcher
    log::info!("Starting dispatcher...");
    Dispatcher::builder(bot, handler) // Pass the handler directly
        .dependencies(dptree::deps![app_state]) // Correct deps macro usage
        .enable_ctrlc_handler() // Graceful shutdown on Ctrl+C
        .build()
        .dispatch()
        .await;

    log::info!("Dispatcher finished."); // Should only happen on shutdown
    Ok(())
}
