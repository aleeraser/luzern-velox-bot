use reqwest;
use scraper::{Html, Selector};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::fs;
use std::io::{self, ErrorKind};
use tokio;

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

// Function to save known cameras to the state file
fn save_known_cameras(path: &str, cameras: &HashSet<String>) -> io::Result<()> {
    let content = serde_json::to_string_pretty(cameras)
        .map_err(|e| io::Error::new(ErrorKind::InvalidData, e))?;
    fs::write(path, content)
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let url = "https://polizei.lu.ch/organisation/sicherheit_verkehrspolizei/verkehrspolizei/spezialversorgung/verkehrssicherheit/Aktuelle_Tempomessungen";
    println!("Fetching URL: {}", url);

    // Load previously known cameras
    let mut known_cameras = load_known_cameras(STATE_FILE_PATH)?;
    println!("Loaded {} known cameras from {}", known_cameras.len(), STATE_FILE_PATH);

    let response = reqwest::get(url).await?;

    if !response.status().is_success() {
        eprintln!("Failed to fetch URL: {}", response.status());
        // Don't save state if fetch failed
        return Ok(());
    }

    let body = response.text().await?;
    println!("Successfully fetched HTML content.");

    let document = Html::parse_document(&body);
    let selector_str = "#radarList li > a";
    let selector = Selector::parse(selector_str).expect("Failed to parse selector");

    println!("\nExtracting current camera locations...");
    let mut current_cameras = HashSet::new();
    let mut found_any_cameras = false;
    for element in document.select(&selector) {
        let text = element.text().collect::<Vec<_>>().join(" ").trim().to_string();
        if !text.is_empty() && text != "Kantonsübersicht zurücksetzen" {
            println!("- Found: {}", text);
            current_cameras.insert(text);
            found_any_cameras = true;
        }
    }

    if !found_any_cameras {
        println!("No camera data found on the page. Check selector or page structure.");
        // Optionally decide if state should be cleared or kept
        // For now, we'll keep the old state if nothing is found
        return Ok(());
    }

    println!("\nComparing with known cameras...");
    let mut new_cameras = Vec::new();
    for camera in &current_cameras {
        if !known_cameras.contains(camera) {
            new_cameras.push(camera.clone());
        }
    }

    if new_cameras.is_empty() {
        println!("No new cameras detected.");
    } else {
        println!("New cameras detected:");
        for camera in &new_cameras {
            println!("- {}", camera);
            // Here you would trigger the Telegram notification in a later step
        }
    }

    // Update known cameras state only if the fetch and parse were successful
    if known_cameras != current_cameras {
        println!("Updating state file {}...", STATE_FILE_PATH);
        save_known_cameras(STATE_FILE_PATH, &current_cameras)?;
        println!("State file updated successfully.");
    } else {
        println!("No changes in camera list, state file not updated.");
    }


    Ok(())
}
