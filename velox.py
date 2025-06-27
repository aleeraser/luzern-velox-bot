#!/usr/bin/python3

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
from telegram.ext import (Application, ApplicationBuilder, CommandHandler, ContextTypes)

BASE_DIR = os.path.abspath(os.path.dirname(__file__))


def generate_maps_base_url(lat_long_t):
    return f"https://www.google.com/maps/search/?api=1&query={lat_long_t[0]}%2C{lat_long_t[1]}"


def fetch_current_dict():
    """Fetch the current velox list and returns it as a {location_name:maps_url} dict"""
    url = 'https://polizei.lu.ch/organisation/sicherheit_verkehrspolizei/verkehrspolizei/spezialversorgung/verkehrssicherheit/Aktuelle_Tempomessungen'

    response = requests.get(url, timeout=30, headers={'User-Agent': 'Mozilla/5.0'})
    if response.status_code != 200:
        print(f"Failed to make request. Status code: {response.status_code}")
        return None

    soup = BeautifulSoup(response.text, 'html.parser')
    radar_list_div = soup.find('div', {'id': 'radarList'})
    if not radar_list_div:
        print("Could not find div with id 'radarList'")
        return None

    li_tags = radar_list_div.find_all('li')

    # exclude the last <li> tag since it is a (rather useless) link to the map itself
    li_tags = li_tags[:-1]

    current_dict = {}
    for li in li_tags:
        a_tag = li.find('a')

        # extract and store the text content and coordinates
        if a_tag:
            match = re.search(r"map\.flyTo\(\[(.*?),(.*?)\]", a_tag.get('onclick', ''))
            if match:
                lat = match.group(1).strip()
                long = match.group(2).strip()
                # velox_url = f"https://www.google.com/maps/search/?api=1&query={lat}%2C{long}"
            else:
                print(f"Error: couldn't retrieve coordinates for {a_tag.text}")
                lat = long = None

            current_dict[a_tag.text] = (lat, long)

    return current_dict


def save_chats(chat_ids):
    with open(f'{BASE_DIR}/chat_ids.json', 'w', encoding='utf-8') as f:
        json.dump(chat_ids, f, indent=2)


# save a new chat_id
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
            return json.load(f)
    except FileNotFoundError:
        # no previous users
        return None


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


# command to handle /start
async def cmd_start(update: Update,
                    context: ContextTypes.DEFAULT_TYPE):
    chat_id = update.message.chat_id
    newly_subscribed = save_chat_id(chat_id)
    msg = "You're subscribed to updates."
    if not newly_subscribed:
        msg = "Already subscribed."
    await context.bot.send_message(chat_id=chat_id,
                                   text=msg)


# command to handle /current_list
async def cmd_current_list(update: Update,
                           context: ContextTypes.DEFAULT_TYPE):

    msg = "Current List\n\n"
    for velox, lat_long_t in fetch_current_dict().items():
        msg += f"- <a href='{generate_maps_base_url(lat_long_t)}'>{velox}</a>\n"

    await context.bot.send_message(chat_id=update.message.chat_id,
                                   text=msg, parse_mode=ParseMode.HTML,
                                   disable_web_page_preview=True)


# command to handle /show_map
async def cmd_show_map(update: Update,
                       context: ContextTypes.DEFAULT_TYPE):

    url = "https://www.google.com/maps/dir/"
    # hardcoded coords of Luzern for map centering
    url_suffix = "//@47.0473835,8.2532969,12.25z"

    for _, lat_long_t in fetch_current_dict().items():
        url += f"{lat_long_t[0]},{lat_long_t[1]}/"
    url += url_suffix

    msg = f"Velox map\n{url}"

    await context.bot.send_message(chat_id=update.message.chat_id,
                                   text=msg, parse_mode=ParseMode.HTML,
                                   disable_web_page_preview=True)


# command to handle /manual_update
async def cmd_manual_update(_update: Update,
                            context: ContextTypes.DEFAULT_TYPE):
    return await check_for_updates(context.application, forced_update=True)


# command to handle /notify_no_updates
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


async def check_for_updates(app=None, save_list=True, forced_update=False):
    """Check for changes and send updates to registered users"""

    # fetch the current list
    current_dict = fetch_current_dict()
    no_updates = False

    if current_dict is None:
        msg = "Failed to fetch updates."

        print(msg)
        if app:
            await broadcast(app, msg, no_updates=no_updates)

        return

    set_current = set(current_dict.keys())

    # load previous dict
    try:
        with open(f'{BASE_DIR}/previous_dict.json', 'r', encoding='utf-8') as f:
            previous_dict = json.load(f)
            set_previous = set(previous_dict.keys())
    except (FileNotFoundError, ValueError):
        set_previous = set()

    # compare and find changes
    added = set_current - set_previous
    removed = set_previous - set_current

    # generate the message to send
    msg = "Checking for updates\n\n"
    if added:
        msg += "Added:\n"
        for el in added:
            msg += f"- <a href='{generate_maps_base_url(current_dict[el])}'>{el}</a>\n"
    if removed:
        msg += "Removed:\n"
        for el in removed:
            msg += f"- <a href='{generate_maps_base_url(previous_dict[el])}'>{el}</a>\n"
    if not added and not removed:
        msg += "No changes detected."
        # mask no_updates flag if forced_update
        no_updates = not forced_update

    print(msg)
    if app:
        await broadcast(app, msg, no_updates=no_updates)

    if not no_updates and save_list:
        # save the current list
        with open(f'{BASE_DIR}/previous_dict.json', 'w', encoding='utf-8') as f:
            json.dump(current_dict, f)


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
    app.add_handler(CommandHandler("show_map",
                                   cmd_show_map))

    async def post_init(application: Application):
        trigger = CronTrigger(
            year="*", month="*", day="*", hour="*", minute="0", second="0"
        )

        scheduler = AsyncIOScheduler()
        scheduler.add_job(
            check_for_updates,
            trigger=trigger,
            args=[application],
            name="get_velox_list",
        )
        scheduler.start()

    app.post_init = post_init
    app.run_polling()


# entry point

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
    for velox, lat_long_t in fetch_current_dict().items():
        print(f"{velox}: {generate_maps_base_url(lat_long_t)}")
