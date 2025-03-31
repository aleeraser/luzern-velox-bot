use reqwest;
use scraper::{Html, Selector};
use tokio;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let url = "https://polizei.lu.ch/organisation/sicherheit_verkehrspolizei/verkehrspolizei/spezialversorgung/verkehrssicherheit/Aktuelle_Tempomessungen";
    println!("Fetching URL: {}", url);

    let response = reqwest::get(url).await?;

    if !response.status().is_success() {
        eprintln!("Failed to fetch URL: {}", response.status());
        return Ok(()); // Or handle error more robustly
    }

    let body = response.text().await?;
    println!("Successfully fetched HTML content.");

    let document = Html::parse_document(&body);

    // Refined selector to capture both lists (semi-stationary and stationary)
    let selector_str = "#radarList li > a";
    let selector = Selector::parse(selector_str).expect("Failed to parse selector");

    println!("\nExtracting all camera locations using selector: '{}'", selector_str);

    let mut found_data = false;
    for element in document.select(&selector) {
        let text = element.text().collect::<Vec<_>>().join(" ").trim().to_string();
        // Filter out the non-camera link
        if !text.is_empty() && text != "Kantonsübersicht zurücksetzen" {
            println!("- {}", text);
            found_data = true;
        }
    }

    if !found_data {
        println!("No data found matching the selector '{}'. The selector might need adjustment or the page structure changed.", selector_str);
    }

    Ok(())
}
