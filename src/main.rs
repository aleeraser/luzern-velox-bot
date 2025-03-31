use anyhow::{anyhow, Context, Result};
use dotenvy;
use log;
use pretty_env_logger;
use std::collections::{HashMap, HashSet}; // Added HashMap
use std::fs;
use std::io::ErrorKind;
use std::path::Path; // Added for path operations
use std::sync::Arc;
use teloxide::{prelude::*, types::ChatId, utils::command::BotCommands};
use tokio::sync::Mutex;
use tokio::time::{sleep, Duration}; // Added for scheduling
use chrono::{Local, Timelike}; // Added for time checking

const KNOWN_CAMERAS_FILE_PATH: &str = "known_cameras.json";
const SUBSCRIBED_CHATS_FILE_PATH: &str = "subscribed_chats.json";
const USER_PREFERENCES_FILE_PATH: &str = "user_preferences.json"; // Added preferences file path

// --- Camera State Handling ---

// Function to load known cameras from the state file
fn load_known_cameras<P: AsRef<Path>>(path: P) -> Result<HashSet<String>> {
    let path = path.as_ref();
    match fs::read_to_string(path) {
        Ok(content) => {
            if content.is_empty() {
                Ok(HashSet::new())
            } else {
                serde_json::from_str(&content)
                    .with_context(|| format!("Failed to parse JSON from {}", path.display())) // Use path.display()
            }
        }
        Err(e) if e.kind() == ErrorKind::NotFound => Ok(HashSet::new()), // File not found is okay, start fresh
        Err(e) => Err(anyhow!(e)).with_context(|| format!("Failed to read state file {}", path.display())),
    }
}

// Function to save known cameras to the state file, sorted alphabetically
fn save_known_cameras<P: AsRef<Path>>(path: P, cameras: &HashSet<String>) -> Result<()> {
    let path = path.as_ref();
    // Convert HashSet to a Vec and sort it
    let mut sorted_cameras: Vec<String> = cameras.iter().cloned().collect();
    sorted_cameras.sort_unstable(); // Use unstable sort for potentially better performance

    // Serialize the sorted Vec
    let content = serde_json::to_string_pretty(&sorted_cameras)
        .with_context(|| "Failed to serialize camera list to JSON")?;
    fs::write(path, content)
        .with_context(|| format!("Failed to write state file {}", path.display()))
}

// --- Subscribed Chats Handling ---

// Function to load subscribed chat IDs
fn load_subscribed_chats<P: AsRef<Path>>(path: P) -> Result<HashSet<ChatId>> {
    let path = path.as_ref();
    match fs::read_to_string(path) {
        Ok(content) => {
            if content.is_empty() {
                Ok(HashSet::new())
            } else {
                serde_json::from_str(&content)
                    .with_context(|| format!("Failed to parse JSON from {}", path.display())) // Use path.display()
            }
        }
        Err(e) if e.kind() == ErrorKind::NotFound => Ok(HashSet::new()),
        Err(e) => Err(anyhow!(e)).with_context(|| format!("Failed to read subscribed chats file {}", path.display())),
    }
}

// Function to save subscribed chat IDs
fn save_subscribed_chats<P: AsRef<Path>>(path: P, chats: &HashSet<ChatId>) -> Result<()> {
    let path = path.as_ref();
    let content = serde_json::to_string_pretty(chats)
        .with_context(|| "Failed to serialize subscribed chats to JSON")?;
    fs::write(path, content)
        .with_context(|| format!("Failed to write subscribed chats file {}", path.display()))
}

// --- User Preferences Handling ---

// Type alias for user preferences (ChatId -> Notify on No Updates?)
type UserPreferences = HashMap<ChatId, bool>;

// Function to load user preferences
fn load_user_preferences<P: AsRef<Path>>(path: P) -> Result<UserPreferences> {
    let path = path.as_ref();
    match fs::read_to_string(path) {
        Ok(content) => {
            if content.is_empty() {
                Ok(HashMap::new())
            } else {
                serde_json::from_str(&content)
                    .with_context(|| format!("Failed to parse JSON from {}", path.display()))
            }
        }
        Err(e) if e.kind() == ErrorKind::NotFound => Ok(HashMap::new()),
        Err(e) => Err(anyhow!(e)).with_context(|| format!("Failed to read user preferences file {}", path.display())),
    }
}

// Function to save user preferences
fn save_user_preferences<P: AsRef<Path>>(path: P, preferences: &UserPreferences) -> Result<()> {
    let path = path.as_ref();
    let content = serde_json::to_string_pretty(preferences)
        .with_context(|| "Failed to serialize user preferences to JSON")?;
    fs::write(path, content)
        .with_context(|| format!("Failed to write user preferences file {}", path.display()))
}


// --- Bot Commands ---

#[derive(BotCommands, Clone, Debug)]
#[command(rename_rule = "lowercase", description = "These commands are supported:")]
enum Command {
    #[command(description = "Start receiving notifications.")]
    Start,
    #[command(description = "Show the current list of known speed cameras.", rename = "current_list")]
    CurrentList,
    #[command(description = "Toggle notifications for checks with no new cameras.", rename = "notify_no_updates")]
    NotifyNoUpdates,
    #[command(description = "Trigger an immediate check for new speed cameras.", rename = "manual_update")]
    ManualUpdate,
    // Add other commands here later
    // Help,
}

// --- Command Handler ---

// Type aliases for shared state
type SharedSubscribedChats = Arc<Mutex<HashSet<ChatId>>>;
type SharedUserPreferences = Arc<Mutex<UserPreferences>>; // Added shared preferences type

async fn handle_command(
    bot: Bot,
    msg: Message,
    cmd: Command,
    subscribed_chats: SharedSubscribedChats,
    user_preferences: SharedUserPreferences, // Added preferences to handler args
) -> Result<()> {
    let chat_id = msg.chat.id; // Get chat_id once for all commands

    match cmd {
        Command::Start => {
            // let chat_id = msg.chat.id; // Already defined above
            log::info!("Received /start command from chat ID: {}", chat_id);

            let mut chats = subscribed_chats.lock().await;
            if chats.insert(chat_id) {
                // Chat ID was added (wasn't already present)
                log::info!("Added chat ID {} to subscriptions.", chat_id);
                // Save the updated list
                if let Err(e) = save_subscribed_chats(SUBSCRIBED_CHATS_FILE_PATH, &chats) {
                    log::error!("Failed to save subscribed chats: {:?}", e);
                    bot.send_message(chat_id, "An error occurred while saving your subscription. Please try again later.").await?;
                } else {
                    log::info!("Successfully saved subscribed chats.");
                    bot.send_message(chat_id, "You are now subscribed to Luzern speed camera notifications!").await?;
                }
            } else {
                // Chat ID was already present
                log::info!("Chat ID {} is already subscribed.", chat_id);
                    bot.send_message(chat_id, "You are already subscribed.").await?;
            }
        }
        Command::CurrentList => {
            // let chat_id = msg.chat.id; // Already defined above
            log::info!("Received /current_list command from chat ID: {}", chat_id);

            match load_known_cameras(KNOWN_CAMERAS_FILE_PATH) {
                Ok(cameras) => {
                    let message = if cameras.is_empty() {
                        "Currently, no speed cameras are known or the list is empty.".to_string()
                    } else {
                        let mut sorted_cameras: Vec<String> = cameras.iter().cloned().collect();
                        sorted_cameras.sort_unstable();
                        format!("Current known speed cameras:\n\n{}", sorted_cameras.join("\n"))
                    };
                    bot.send_message(chat_id, message).await?;
                    log::info!("Sent current camera list to chat ID: {}", chat_id);
                }
                Err(e) => {
                    log::error!("Failed to load known cameras for /current_list: {:?}", e);
                    bot.send_message(chat_id, "Sorry, I couldn't retrieve the current camera list due to an error.").await?;
                }
            }
        }
        Command::NotifyNoUpdates => {
            // let chat_id = msg.chat.id; // Already defined above
            log::info!("Received /notify_no_updates command from chat ID: {}", chat_id);

            let mut prefs = user_preferences.lock().await;
            // Get current value, default to false if not present
            let current_pref = prefs.entry(chat_id).or_insert(false);
            // Toggle the value
            *current_pref = !*current_pref;
            let new_pref = *current_pref; // Copy the new value for the message

            // Save the updated preferences
            if let Err(e) = save_user_preferences(USER_PREFERENCES_FILE_PATH, &prefs) {
                log::error!("Failed to save user preferences: {:?}", e);
                bot.send_message(chat_id, "An error occurred while saving your preference. Please try again later.").await?;
            } else {
                log::info!("Successfully saved user preferences for chat ID {}.", chat_id);
                let message = if new_pref {
                    "You will now be notified even when no new cameras are found."
                } else {
                    "You will only be notified when new cameras are found."
                };
                bot.send_message(chat_id, message).await?;
            }
        }
        Command::ManualUpdate => {
            log::info!("Received /manual_update command from chat ID: {}", chat_id);
            bot.send_message(chat_id, "Starting manual update check...").await?;

            // Call the update check logic
            // We need to pass the necessary shared state here
            match check_for_updates(bot.clone(), subscribed_chats.clone(), user_preferences.clone()).await {
                Ok(status_message) => {
                    log::info!("Manual update check completed for chat ID {}: {}", chat_id, status_message);
                    bot.send_message(chat_id, status_message).await?;
                }
                Err(e) => {
                    log::error!("Manual update check failed for chat ID {}: {:?}", chat_id, e);
                    bot.send_message(chat_id, format!("Manual update check failed: {}", e)).await?;
                }
            }
        }
        // Add handlers for other commands here later
        // Command::Help => { ... }
    }
    Ok(())
}


// --- Camera Update Logic ---

// Placeholder for the actual web scraping logic
async fn fetch_current_cameras() -> Result<HashSet<String>> {
    // In a real implementation, this would use reqwest and scraper
    // to fetch and parse the HTML from the Luzern Police website.
    // Example data:
    let cameras = HashSet::new();
    // cameras.insert("Luzern, Example Street 1".to_string());
    // cameras.insert("Emmenbrücke, Test Road 2".to_string());
    Ok(cameras)
    // Err(anyhow!("Placeholder: Scraping not implemented"))
}

// Function to perform the update check and notify users
async fn check_for_updates(
    bot: Bot,
    subscribed_chats: SharedSubscribedChats,
    user_preferences: SharedUserPreferences,
) -> Result<String> {
    log::info!("Starting update check...");

    let current_cameras = fetch_current_cameras().await
        .context("Failed to fetch current cameras from website")?;
    log::debug!("Fetched cameras: {:?}", current_cameras);

    let known_cameras = load_known_cameras(KNOWN_CAMERAS_FILE_PATH)
        .context("Failed to load known cameras")?;
    log::debug!("Loaded known cameras: {:?}", known_cameras);

    let new_cameras: HashSet<String> = current_cameras.difference(&known_cameras).cloned().collect();
    log::info!("Found {} new cameras.", new_cameras.len());

    let chats_to_notify = subscribed_chats.lock().await;
    let preferences = user_preferences.lock().await;

    if !new_cameras.is_empty() {
        let mut sorted_new_cameras: Vec<String> = new_cameras.iter().cloned().collect();
        sorted_new_cameras.sort_unstable();
        let message = format!(
            "🚨 New speed camera(s) detected in Luzern:\n\n{}",
            sorted_new_cameras.join("\n")
        );

        log::info!("Saving updated camera list...");
        if let Err(e) = save_known_cameras(KNOWN_CAMERAS_FILE_PATH, &current_cameras) {
            log::error!("Failed to save updated known cameras: {:?}", e);
            // Decide if we should still try to notify or return error
            // For now, log the error and continue notifying
        } else {
            log::info!("Successfully saved updated camera list.");
        }

        log::info!("Notifying {} subscribed chats about new cameras...", chats_to_notify.len());
        for chat_id in chats_to_notify.iter() {
            match bot.send_message(*chat_id, message.clone()).await {
                Ok(_) => log::debug!("Sent new camera notification to chat ID: {}", chat_id),
                Err(e) => log::error!("Failed to send new camera notification to chat ID {}: {:?}", chat_id, e),
            }
            // Add a small delay to avoid hitting rate limits, if necessary
            // tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;
        }
        Ok(format!("Update check complete. Found {} new camera(s). Notifications sent.", new_cameras.len()))

    } else {
        log::info!("No new cameras detected.");
        log::info!("Checking preferences for 'no update' notifications for {} chats...", chats_to_notify.len());

        let message = "Update check complete. No new speed cameras detected.".to_string();

        for chat_id in chats_to_notify.iter() {
            // Get preference, default to false (don't notify if no updates)
            let should_notify = preferences.get(chat_id).copied().unwrap_or(false);
            if should_notify {
                 match bot.send_message(*chat_id, message.clone()).await {
                    Ok(_) => log::debug!("Sent 'no update' notification to chat ID: {}", chat_id),
                    Err(e) => log::error!("Failed to send 'no update' notification to chat ID {}: {:?}", chat_id, e),
                }
                // Add a small delay if needed
                // tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;
            } else {
                log::debug!("Skipping 'no update' notification for chat ID {} based on preferences.", chat_id);
            }
        }
        Ok(message) // Return the status message for the manual trigger
    }
}


// --- Main Application Logic ---

#[tokio::main]
async fn main() -> Result<()> {
    // Load .env file
    match dotenvy::dotenv() {
        Ok(path) => log::info!("Loaded .env file from path: {}", path.display()),
        Err(e) if e.not_found() => log::info!(".env file not found, using system environment variables."),
        Err(e) => log::warn!("Failed to load .env file: {}", e),
    }

    // Initialize logging
    pretty_env_logger::init();
    log::info!("Starting bot...");

    // Initialize Teloxide bot
    let bot = Bot::from_env(); // Reads TELEGRAM_BOT_TOKEN from env
    log::info!("Bot initialized.");

    // Load subscribed chats
    let initial_chats = load_subscribed_chats(SUBSCRIBED_CHATS_FILE_PATH)?;
    log::info!("Loaded {} subscribed chats from {}", initial_chats.len(), SUBSCRIBED_CHATS_FILE_PATH);
    let subscribed_chats: SharedSubscribedChats = Arc::new(Mutex::new(initial_chats));

    // Load user preferences
    let initial_prefs = load_user_preferences(USER_PREFERENCES_FILE_PATH)?;
    log::info!("Loaded preferences for {} users from {}", initial_prefs.len(), USER_PREFERENCES_FILE_PATH);
    let user_preferences: SharedUserPreferences = Arc::new(Mutex::new(initial_prefs));

    // --- Clone shared state for the background task BEFORE moving originals to dispatcher ---
    let bot_clone_for_scheduler = bot.clone();
    let chats_clone_for_scheduler = subscribed_chats.clone();
    let prefs_clone_for_scheduler = user_preferences.clone();

    // --- Set up command handler ---
    // The handler now needs both shared states
    let command_handler = move |bot: Bot, msg: Message, cmd: Command, subscribed_chats: SharedSubscribedChats, user_preferences: SharedUserPreferences| async move {
        handle_command(bot, msg, cmd, subscribed_chats, user_preferences).await
    };

    let mut dispatcher = Dispatcher::builder(bot, Update::filter_message().branch(dptree::entry().filter_command::<Command>().endpoint(command_handler)))
        .dependencies(dptree::deps![subscribed_chats, user_preferences]) // Add user_preferences dependency
        .enable_ctrlc_handler()
        .build();

    // --- Spawn background task for periodic checks ---
    // Use the clones created *before* the dispatcher builder
    tokio::spawn(async move {
        log::info!("Background update checker task started.");
        loop {
            let now = Local::now();
            let hour = now.hour();

            // Downtime between 2:00 AM and 6:59 AM
            if hour >= 2 && hour < 7 {
                log::info!("Bot is in scheduled downtime (2 AM - 7 AM). Checking again at 7 AM.");
                // Calculate duration until 7 AM
                let next_check_time = now.date_naive().and_hms_opt(7, 0, 0).unwrap(); // Today at 7:00:00
                let time_until_7am = next_check_time.signed_duration_since(now.naive_local());

                if let Ok(duration_until_7am) = time_until_7am.to_std() {
                    sleep(duration_until_7am).await;
                } else {
                    // Should not happen unless time goes backwards, but handle defensively
                    log::warn!("Could not calculate sleep duration until 7 AM. Sleeping for 1 hour.");
                    sleep(Duration::from_secs(60 * 60)).await; // Sleep for an hour as a fallback
                }
                continue; // Skip the check and restart the loop
            }

            // --- Perform the check ---
            log::info!("Performing scheduled update check...");
            // Pass the clones into the check function
            match check_for_updates(bot_clone_for_scheduler.clone(), chats_clone_for_scheduler.clone(), prefs_clone_for_scheduler.clone()).await {
                Ok(status) => log::info!("Scheduled check completed: {}", status),
                Err(e) => log::error!("Scheduled check failed: {:?}", e),
            }

            // --- Wait for the next interval ---
            log::info!("Sleeping for 30 minutes until the next scheduled check.");
            sleep(Duration::from_secs(30 * 60)).await; // Sleep for 30 minutes
        }
    });

    // --- Start the command dispatcher ---
    log::info!("Starting command dispatcher...");
    dispatcher.dispatch().await;

    log::info!("Bot stopped.");
    Ok(())
}
