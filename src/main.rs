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
use teloxide::{prelude::*, types::ChatId, Bot};
use tokio;

const STATE_FILE_PATH: &str = "known_cameras.json";
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

// Configuration struct to hold application settings
struct Config {
    bot: Bot,
    chat_id: ChatId,
}

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

// Initialize logging, load .env, and create the Config struct
async fn init_logging_and_config() -> Result<Config> {
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

    let bot = Bot::from_env();

    let chat_id_str = env::var("TELEGRAM_CHAT_ID")
        .context("TELEGRAM_CHAT_ID environment variable not set")?;
    let chat_id = ChatId(chat_id_str.parse::<i64>()
        .with_context(|| format!("Failed to parse TELEGRAM_CHAT_ID '{}' as integer", chat_id_str))?);
    log::info!("Bot initialized. Target chat ID: {}", chat_id_str);

    Ok(Config { bot, chat_id })
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

// Compare current cameras with known ones and send Telegram notification if new ones are found
async fn compare_and_notify(
    config: &Config, // Use Config struct
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

        match config.bot.send_message(config.chat_id, &message_text).await { // Use config fields
            Ok(_) => log::info!("Successfully sent notification to chat ID {}", config.chat_id),
            Err(e) => {
                // Log error but continue execution (e.g., still update state file)
                log::error!("Failed to send Telegram message: {}", e);
                // Optionally, return the error if notification failure is critical:
                // return Err(anyhow!(e).context("Failed to send Telegram message"));
            }
        }
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


#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    let config = init_logging_and_config().await?;

    let known_cameras = load_known_cameras(STATE_FILE_PATH)?;
    log::info!("Loaded {} known cameras from {}", known_cameras.len(), STATE_FILE_PATH);

    let current_cameras = fetch_and_parse_cameras(cli.offline).await?;

    // Exit early if no cameras were found on the page (logged in fetch_and_parse_cameras)
    if current_cameras.is_empty() {
         log::warn!("No current cameras found. Exiting.");
         return Ok(());
    }

    compare_and_notify(&config, &current_cameras, &known_cameras).await?; // Pass config

    update_state_file(&current_cameras, &known_cameras)?;

    log::info!("Finished execution successfully.");
    Ok(())
}
