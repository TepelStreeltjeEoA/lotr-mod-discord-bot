pub mod api;
pub mod checks;
pub mod commands;
pub mod constants;
pub mod database;
pub mod user_data;
pub mod utils;

pub use poise::serenity_prelude as serenity;
pub use sqlx::mysql;

use std::env;

pub use user_data::{Context, Data, Error, Result};

#[tokio::main]
async fn main() -> Result {
    let token = env::var("DISCORD_TOKEN").expect("Expected a token in the environment");
    let db_uri = env::var("DATABASE_URL").expect("Expected a DB_URI environment variable");

    let framework_options = poise::FrameworkOptions {
        owners: [constants::OWNER_ID].into(),
        allowed_mentions: None,
        ..Default::default()
    };

    let (framework, client) = poise::Framework::<Data, Error>::build()
        .token(token)
        .user_data_setup(|ctx, ready, framework| Box::pin(Data::new(ctx, ready, framework, db_uri)))
        .options(framework_options)
        .command(commands::discord(), |f| f)
        .command(commands::minecraft::online(), |f| {
            f.category("Minecraft Server Commands")
        })
        .command(commands::minecraft::ip(), |f| {
            f.category("Minecraft Server Commands")
                .subcommand(commands::minecraft::set(), |f| f)
                .subcommand(commands::minecraft::display(), |f| f)
        })
        .build()
        .await?;

    {
        // Ctrl+C listener
        let shard_manager = framework.shard_manager();
        tokio::spawn(async move {
            tokio::signal::ctrl_c().await.unwrap();
            println!("Shutting down...");
            shard_manager.lock().await.shutdown_all().await;
        });
    }

    #[cfg(unix)]
    {
        // Sigterm listener
        let shard_manager = framework.shard_manager();
        tokio::spawn(async move {
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
                .unwrap()
                .recv()
                .await
                .unwrap();
            println!("Shutting down...");
            shard_manager.lock().await.shutdown_all().await;
        });
    }

    framework.start(client).await?;

    Ok(())
}
