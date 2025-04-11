#![allow(unused)]

use anyhow::{anyhow, Context};
use reqwest::IntoUrl;
use serde::Deserialize;

const LOAD_DECK: &str = "https://www.duelingbook.com/php-scripts/load-deck.php";

#[derive(Debug, Deserialize, Hash, PartialEq, Eq)]
pub struct DuelingBookCard {
    pub id: u32,
    pub name: String,
    pub treated_as: String,
    pub effect: String,
    pub pendulum_effect: String,
    pub card_type: String,
    pub monster_color: String,
    pub is_effect: u8,
    #[serde(rename = "type")]
    pub ty: String,
    pub attribute: String,
    pub level: u8,
    pub ability: String,
    pub flip: u8,
    pub pendulum: u8,
    pub scale: u8,
    pub arrows: String,
    pub atk: String,
    pub def: String,
    pub tcg_limit: u8,
    pub ocg_limit: u8,
    pub serial_number: String,
    pub tcg: u8,
    pub ocg: u8,
    pub rush: u8,
    pub pic: String,
    pub hidden: u8,
    pub username: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct DuelingBookDeck {
    pub action: String,
    pub id: u32,
    pub name: String,
    pub main: Vec<DuelingBookCard>,
    pub side: Vec<DuelingBookCard>,
    pub extra: Vec<DuelingBookCard>,
    pub legality: String,
    pub tcg: String,
    pub ocg: String,
    pub links: String,
}

impl DuelingBookDeck {
    // https://www.duelingbook.com/deck?id=16249952
    pub async fn get_deck<T: IntoUrl>(deck_url: T) -> Result<DuelingBookDeck, anyhow::Error> {
        let url = deck_url.into_url()?;
        let mut pairs = url.query_pairs();
        let (_, id) = pairs
            .find(|(k, _v)| k == "id")
            .ok_or_else(|| anyhow!("no id in duelingbook URL"))?;

        let response = reqwest::Client::new()
            .post(LOAD_DECK)
            .multipart(reqwest::multipart::Form::new().text("id", id.to_string()))
            .send()
            .await?;

        let text = response.text().await?;

        serde_json::from_str(&text).with_context(|| format!("While parsing `{text}`"))
    }
}
