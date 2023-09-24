#!/usr/bin/python

import json
import os
import sys

import requests
from apscheduler.schedulers.asyncio import AsyncIOScheduler
from apscheduler.triggers.cron import CronTrigger
from bs4 import BeautifulSoup
from telegram import Update
from telegram.ext import ApplicationBuilder, CommandHandler, ContextTypes

BASE_DIR = os.path.abspath(os.path.dirname(__file__))


# Function to fetch the current list from the website
def fetch_current_list():
    # Define the URL
    url = 'https://polizei.lu.ch/organisation/sicherheit_verkehrspolizei/verkehrspolizei/spezialversorgung/verkehrssicherheit/Aktuelle_Tempomessungen'  # noqa: E501

    # Make an HTTP request to the URL
    response = requests.get(url, timeout=30)

    # To store the current list
    current_list = []

    # Check if the request was successful
    if response.status_code != 200:
        print(f"Failed to make request. Status code: {response.status_code}")
        return None

    # Parse the HTML content using BeautifulSoup
    soup = BeautifulSoup(response.text, 'html.parser')

    # Find the div with id "radarList"
    radar_list_div = soup.find('div', {'id': 'radarList'})

    if not radar_list_div:
        print("Could not find div with id 'radarList'")
        return None

    # Find all the <li> tags within the div
    li_tags = radar_list_div.find_all('li')

    # Exclude the last <li> tag
    li_tags = li_tags[:-1]

    # Loop through each <li> tag
    for li in li_tags:
        # Find the inner <a> tag
        a_tag = li.find('a')

        # Extract and store the text content
        if a_tag:
            current_list.append(a_tag.text)

    return current_list


def save_chats(chat_ids):
    with open(f'{BASE_DIR}/chat_ids.json', 'w', encoding='utf-8') as f:
        json.dump(chat_ids, f, indent=2)


# Save new chat_id to a text file
def save_chat_id(chat_id):
    chat_id = str(chat_id)

    try:
        with open(f'{BASE_DIR}/chat_ids.json', 'r', encoding='utf-8') as f:
            chat_ids = json.load(f)
    except FileNotFoundError:
        chat_ids = {}

    if chat_id in chat_ids.keys():
        return False

    print(f"New chat id {chat_id}")
    chat_ids[chat_id] = {"notify_for_no_updates": False}

    save_chats(chat_ids)
    return True


def get_chats():
    try:
        with open(f'{BASE_DIR}/chat_ids.json', 'r', encoding='utf-8') as f:
            chats = json.load(f)
    except FileNotFoundError:
        return None  # No chats saved

    return chats


def should_notify_no_updates(chat_id):
    chat_id = str(chat_id)
    chat_ids = get_chats()

    if not chat_ids:
        return False

    return chat_ids[chat_id].get("notify_for_no_updates", False)


async def broadcast(app, msg, no_updates):
    chat_ids = get_chats()

    if not chat_ids:
        return None

    for chat_id in chat_ids.keys():
        chat_id = str(chat_id)
        if no_updates and not should_notify_no_updates(chat_id):
            pass
        await app.bot.send_message(chat_id=chat_id, text=msg)


# Command to handle /start
async def cmd_start(update: Update,
                    context: ContextTypes.DEFAULT_TYPE):
    chat_id = update.message.chat_id
    newly_subscribed = save_chat_id(chat_id)
    msg = "You're subscribed to updates."
    if not newly_subscribed:
        msg = "Already subscribed."
    await context.bot.send_message(chat_id=chat_id,
                                   text=msg)


# Command to handle /current_list
async def cmd_current_list(update: Update,
                           context: ContextTypes.DEFAULT_TYPE):
    current_list = fetch_current_list()
    formatted_list = "\n".join(current_list)
    await context.bot.send_message(chat_id=update.message.chat_id,
                                   text=f"Current List:\n{formatted_list}")


# Command to handle /manual_update
async def cmd_manual_update(_update: Update,
                            context: ContextTypes.DEFAULT_TYPE):
    return await check_for_updates(context.application)


# Command to handle /notify_no_updates
async def cmd_set_notify_no_updates(update: Update,
                                    context: ContextTypes.DEFAULT_TYPE):
    chat_id = str(update.message.chat_id)
    chat_ids = get_chats()

    if not chat_ids:
        return None

    new_val = not chat_ids[chat_id]["notify_for_no_updates"]
    chat_ids[chat_id]["notify_for_no_updates"] = new_val

    save_chats(chat_ids)

    msg = "Disabled - no status updates if no changes are detected"
    if new_val:
        msg = "Enabled - get status updates even if no changes are detected"

    await context.bot.send_message(chat_id=update.message.chat_id,
                                   text=f"{msg}")


# Check for changes and send updates to Telegram
async def check_for_updates(app):
    # Fetch the current list
    current_list = fetch_current_list()
    no_updates = False

    if current_list is None:
        msg = "Failed to fetch updates."
        await broadcast(app, msg, no_updates=no_updates)
        return

    # Load previous list
    try:
        with open(f'{BASE_DIR}/previous_list.txt', 'r', encoding='utf-8') as f:
            previous_list = json.load(f)
    except FileNotFoundError:
        previous_list = []

    # Compare and find changes
    set_current = set(current_list)
    set_previous = set(previous_list)
    added = set_current - set_previous
    removed = set_previous - set_current

    # Generate the message to send
    msg = "Checking for updates:\n"
    if added:
        msg += "Added:\n- " + "\n- ".join(added) + "\n"
    if removed:
        msg += "Removed:\n- " + "\n- ".join(removed) + "\n"
    if not added and not removed:
        msg += "No changes detected."
        no_updates = True

    await broadcast(app, msg, no_updates=no_updates)

    # Save the current list for future comparison
    with open(f'{BASE_DIR}/previous_list.txt', 'w', encoding='utf-8') as f:
        json.dump(current_list, f)

# get the token from config.json
try:
    with open(f'{BASE_DIR}/config.json', 'r', encoding='utf-8') as f:
        configs = json.load(f)
except (FileNotFoundError, ValueError):
    with open(f'{BASE_DIR}/config.json', 'w', encoding='utf-8') as f:
        configs = {"BOT_TOKEN": ""}
        json.dump(configs, f, indent=2)

    print("Error: no valid config.json found. A template has been created,"
          "but you need to fill your bot's token.")
    sys.exit(1)

if configs["BOT_TOKEN"] in (None, ""):
    print("Error: no BOT_TOKEN in config.json. Please add it.")
    sys.exit(1)

app = ApplicationBuilder().token(configs["BOT_TOKEN"]).build()

app.add_handler(CommandHandler("start", cmd_start))
app.add_handler(CommandHandler("current_list", cmd_current_list))
app.add_handler(CommandHandler("manual_update", cmd_manual_update))
app.add_handler(CommandHandler("notify_no_updates", cmd_set_notify_no_updates))

trigger = CronTrigger(
    year="*", month="*", day="*", hour="8,16", minute="0", second="0"
)

scheduler = AsyncIOScheduler()
scheduler.start()

scheduler.add_job(
    check_for_updates,
    trigger=trigger,
    args=[app],
    name="get_velox_list",
)

app.run_polling()
