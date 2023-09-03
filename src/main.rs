mod kinda_db;
mod bot_flow;

use async_openai::Client;
use config::Config;
use dotenv::dotenv;
use serde::Deserialize;
use teloxide::{Bot, dptree};
use teloxide::prelude::Dispatcher;
use teloxide::types::ChatId;
use crate::kinda_db::KindaDb;

#[derive(Deserialize, Clone)]
pub struct AppConfig {
    admin_id: ChatId,
    db_path: String,
}

#[tokio::main]
async fn main() {
    dotenv().ok();
    pretty_env_logger::init();

    log::info!("reading cfg, loading state, doing initialization mumbo-jumbo...");

    let config: AppConfig = Config::builder()
        .add_source(
            config::Environment::with_prefix("APP")
                .try_parsing(true)
                .separator("__"),
        )
        .build()
        .unwrap()
        .try_deserialize()
        .unwrap();

    let db = KindaDb::new(config.db_path.clone()).await;
    let bot = Bot::from_env();
    let gpt_client = Client::new();

    log::info!("councillor bot started...");

    Dispatcher::builder(bot, bot_flow::schema())
        .dependencies(dptree::deps![db, gpt_client, config])
        .enable_ctrlc_handler()
        .build()
        .dispatch()
        .await;

    log::info!("councillor bot stopped...");
}
