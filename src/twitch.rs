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
    live_rx: Sender<String>,
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
    let websocket_client = websocket::WebsocketClient {
        connect_url: twitch_api::TWITCH_EVENTSUB_WEBSOCKET_URL.clone(),
        session_id: None,
        token,
        client,
        streamer_rx,
        live_rx,
    };

    let websocket_client = tokio::spawn(async move { websocket_client.run().await });

    websocket_client
}
