# Luzern Velox Vibebot 🚨

A Telegram bot that notifies users about newly activated speed cameras in the canton of Luzern, Switzerland.

## Description

This bot monitors the official Luzern Police website for updates on active speed camera locations. When a new camera is detected, it sends a notification to subscribed Telegram users or channels.

The bot is written entirely in Rust for performance and reliability.

## Features

* **Real-time Notifications:** Get alerted when new speed cameras are activated in Luzern.
* **User Subscription Management:** Subscribe and unsubscribe from notifications with simple commands.
* **Customizable Notifications:** Toggle "no updates" notifications when checks find no changes.
* **Data Source:** Fetches data directly from the official [Luzern Police website](https://polizei.lu.ch/organisation/sicherheit_verkehrspolizei/verkehrspolizei/spezialversorgung/verkehrssicherheit/Aktuelle_Tempomessungen).
* **Regular Checks:** Polls for updates every 30 minutes during operational hours.
* **Scheduled Downtime:** The bot pauses checks between 2:00 AM and 7:00 AM (local time) to conserve resources.
* **Manual Updates:** Force immediate camera checks on demand.
* **Status Monitoring:** View bot status, subscriber count, and monitoring information.
* **Comprehensive Help:** Built-in help system with all available commands.
* **Built with Rust:** Leveraging Rust's safety and performance features.

## How it Works

The bot periodically scrapes the specified Luzern Police webpage. It compares the currently listed speed cameras with the previously known list. If new entries are found, it formats a message and sends it via the Telegram Bot API to all subscribed users.

Users can subscribe and unsubscribe using simple commands, and customize their notification preferences (such as receiving notifications when no changes are detected). The bot maintains persistent storage of subscriber data and preferences in JSON files.

## Project Milestones

Here's a breakdown of the planned development steps:

1. **Setup Basic Rust Project:** Initialize the Rust project structure (`cargo new`), add dependencies (`reqwest`, `scraper`, `tokio`, `teloxide`).
2. **Implement Web Scraper:** Fetch and parse HTML from the Luzern Police website to extract speed camera data.
3. **Implement State Management:** Store previously seen cameras to detect new ones in a file.
4. **Implement Telegram Integration:** Initialize the bot and send notification messages via the Telegram API.
5. **Configuration:** Read settings (API token) from environment variables or a config file.
6. **Error Handling & Logging:** Implement robust error handling and basic logging.
7. **Build & Deployment:** Document build steps and basic deployment guidance.
8. **Refine dependencies:** Remove unnecessary dependencies.
9. **Implement `/start` command:** Add handler to subscribe users for notifications.
10. **Implement `/current_list` command:** Add handler to display the current list of known cameras.
11. **Implement `/unsubscribe` command:** Add handler to allow users to stop receiving notifications.
12. **Implement `/help` command:** Add handler to show comprehensive help and command information.
13. **Implement `/manual_update` command:** Add handler to trigger an immediate update check.
14. **Implement `/status` command:** Add handler to show bot status, statistics, and monitoring information.
15. **Implement `/notify_no_updates` command:** Add handler to toggle notifications for checks with no new cameras.
16. **Persistent User Storage:** Implement saving/loading of chat IDs and notification preferences.
17. **Systemd Integration:** Check systemd service file and instructions.
18. **Implement Scheduling Logic:** Add polling (every 30 mins) and scheduled downtime (2 AM - 7 AM).

## Setup & Installation

1. **Clone the repository:**

    ```bash
    git clone https://github.com/your-username/luzern-velox-vibebot.git
    cd luzern-velox-vibebot
    ```

2. **Configure Environment Variables:**
    The bot requires your Telegram Bot Token. You can provide this using a `.env` file in the project root.
    * Copy the example file:

        ```bash
        cp .env.example .env
        ```

    * Edit the `.env` file and add your actual token:

        ```dotenv
        # .env
        TELOXIDE_TOKEN=YOUR_BOT_TOKEN_HERE
        # Optional: Set log level (e.g., RUST_LOG=debug)
        # RUST_LOG=info
        ```

    * Alternatively, you can still set this as a system environment variable. The `.env` file takes precedence if it exists.
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

* `/start`: Subscribe to receive speed camera notifications. Enables the bot for the user and saves their chat ID to the persistent subscriber list.
* `/unsubscribe`: Stop receiving speed camera notifications. Removes the user from the subscriber list.
* `/current_list`: Returns a message listing the currently known active speed camera locations.
* `/manual_update`: Triggers an immediate check for speed camera updates, independent of the regular schedule.
* `/notify_no_updates`: Toggles notifications for when the update check runs but finds no changes. This preference is stored per user.
* `/status`: Shows bot status including subscriber count, known camera count, check interval, and current monitoring status.
* `/help`: Shows a comprehensive help message with all available commands and bot features.

If no commands are used, the bot will automatically send notifications to subscribed users when new speed cameras are detected based on its schedule.

## Systemd Service

For running the bot reliably as a background service on Linux systems, a systemd service unit can be used.

1. **Create the Service File:** Create a file named `luzern-velox-vibebot.service` in `/etc/systemd/system/` with content similar to the example below. You might need `sudo` privileges.
2. **Customize:** Adjust paths (especially `WorkingDirectory` and `ExecStart`) and `User`/`Group` to match your setup. Ensure the specified user has the necessary permissions to run the bot and access its data files (like `known_cameras.json` and `subscribers.json`).
3. **Environment Variables:** The bot reads configuration from a `.env` file in its `WorkingDirectory` by default. Ensure the `.env` file (created during setup) is present in the `WorkingDirectory` specified below and contains the necessary `TELOXIDE_TOKEN`. Alternatively, you can set environment variables directly using `Environment="VAR=value"` directives in the service file, or specify a different environment file using `EnvironmentFile=`. The `.env` file in the working directory takes precedence if found.
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
After=network.target

[Service]
# User and Group that the service will run as
# Ensure this user has read/write permissions for WorkingDirectory and the executable
User=your_user
Group=your_group

# Set the working directory to the project root
WorkingDirectory=/home/alessandro/git/luzern-velox-vibebot # <-- ADJUST THIS PATH

# Environment variables (TELEGRAM_BOT_TOKEN, TELEGRAM_CHAT_ID, RUST_LOG)
# are expected to be loaded from the .env file located in the WorkingDirectory.
# Ensure the .env file exists and is configured correctly.
# Alternatively, uncomment and use EnvironmentFile= to specify a different path,
# or use Environment="VAR=value" directives here (less recommended for secrets).
# Example: EnvironmentFile=/etc/luzern-velox-vibebot/config
# Example: Environment="RUST_LOG=debug"

# Command to execute
# Ensure the binary is built and located here
ExecStart=/home/alessandro/git/luzern-velox-vibebot/target/release/luzern-velox-vibebot # <-- ADJUST THIS PATH if needed

# Restart the service if it fails
Restart=on-failure
RestartSec=5s

# Standard output and error logging configuration
StandardOutput=journal
StandardError=journal

[Install]
# Enable the service for the default multi-user target
WantedBy=multi-user.target
```

## Contributing

Contributions are welcome! Please feel free to submit pull requests or open issues to improve the bot.

## License

This project is licensed under the [MIT License](LICENSE). *(You'll need to add a LICENSE file if you choose one)*
