#![deny(unused)]

use std::{
    collections::{HashMap, HashSet},
    future::Future,
    pin::Pin,
    sync::OnceLock,
};

use chrono::Utc;
use duelingbook::DuelingBookCard;
use poise::{
    serenity_prelude::{self as serenity, CreateAllowedMentions, CreateMessage},
    CreateReply,
};
use regex::Regex;
use serde::{Deserialize, Serialize};
use tokio::sync::{RwLock, RwLockReadGuard};
use tracing_subscriber::{layer::SubscriberExt as _, Layer as _, Registry};

mod duelingbook;

const CARD_DATA: &str = "https://theplunderpirates.cc/card_data.json";
const IMG_BASE: &str = "https://theplunderpirates.cc/card_images/";

#[derive(Debug, Clone, Deserialize, Serialize)]
struct CardDatum {
    name: String,
    full_type: String,
    race: String,
    desc: String,
    #[serde(rename = "frameType")]
    frame_type: String,
    archetype: String,
    image_url: String,
    #[serde(rename = "type")]
    ty: String,
    category: Option<Vec<String>>,
    attribute: Option<String>,
    atk: Option<i32>,
    def: Option<i32>,
    number_value: Option<u32>,
    level: Option<u32>,
    linkval: Option<u32>,
}

struct FreshData<D> {
    frequency: chrono::Duration,
    refresh: fn() -> Pin<Box<dyn Future<Output = D> + Send>>,
    data: RwLock<(chrono::DateTime<Utc>, D)>,
}

impl<D> FreshData<D> {
    async fn new(
        frequency: chrono::Duration,
        refresh: fn() -> Pin<Box<dyn Future<Output = D> + Send>>,
    ) -> FreshData<D> {
        let data = (refresh)().await;
        FreshData {
            frequency,
            refresh,
            data: RwLock::new((Utc::now() + frequency, data)),
        }
    }

    async fn get(&self) -> RwLockReadGuard<'_, D> {
        {
            let mut lock = self.data.write().await;
            if Utc::now() >= lock.0 {
                tracing::info!("Refreshing data!");
                lock.1 = (self.refresh)().await;
                lock.0 = Utc::now() + self.frequency;
            }
        }
        RwLockReadGuard::map(self.data.read().await, |(_, d)| d)
    }
}

struct Data {
    cards: FreshData<Vec<CardDatum>>,
}

static DISALLOWED_CHARACTERS: OnceLock<Regex> = OnceLock::new();
fn disallowed_characters() -> &'static Regex {
    DISALLOWED_CHARACTERS.get_or_init(|| {
        Regex::new(r#"[?/'!,:&."]"#).expect("Cannot compile disallowed characters re")
    })
}

static WHITESPACE: OnceLock<Regex> = OnceLock::new();
fn ws() -> &'static Regex {
    WHITESPACE.get_or_init(|| Regex::new(r"\s+").expect("Cannot compile whitespace re"))
}

impl CardDatum {
    fn make_embed(&self) -> serenity::CreateEmbed {
        let removed_disallowed = disallowed_characters().replace_all(&self.name, "_");
        let formatted_name = ws().replace_all(&removed_disallowed, "%20");
        let img_url = format!("{IMG_BASE}{formatted_name}.jpg");
        let mut embed = serenity::CreateEmbed::new()
            .title(self.name.clone())
            .description(self.desc.clone())
            .footer(
                serenity::CreateEmbedFooter::new("The Plunder Pirates")
                    .icon_url("https://theplunderpirates.cc/icon/apple-touch-icon.png"),
            )
            .url({
                let mut url =
                    reqwest::Url::parse("https://theplunderpirates.cc").expect("must be valid");
                url.set_query(Some(&format!("current_card={}", self.name)));
                url.to_string()
            })
            .color(match &self.frame_type[..] {
                "effect" => serenity::Color::ORANGE,
                "fusion" => serenity::Color::PURPLE,
                "link" => serenity::Color::DARK_BLUE,
                "spell" => serenity::Color::TEAL,
                "synchro" | "synchro_pendulum" => serenity::Color::from_rgb(255, 255, 255),
                "trap" => serenity::Color::MAGENTA,
                "xyz" => serenity::Color::from_rgb(0, 0, 0),

                _ => serenity::Color::DARK_GREY,
            })
            .image(img_url)
            .field("Type", self.full_type.clone(), true)
            .field(self.ty.replace("self", "Type"), self.race.clone(), true);
        if let Some(attr) = &self.attribute {
            embed = embed.field("Attribute", attr.clone(), true);
        }
        if let Some(level) = &self.level {
            embed = embed.field("Level", level.to_string(), true);
        }
        if let Some(linkval) = &self.linkval {
            embed = embed.field("Link Value", linkval.to_string(), true);
        }
        if let Some((atk, def)) = self.atk.zip(self.def) {
            embed = embed.field(
                "Atk/Def",
                format!(
                    "{} / {}",
                    if atk < 0 {
                        "?".to_string()
                    } else {
                        atk.to_string()
                    },
                    if def < 0 {
                        "?".to_string()
                    } else {
                        def.to_string()
                    }
                ),
                true,
            );
        }
        embed
    }
}

fn normalize_search_term(term: &str) -> String {
    disallowed_characters()
        .replace_all(term, "_")
        .to_lowercase()
}

impl Data {
    async fn filter_cards(&self, name: Option<&str>, effect: Option<&str>) -> Vec<CardDatum> {
        let cards = self.cards.get().await;
        let name = normalize_search_term(name.unwrap_or_default());
        let effect = normalize_search_term(effect.unwrap_or_default());

        cards
            .iter()
            .filter(|card| {
                let card_name = normalize_search_term(&card.name);
                for term in name.split("*") {
                    if !card_name.contains(term.trim()) {
                        return false;
                    }
                }
                true
            })
            .filter(|card| {
                let card_desc = normalize_search_term(&card.desc);
                for term in effect.split("*") {
                    if !card_desc.contains(term.trim()) {
                        return false;
                    }
                }
                true
            })
            .cloned()
            .collect()
    }

    async fn get_reply(
        &self,
        card_name: Option<&str>,
        effect: Option<&str>,
    ) -> Result<serenity::CreateEmbed, anyhow::Error> {
        let mut cards = self.filter_cards(card_name, effect).await;
        cards.sort_by(|a, b| a.name.cmp(&b.name));

        Ok(match cards.len() {
            0 => serenity::CreateEmbed::new()
                .title("No cards found".to_string())
                .description("No cards were found that match the provided filters"),
            1 => cards[0].make_embed(),
            _ => serenity::CreateEmbed::new()
                .title("Multiple matches found".to_string())
                .description(format!(
                    "Did you mean: \n- {}",
                    cards
                        .into_iter()
                        .take(25)
                        .map(|card| card.name.to_string())
                        .collect::<Vec<_>>()
                        .join("\n- ")
                )),
        })
    }
}

type Context<'a> = poise::Context<'a, Data, anyhow::Error>;

async fn event_handler(
    ctx: &serenity::Context,
    event: &serenity::FullEvent,
    _framework: poise::FrameworkContext<'_, Data, anyhow::Error>,
    data: &Data,
) -> Result<(), anyhow::Error> {
    if let serenity::FullEvent::Message { new_message } = event {
        if let Some(msg) = new_message
            .content
            .strip_prefix("<")
            .and_then(|msg| msg.strip_suffix(">"))
        {
            if !msg.starts_with("@") {
                let builder = CreateMessage::new()
                    .add_embed(data.get_reply(Some(msg), None).await?)
                    .reference_message(new_message)
                    .allowed_mentions(
                        CreateAllowedMentions::new()
                            .replied_user(false)
                            .everyone(true)
                            .all_users(true)
                            .all_roles(true),
                    );
                new_message.channel_id.send_message(ctx, builder).await?;
            }
        }
    }
    Ok(())
}

async fn autocomplete_search(ctx: Context<'_>, partial: &str) -> Vec<String> {
    let mut result = ctx
        .data()
        .cards
        .get()
        .await
        .iter()
        .filter(|c| c.name.to_lowercase().contains(&partial.to_lowercase()))
        .map(|c| c.name.clone())
        .collect::<Vec<_>>();
    result.sort();
    result
}

#[poise::command(slash_command)]
/// Search for a card in the TPP format. Separate search terms with *.
async fn search(
    ctx: Context<'_>,
    #[description = "Card Name"]
    #[autocomplete = autocomplete_search]
    name: Option<String>,
    #[description = "Card Effect"] effect: Option<String>,
) -> Result<(), anyhow::Error> {
    ctx.send(
        CreateReply::default().embed(
            ctx.data()
                .get_reply(name.as_deref(), effect.as_deref())
                .await?,
        ),
    )
    .await?;
    Ok(())
}

#[poise::command(slash_command)]
/// Verify that the provided deck is valid in the TPP format.
async fn check_deck(
    ctx: Context<'_>,
    #[description = "Deck URL in the format https://www.duelingbook.com/deck?id=<id>"] url: String,
) -> Result<(), anyhow::Error> {
    ctx.defer_ephemeral().await?;

    let data = ctx.data().cards.get().await;
    let valid_cards: HashSet<_> = data.iter().map(|c| &c.name).cloned().collect();
    let deck = duelingbook::DuelingBookDeck::get_deck(url).await?;

    fn track_invalids<'a>(
        valid_cards: &HashSet<String>,
        cards: &'a Vec<DuelingBookCard>,
    ) -> (HashMap<&'a DuelingBookCard, usize>, usize) {
        let mut invalids = HashMap::new();
        let mut invalid_count = 0;
        for card in cards {
            if !valid_cards.contains(&card.name) {
                *invalids.entry(card).or_default() += 1;
                invalid_count += 1;
            }
        }
        (invalids, invalid_count)
    }

    let add_invalids = |name: &str,
                        invalids: HashMap<&DuelingBookCard, usize>,
                        invalid_count: usize,
                        msg: &mut Vec<String>| {
        if !invalids.is_empty() {
            msg.push(format!(
                "## {name} deck includes {invalid_count} invalid cards:"
            ));
            let mut entries: Vec<_> = invalids.into_iter().collect();
            entries.sort_by_key(|(c, _)| &c.name);
            for (card, count) in entries {
                msg.push(format!(
                    "- **{}**{} x {count}",
                    card.name,
                    if let Some(name) = &card.username {
                        format!(" *(custom by: {})*", name)
                    } else {
                        String::new()
                    }
                ));
            }
        }
    };

    let (invalid_main, main_count) = track_invalids(&valid_cards, &deck.main);
    let (invalid_side, side_count) = track_invalids(&valid_cards, &deck.side);
    let (invalid_extra, extra_count) = track_invalids(&valid_cards, &deck.extra);

    let msg = if invalid_main.is_empty() && invalid_side.is_empty() && invalid_extra.is_empty() {
        "This deck is valid.".to_string()
    } else {
        let mut msg = vec![format!(
            "# This deck has the following {} invalid cards:",
            main_count + side_count + extra_count
        )];
        add_invalids("Main", invalid_main, main_count, &mut msg);
        add_invalids("Side", invalid_side, side_count, &mut msg);
        add_invalids("Extra", invalid_extra, extra_count, &mut msg);
        msg.join("\n")
    };
    ctx.reply(msg).await?;
    Ok(())
}

fn setup_tracing() -> Result<(), anyhow::Error> {
    let appender = tracing_appender::rolling::RollingFileAppender::builder()
        .max_log_files(10)
        .filename_prefix("rolling")
        .filename_suffix("log")
        .rotation(tracing_appender::rolling::Rotation::DAILY)
        .build("logs")?;

    let subscriber = Registry::default()
        .with(
            // Stdout
            tracing_subscriber::fmt::layer()
                .compact()
                .with_ansi(true)
                .with_filter(tracing::level_filters::LevelFilter::from_level(
                    tracing::Level::INFO,
                )),
        )
        .with(
            // Rolling logs
            tracing_subscriber::fmt::layer()
                .json()
                .with_writer(appender)
                .with_filter(
                    tracing_subscriber::filter::Targets::new()
                        .with_target("tpp-bot-beard", tracing::Level::TRACE)
                        .with_default(tracing::Level::DEBUG),
                ),
        );

    tracing::subscriber::set_global_default(subscriber)?;

    Ok(())
}

#[tokio::main]
async fn main() -> Result<(), anyhow::Error> {
    setup_tracing()?;
    dotenv::dotenv()?;

    let token = std::env::var("DISCORD_TOKEN")?;
    let intents =
        serenity::GatewayIntents::non_privileged() | serenity::GatewayIntents::MESSAGE_CONTENT;

    let framework = poise::Framework::builder()
        .options(poise::FrameworkOptions {
            commands: vec![search(), check_deck()],
            event_handler: |ctx, event, framework, data| {
                Box::pin(event_handler(ctx, event, framework, data))
            },

            ..Default::default()
        })
        .setup(|ctx, _ready, framework| {
            Box::pin(async move {
                poise::builtins::register_globally(ctx, &framework.options().commands).await?;
                Ok(Data {
                    cards: FreshData::new(chrono::Duration::minutes(15), || {
                        Box::pin(async move {
                            reqwest::get(CARD_DATA)
                                .await
                                .expect("Could not fetch new card data")
                                .json()
                                .await
                                .expect("could not decode card data")
                        })
                    })
                    .await,
                })
            })
        })
        .build();
    let mut client = serenity::ClientBuilder::new(token, intents)
        .framework(framework)
        .await?;
    client.start().await?;

    Ok(())
}
