# Luzern Velox Vibebot 🚨

A Telegram bot that notifies users about newly activated speed cameras in the canton of Luzern, Switzerland.

## Description

This bot monitors the official Luzern Police website for updates on active speed camera locations. When a new camera is detected, it sends a notification to subscribed Telegram users or channels.

The bot is written entirely in Rust for performance and reliability.

## Features

* **Real-time Notifications:** Get alerted when new speed cameras are activated in Luzern.
* **Data Source:** Fetches data directly from the official [Luzern Police website](https://polizei.lu.ch/organisation/sicherheit_verkehrspolizei/verkehrspolizei/spezialversorgung/verkehrssicherheit/Aktuelle_Tempomessungen).
* **Regular Checks:** Polls for updates every 30 minutes during operational hours.
* **Scheduled Downtime:** The bot pauses checks between 2:00 AM and 7:00 AM (local time) to conserve resources.
* **Built with Rust:** Leveraging Rust's safety and performance features.

## How it Works

The bot periodically scrapes the specified Luzern Police webpage. It compares the currently listed speed cameras with the previously known list. If new entries are found, it formats a message and sends it via the Telegram Bot API to the configured chat(s).

## Project Milestones

Here's a breakdown of the planned development steps:

1. **Setup Basic Rust Project:** Initialize the Rust project structure (`cargo new`), add dependencies (`reqwest`, `scraper`, `tokio`, `teloxide`).
2. **Implement Web Scraper:** Fetch and parse HTML from the Luzern Police website to extract speed camera data.
3. **Implement State Management:** Store previously seen cameras to detect new ones (in-memory or file-based).
4. **Implement Telegram Integration:** Initialize the bot and send notification messages via the Telegram API.
5. **Implement Scheduling Logic:** Add polling (every 30 mins) and scheduled downtime (2 AM - 7 AM).
6. **Configuration:** Read settings (API token, chat ID) from environment variables or a config file.
7. **Error Handling & Logging:** Implement robust error handling and basic logging.
8. **Build & Deployment:** Document build steps and basic deployment guidance.
9. **Refine dependencies:** Remove unnecessary dependencies

## Setup & Installation

*(Instructions on how to set up the bot, configure the Telegram API token, chat IDs, etc. will go here. This typically involves cloning the repository, setting environment variables, and building/running the Rust application.)*

```bash
# Example placeholder commands
git clone https://github.com/your-username/luzern-velox-vibebot.git
cd luzern-velox-vibebot
# Set environment variables (e.g., TELEGRAM_BOT_TOKEN, CHAT_ID)
export TELEGRAM_BOT_TOKEN="YOUR_BOT_TOKEN"
export CHAT_ID="YOUR_TARGET_CHAT_ID"
cargo build --release
./target/release/luzern-velox-vibebot
```

## Usage

*(Details on how users interact with the bot, if applicable (e.g., commands like /subscribe, /status). If it's purely notification-based, this section might be brief.)*

Once set up and running, the bot will automatically send notifications to the configured Telegram chat ID when new speed cameras are detected.

## Contributing

Contributions are welcome! Please feel free to submit pull requests or open issues to improve the bot.

## License

*(Specify the license under which this project is released, e.g., MIT, Apache 2.0. If undecided, you can leave this as a placeholder.)*

This project is licensed under the [MIT License](LICENSE).
