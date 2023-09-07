import json

import requests
from bs4 import BeautifulSoup


# Function to fetch the current list from the website
def fetch_current_list():
    # Define the URL
    url = 'https://polizei.lu.ch/organisation/sicherheit_verkehrspolizei/verkehrspolizei/spezialversorgung/verkehrssicherheit/Aktuelle_Tempomessungen'

    # Make an HTTP request to the URL
    response = requests.get(url, timeout=30)

    # To store the current list
    current_list = []

    # Check if the request was successful
    if response.status_code == 200:
        # Parse the HTML content using BeautifulSoup
        soup = BeautifulSoup(response.text, 'html.parser')

        # Find the div with id "radarList"
        radar_list_div = soup.find('div', {'id': 'radarList'})

        if radar_list_div:
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
        else:
            print("Could not find div with id 'radarList'")
            return None
    else:
        print(f"Failed to make request. Status code: {response.status_code}")
        return None


# Fetch the current list from the website
current_list = fetch_current_list()

if current_list is not None:
    # Save to a text file as a JSON array
    with open('current_list.txt', 'w', encoding='utf-8') as f:
        json.dump(current_list, f)

    print("Current list saved.")

    # To store the previous list
    previous_list = []

    # Load the previous list from the text file
    try:
        with open('previous_list.txt', 'r', encoding='utf-8') as f:
            previous_list = json.load(f)
    except FileNotFoundError:
        print("No previous list found.")

    # Convert lists to sets for comparison
    set_current = set(current_list)
    set_previous = set(previous_list)

    # Find added and removed elements
    added = set_current - set_previous
    removed = set_previous - set_current

    # Display the changes
    if added:
        print("Added:\n{}".format('\n- '.join(added)))
    if removed:
        print("Removed:\n{}".format('\n- '.join(removed)))

    if not added and not removed:
        print("No changes detected.")

    # Save the current list as the new previous list
    with open('previous_list.txt', 'w', encoding='utf-8') as f:
        json.dump(current_list, f)
