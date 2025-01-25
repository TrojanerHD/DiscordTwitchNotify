use anyhow::{Context, Result};
use std::{collections::HashMap, env};

use flume::{Receiver, Sender};
use poise::{
    samples::register_in_guild,
    serenity_prelude::{
        futures::lock::Mutex, ChannelId, ClientBuilder, CreateMessage, FullEvent, GatewayIntents,
        GuildId,
    },
    Framework, FrameworkOptions,
};

#[derive(Debug)]
pub enum Action {
    ADD,
    REMOVE,
}

type StreamerMessage = (String, Action);

#[derive(serde::Deserialize, serde::Serialize, Clone)]
pub struct GuildConfig {
    streamers: Vec<String>,
    channel: Option<ChannelId>,
}

type GuildsConfig = HashMap<GuildId, GuildConfig>;

pub struct Cache {
    guilds: GuildsConfig,
}

pub struct ContextData {
    cache: Mutex<Cache>,
    streamer_tx: Sender<StreamerMessage>,
    live_rx: Receiver<String>,
}

pub mod commands;
pub mod store;
pub mod twitch;
#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<()> {
    dotenvy::dotenv()?;

    let (streamer_tx, streamer_rx) = flume::unbounded();
    let (live_tx, live_rx) = flume::unbounded();

    let token = env::var("DISCORD_TOKEN").expect("No DISCORD_TOKEN in env provided");
    let intents = GatewayIntents::non_privileged();

    let framework = Framework::builder()
        .options(FrameworkOptions {
            commands: vec![commands::streamer(), commands::channel()],
            event_handler: |ctx, event, framework, context_data| {
                Box::pin(async move {
                    match event {
                        FullEvent::GuildCreate { guild, is_new } => {
                            if is_new.is_none_or(|new| new) {
                                register_in_guild(ctx, &framework.options().commands, guild.id)
                                    .await
                                    .with_context(|| format!("Could not add commands to guild {}", guild.id))?
                            }
                        }
                        FullEvent::Ready { data_about_bot } => {
                            for guild in data_about_bot.guilds.clone() {
                                register_in_guild(ctx, &framework.options().commands, guild.id)
                                    .await
                                    .with_context(|| format!("Could not add commands to guild {}", guild.id))?
                            }
                            while let Ok(message) = context_data.live_rx.recv_async().await {
                                let cache = context_data.cache.lock().await;
                                for config in cache.guilds.values() {
                                    if config.streamers.iter().any(|streamer| {
                                        streamer.to_lowercase() == message.to_lowercase()
                                    }) {
                                        if let Some(config_channel) = config.channel {
                                            if let Ok(channel) =
                                                ctx.http.get_channel(config_channel).await
                                            {
                                                channel.id().send_message(ctx, CreateMessage::new().content(
                                                    format!("The streamer {} is now live at https://twitch.tv/{}", message, message)),
                                                ).await?;
                                            }
                                        }
                                    }
                                }
                                drop(cache);
                                // if let Some(config) = cache.guilds.get(&guild.id) {
                                //     if config.streamers.iter().any(|streamer| {
                                //         streamer.to_lowercase() == message.to_lowercase()
                                //     }) {
                                //         ctx.http.get_channel(channel_id);
                                //     }
                                // }
                            }
                        }
                        _ => {}
                    };
                    Ok(())
                })
            },
            ..Default::default()
        })
        .setup(|_ctx, _ready, _framework| {
            Box::pin(async {
                let config = store::retrieve_config().expect("Could not read stored config");
                let cache = Mutex::new(Cache {
                    guilds: config.clone(),
                });

                for streamer in config
                    .iter()
                    .flat_map(|(_key, value)| value.streamers.clone())
                {
                    streamer_tx.send_async((streamer, Action::ADD)).await?
                }
                // streamer_tx.send(value)
                Ok(ContextData {
                    cache,
                    streamer_tx,
                    live_rx,
                })
            })
        })
        .build();

    let client = ClientBuilder::new(token, intents)
        .framework(framework)
        .await;

    let discord = tokio::spawn(async move {
        client
            .expect("Could not get client")
            .start()
            .await
            .map_err(|err| err.into())
    });
    let websocket = twitch::run(streamer_rx, live_tx);
    tokio::try_join!(flatten(websocket), flatten(discord))?;
    Ok(())
}

async fn flatten<T>(handle: tokio::task::JoinHandle<Result<T>>) -> Result<T> {
    match handle.await {
        Ok(Ok(result)) => Ok(result),
        Ok(Err(err)) => Err(err),
        Err(e) => Err(e).expect("handling failed"),
    }
}
