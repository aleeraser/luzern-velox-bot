#!/usr/bin/python

import argparse
import asyncio
import json
import os
import re
import sys

import requests
from apscheduler.schedulers.asyncio import AsyncIOScheduler
from apscheduler.triggers.cron import CronTrigger
from bs4 import BeautifulSoup
from telegram import Update
from telegram.constants import ParseMode
from telegram.ext import ApplicationBuilder, CommandHandler, ContextTypes

BASE_DIR = os.path.abspath(os.path.dirname(__file__))


# Function to fetch the current list from the website
def fetch_current_dict():
    # Define the URL
    url = 'https://polizei.lu.ch/organisation/sicherheit_verkehrspolizei/verkehrspolizei/spezialversorgung/verkehrssicherheit/Aktuelle_Tempomessungen'  # noqa: E501

    # Make an HTTP request to the URL
    response = requests.get(url, timeout=30)

    # To store the current list
    current_dict = {}

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

        # Extract and store the text content and coordinates
        if a_tag:
            match = re.search(r"map\.flyTo\(\[(.*?),(.*?)\]", a_tag.get('onclick', ''))
            if match:
                lat = match.group(1).strip()
                long = match.group(2).strip()
                velox_url = f"https://www.google.com/maps/search/?api=1&query={lat}%2C{long}"
            else:
                print(f"Error: couldn't retrieve coordinates for {a_tag.text}")
                velox_url = url

            current_dict[a_tag.text] = velox_url

    return current_dict


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
            continue
        await app.bot.send_message(chat_id=chat_id, text=msg,
                                   parse_mode=ParseMode.HTML,
                                   disable_web_page_preview=True)


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

    msg = "Current List\n\n"
    for velox, url in fetch_current_dict().items():
        msg += f"- <a href='{url}'>{velox}</a>\n"

    await context.bot.send_message(chat_id=update.message.chat_id,
                                   text=msg, parse_mode=ParseMode.HTML,
                                   disable_web_page_preview=True)


# Command to handle /manual_update
async def cmd_manual_update(_update: Update,
                            context: ContextTypes.DEFAULT_TYPE):
    return await check_for_updates(context.application, forced_update=True)


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
async def check_for_updates(app=None, save_list=True, forced_update=False):
    # Fetch the current list
    current_dict = fetch_current_dict()
    no_updates = False

    if current_dict is None:
        msg = "Failed to fetch updates."

        print(msg)
        if app:
            await broadcast(app, msg, no_updates=no_updates)

        return

    # Load previous list
    try:
        with open(f'{BASE_DIR}/previous_list.txt', 'r', encoding='utf-8') as f:
            previous_list = json.load(f)
    except (FileNotFoundError, ValueError):
        previous_list = []

    # Compare and find changes
    set_current = set(current_dict.keys())
    set_previous = set(previous_list)
    added = set_current - set_previous
    removed = set_previous - set_current

    # Generate the message to send
    msg = "Checking for updates\n\n"
    if added:
        msg += "Added:\n"
        for el in added:
            msg += f"- <a href='{current_dict[el]}'>{el}</a>\n"
    if removed:
        msg += "Removed:\n"
        for el in removed:
            msg += f"- <a href='{current_dict[el]}'>{el}</a>\n"
    if not added and not removed:
        msg += "No changes detected."
        # mask no_updates flag if forced_update
        no_updates = not forced_update

    print(msg)
    if app:
        await broadcast(app, msg, no_updates=no_updates)

    if save_list:
        # Save the current list for future comparison
        with open(f'{BASE_DIR}/previous_list.txt', 'w', encoding='utf-8') as f:
            json.dump(list(current_dict.keys()), f)


def bot_start():
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

    app.add_handler(CommandHandler("start",
                                   cmd_start))
    app.add_handler(CommandHandler("current_list",
                                   cmd_current_list))
    app.add_handler(CommandHandler("manual_update",
                                   cmd_manual_update))
    app.add_handler(CommandHandler("notify_no_updates",
                                   cmd_set_notify_no_updates))

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


# entry point
# https://www.google.com/maps/@47.0512228,8.3010048,16z?entry=ttu

parser = argparse.ArgumentParser()
parser.add_argument('-t', '--telegram-bot', action='store_true',
                    help='Start the Telegram bot')
parser.add_argument('-s', '--save-list', action='store_true',
                    help='[CLI] Save list when performing an update check')
parser.add_argument('-p', '--print-list', action='store_true',
                    help='[CLI] Print the current list')
args = parser.parse_args()

if args.telegram_bot:
    bot_start()
    sys.exit(0)

# cli section
asyncio.run(check_for_updates(save_list=args.save_list))
if args.print_list:
    print("\nCurrent list:")
    for velox, url in fetch_current_dict().items():
        print(f"{velox}: {url}")
