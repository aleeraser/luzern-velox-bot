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
3. **Implement State Management:** Store previously seen cameras to detect new ones in a file.
4. **Implement Telegram Integration:** Initialize the bot and send notification messages via the Telegram API.
5. **Configuration:** Read settings (API token, chat ID) from environment variables or a config file.
6. **Error Handling & Logging:** Implement robust error handling and basic logging.
7. **Build & Deployment:** Document build steps and basic deployment guidance.
8. **Refine dependencies:** Remove unnecessary dependencies.
9. **Implement `/start` command:** Add handler to subscribe users for notifications.
10. **Implement `/current_list` command:** Add handler to display the current list of known cameras.
11. **Implement `/notify_no_updates` command:** Add handler to toggle notifications for checks with no new cameras.
12. **Implement `/manual_update` command:** Add handler to trigger an immediate update check.
13. **Persistent User Storage:** Implement saving/loading of chat IDs and notification preferences.
14. **Systemd Integration:** Check systemd service file and instructions.
15. **Implement Scheduling Logic:** Add polling (every 30 mins) and scheduled downtime (2 AM - 7 AM).

## Setup & Installation

1. **Clone the repository:**

    ```bash
    git clone https://github.com/your-username/luzern-velox-vibebot.git
    cd luzern-velox-vibebot
    ```

2. **Configure Environment Variables:**
    The bot requires your Telegram Bot Token and the target Chat ID. You can provide these using a `.env` file in the project root.
    * Copy the example file:

        ```bash
        cp .env.example .env
        ```

    * Edit the `.env` file and add your actual token and chat ID:

        ```dotenv
        # .env
        TELEGRAM_BOT_TOKEN=YOUR_BOT_TOKEN_HERE
        TELEGRAM_CHAT_ID=YOUR_CHAT_ID_HERE
        # Optional: Set log level (e.g., RUST_LOG=debug)
        ```

    * Alternatively, you can still set these as system environment variables. The `.env` file takes precedence if it exists.
3. **Build the application:**

    ```bash
    cargo build --release
    ```

4. **Run the bot:**

    ```bash
    ./target/release/luzern-velox-vibebot
    ```

## Usage

The bot responds to the following commands:

* `/start`: Enables the bot for the user sending the command. The user's chat ID is saved to a persistent list of subscribed users.
* `/current_list`: Returns a message listing the currently known active speed camera locations.
* `/notify_no_updates`: Toggles notifications for when the update check runs but finds no changes. This preference is stored per user.
* `/manual_update`: Triggers an immediate check for speed camera updates, independent of the regular schedule.

If no commands are used, the bot will automatically send notifications to subscribed users when new speed cameras are detected based on its schedule.

## Systemd Service

For running the bot reliably as a background service on Linux systems, a systemd service unit can be used.

1. **Create the Service File:** Create a file named `luzern-velox-vibebot.service` in `/etc/systemd/system/` with content similar to the example below. You might need `sudo` privileges.
2. **Customize:** Adjust paths (especially `WorkingDirectory` and `ExecStart`) and `User`/`Group` to match your setup. Ensure the specified user has the necessary permissions to run the bot and access its data files (like `known_cameras.json` and the user list).
3. **Environment Variables:** The bot reads configuration from a `.env` file in its `WorkingDirectory` by default. Ensure the `.env` file (created during setup) is present in the `WorkingDirectory` specified below and contains the necessary `TELEGRAM_BOT_TOKEN` and `TELEGRAM_CHAT_ID`. Alternatively, you can set environment variables directly using `Environment="VAR=value"` directives in the service file, or specify a different environment file using `EnvironmentFile=`. The `.env` file in the working directory takes precedence if found.
4. **Enable & Start:**

    ```bash
    sudo systemctl daemon-reload                 # Reload systemd manager configuration
    sudo systemctl enable luzern-velox-vibebot.service # Enable the service to start on boot
    sudo systemctl start luzern-velox-vibebot.service  # Start the service immediately
    sudo systemctl status luzern-velox-vibebot.service # Check the service status
    journalctl -u luzern-velox-vibebot.service -f    # Follow the service logs
    ```

**Example `luzern-velox-vibebot.service`:**

```ini
[Unit]
Description=Luzern Velox Vibebot Telegram Bot
# Start after the network is available
After=network-online.target

[Service]
# User and Group that the service will run as
# Ensure this user has read/write permissions for WorkingDirectory and the executable
User=your_user
Group=your_group

# Set the working directory to the project root
WorkingDirectory=$HOME/git/luzern-velox-vibebot # <-- ADJUST THIS PATH

# Environment variables are typically loaded from the .env file in WorkingDirectory.
# You can override or set them here if needed, or use EnvironmentFile=.
# Example: Environment="RUST_LOG=debug"
# Example: EnvironmentFile=/etc/luzern-velox-vibebot/config

# Command to execute
# Ensure the binary is built and located here
ExecStart=$HOME/git/luzern-velox-vibebot/target/release/luzern-velox-vibebot # <-- ADJUST THIS PATH if needed

# Restart the service if it fails
Restart=on-failure
RestartSec=30s

[Install]
# Enable the service for the default multi-user target
WantedBy=multi-user.target
```

## Contributing

Contributions are welcome! Please feel free to submit pull requests or open issues to improve the bot.

## License

This project is licensed under the [MIT License](LICENSE).
