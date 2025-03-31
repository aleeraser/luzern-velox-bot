use reqwest;
use scraper::{Html, Selector};
use std::collections::HashSet;
use std::fs;
use std::io::{self, ErrorKind};
use std::env; // Added for environment variables
use tokio;
use teloxide::{prelude::*, types::ChatId}; // Added for Teloxide

const STATE_FILE_PATH: &str = "known_cameras.json";

// Function to load known cameras from the state file
fn load_known_cameras(path: &str) -> io::Result<HashSet<String>> {
    match fs::read_to_string(path) {
        Ok(content) => {
            if content.is_empty() {
                Ok(HashSet::new())
            } else {
                serde_json::from_str(&content)
                    .map_err(|e| io::Error::new(ErrorKind::InvalidData, e))
            }
        }
        Err(e) if e.kind() == ErrorKind::NotFound => Ok(HashSet::new()), // File not found is okay, start fresh
        Err(e) => Err(e),
    }
}

// Function to save known cameras to the state file, sorted alphabetically
fn save_known_cameras(path: &str, cameras: &HashSet<String>) -> io::Result<()> {
    // Convert HashSet to a Vec and sort it
    let mut sorted_cameras: Vec<String> = cameras.iter().cloned().collect();
    sorted_cameras.sort_unstable(); // Use unstable sort for potentially better performance

    // Serialize the sorted Vec
    let content = serde_json::to_string_pretty(&sorted_cameras)
        .map_err(|e| io::Error::new(ErrorKind::InvalidData, e))?;
    fs::write(path, content)
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Initialize logging
    pretty_env_logger::init();
    log::info!("Starting bot..."); // Use log crate for logging

    // Initialize Teloxide bot
    let bot = Bot::from_env(); // Reads TELEGRAM_BOT_TOKEN from env

    // Get chat ID from environment variable
    let chat_id_str = env::var("TELEGRAM_CHAT_ID")
        .expect("TELEGRAM_CHAT_ID environment variable not set");
    let chat_id = ChatId(chat_id_str.parse::<i64>()?); // Parse the chat ID string to i64
    log::info!("Bot initialized. Target chat ID: {}", chat_id_str);


    let url = "https://polizei.lu.ch/organisation/sicherheit_verkehrspolizei/verkehrspolizei/spezialversorgung/verkehrssicherheit/Aktuelle_Tempomessungen";
    log::info!("Fetching URL: {}", url);

    // Load previously known cameras
    let known_cameras = load_known_cameras(STATE_FILE_PATH)?; // Removed 'mut'
    log::info!("Loaded {} known cameras from {}", known_cameras.len(), STATE_FILE_PATH);

    let response = reqwest::get(url).await?;

    if !response.status().is_success() {
        log::error!("Failed to fetch URL: {}", response.status());
        // Don't save state if fetch failed
        return Ok(());
    }

    let body = response.text().await?;
    log::info!("Successfully fetched HTML content.");

    let document = Html::parse_document(&body);
    let selector_str = "#radarList li > a";
    let selector = Selector::parse(selector_str).expect("Failed to parse selector");

    log::info!("Extracting current camera locations...");
    let mut current_cameras = HashSet::new();
    let mut found_any_cameras = false;
    for element in document.select(&selector) {
        let text = element.text().collect::<Vec<_>>().join(" ").trim().to_string();
        if !text.is_empty() && text != "Kantonsübersicht zurücksetzen" {
            log::debug!("- Found: {}", text); // Changed to debug level
            current_cameras.insert(text);
            found_any_cameras = true;
        }
    }

    if !found_any_cameras {
        log::warn!("No camera data found on the page. Check selector or page structure."); // Changed to warn level
        // Optionally decide if state should be cleared or kept
        // For now, we'll keep the old state if nothing is found
        return Ok(());
    }

    log::info!("Comparing with known cameras...");
    let mut new_cameras = Vec::new();
    for camera in &current_cameras {
        if !known_cameras.contains(camera) {
            new_cameras.push(camera.clone());
        }
    }

    if new_cameras.is_empty() {
        log::info!("No new cameras detected.");
    } else {
        log::info!("New cameras detected:");
        // Sort the new cameras alphabetically before creating the message
        new_cameras.sort_unstable();
        let mut message_text = String::from("🚨 Neue Blitzerstandorte in Luzern:\n");
        for camera in &new_cameras {
            log::info!("- {}", camera); // Log sorted order as well
            message_text.push_str(&format!("- {}\n", camera));
        }

        // Send notification via Telegram
        match bot.send_message(chat_id, &message_text).await {
            Ok(_) => log::info!("Successfully sent notification to chat ID {}", chat_id),
            Err(e) => log::error!("Failed to send Telegram message: {}", e),
        }
    }

    // Update known cameras state only if the fetch and parse were successful
    if known_cameras != current_cameras {
        log::info!("Updating state file {}...", STATE_FILE_PATH);
        save_known_cameras(STATE_FILE_PATH, &current_cameras)?;
        log::info!("State file updated successfully.");
    } else {
        log::info!("No changes in camera list, state file not updated.");
    }


    Ok(())
}
