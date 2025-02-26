use anyhow::Result;
use flume::{Receiver, Sender};
use std::env;
use tokio::task::JoinHandle;

use twitch_api::{helix::HelixClient, twitch_oauth2};

use crate::StreamerMessage;
// pub fn add_events_for_streamers(streamers: Vec<String>) {}
pub mod websocket;

pub fn run(
    streamer_rx: Receiver<StreamerMessage>,
    live_tx: Sender<String>,
) -> JoinHandle<Result<()>> {
    let client = HelixClient::with_client(<reqwest::Client>::default());
    let token = twitch_oauth2::AccessToken::new(
        env::var("TWITCH_ACCESS_TOKEN").expect("No TWITCH_ACCESS_TOKEN specified"),
    );
    // let app_access_token = twitch_oauth2::AppAccessToken::get_app_access_token(
    //     &client,
    //     twitch_oauth2::ClientId::new(env::var("TWITCH_ID").expect("No TWITCH_ID specified")),
    //     twitch_oauth2::ClientSecret::new(
    //         env::var("TWITCH_SECRET").expect("No TWITCH_SECRET specified"),
    //     ),
    //     Vec::new(),
    // )
    // .await?;

    let websocket_client = tokio::spawn(async move {
        while let Ok((streamer_action, _action)) = streamer_rx.recv_async().await {
            let websocket_client = websocket::WebsocketClient::new(
                twitch_api::TWITCH_EVENTSUB_WEBSOCKET_URL.clone(),
                None,
                token.clone(),
                client.clone(),
                streamer_rx.clone(),
                live_tx.clone(),
            );
            websocket_client.run(streamer_action).await?;
        }
        Ok(())
    });

    websocket_client
}
