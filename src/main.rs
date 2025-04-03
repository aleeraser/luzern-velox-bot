use reqwest;
use scraper::{Html, Selector};
use std::collections::HashSet;
use std::fs;
use std::io::ErrorKind;
use std::env;
use tokio;
use teloxide::{prelude::*, types::ChatId};
use anyhow::{Context, Result};

const STATE_FILE_PATH: &str = "known_cameras.json";

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

use dotenvy;

#[tokio::main]
async fn main() -> Result<()> {
    match dotenvy::dotenv() {
        Ok(path) => log::info!("Loaded .env file from path: {}", path.display()),
        Err(e) if e.not_found() => log::info!(".env file not found, using system environment variables."),
        Err(e) => log::warn!("Failed to load .env file: {}", e),
    }

    pretty_env_logger::init();
    log::info!("Starting bot...");

    let bot = Bot::from_env();

    let chat_id_str = env::var("TELEGRAM_CHAT_ID")
        .context("TELEGRAM_CHAT_ID environment variable not set")?;
    let chat_id = ChatId(chat_id_str.parse::<i64>()
        .with_context(|| format!("Failed to parse TELEGRAM_CHAT_ID '{}' as integer", chat_id_str))?);
    log::info!("Bot initialized. Target chat ID: {}", chat_id_str);


    let url = "https://polizei.lu.ch/organisation/sicherheit_verkehrspolizei/verkehrspolizei/spezialversorgung/verkehrssicherheit/Aktuelle_Tempomessungen";
    log::info!("Fetching URL: {}", url);

    let known_cameras = load_known_cameras(STATE_FILE_PATH)?;
    log::info!("Loaded {} known cameras from {}", known_cameras.len(), STATE_FILE_PATH);

    let response = reqwest::get(url).await
        .with_context(|| format!("Failed to send GET request to {}", url))?;

    if !response.status().is_success() {
        log::error!("Failed to fetch URL {}: Status {}", url, response.status());
        anyhow::bail!("HTTP request failed with status: {}", response.status());
    }

    let body = response.text().await
        .with_context(|| format!("Failed to read response body from {}", url))?;
    log::info!("Successfully fetched HTML content.");

    let document = Html::parse_document(&body);
    let selector_str = "#radarList li > a";
    let selector = Selector::parse(selector_str)
        .map_err(|e| anyhow::anyhow!("Failed to parse CSS selector '{}': {:?}", selector_str, e))?;

    log::info!("Extracting current camera locations...");
    let mut current_cameras = HashSet::new();
    let mut found_any_cameras = false;
    for element in document.select(&selector) {
        let text = element.text().collect::<Vec<_>>().join(" ").trim().to_string();
        if !text.is_empty() && text != "Kantonsübersicht zurücksetzen" {
            log::debug!("- Found: {}", text);
            current_cameras.insert(text);
            found_any_cameras = true;
        }
    }

    if !found_any_cameras {
        log::warn!("No camera data found on the page. Check selector or page structure.");
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
        new_cameras.sort_unstable();
        let mut message_text = String::from("🚨 Neue Blitzerstandorte in Luzern:\n");
        for camera in &new_cameras {
            log::info!("- {}", camera);
            message_text.push_str(&format!("- {}\n", camera));
        }

        match bot.send_message(chat_id, &message_text).await {
            Ok(_) => log::info!("Successfully sent notification to chat ID {}", chat_id),
            Err(e) => log::error!("Failed to send Telegram message: {}", e),
        }
    }

    if known_cameras != current_cameras {
        log::info!("Updating state file {}...", STATE_FILE_PATH);
        save_known_cameras(STATE_FILE_PATH, &current_cameras)?;
        log::info!("State file updated successfully.");
    } else {
        log::info!("No changes in camera list, state file not updated.");
    }


    Ok(())
}
