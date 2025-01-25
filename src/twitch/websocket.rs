use anyhow::Result;
use flume::{Receiver, Sender};
use std::{collections::HashMap, error::Error};

use tokio_tungstenite::tungstenite;
use twitch_api::{
    eventsub::{
        self, stream::StreamOnlineV1Payload, Event, EventsubWebsocketData, Message,
        ReconnectPayload, SessionData, Transport, WelcomePayload,
    },
    twitch_oauth2::{AccessToken, UserToken},
    types::EventSubId,
    HelixClient,
};

use crate::{Action, StreamerMessage};

#[derive(Debug)]
struct Streamer {
    subscriptions: u32,
    id: EventSubId,
}

// This code is mostly copy-pasted from
// https://github.com/twitch-rs/twitch_api/blob/1158b7d840d626af5d68498828e39a209a7f322a/examples/eventsub_websocket/src/websocket.rs
pub struct WebsocketClient {
    pub connect_url: url::Url,
    pub session_id: Option<String>,
    pub token: AccessToken,
    pub client: HelixClient<'static, reqwest::Client>,
    pub streamer_rx: Receiver<StreamerMessage>,
    pub live_rx: Sender<String>,
}
impl WebsocketClient {
    pub async fn connect(
        &self,
    ) -> Result<
        tokio_tungstenite::WebSocketStream<
            tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
        >,
        Box<dyn Error>,
    > {
        let config = tungstenite::protocol::WebSocketConfig {
            max_message_size: Some(64 << 20), // 64 MiB
            max_frame_size: Some(16 << 20),   // 16 MiB
            accept_unmasked_frames: false,
            ..tungstenite::protocol::WebSocketConfig::default()
        };

        let (socket, _) =
            tokio_tungstenite::connect_async_with_config(&self.connect_url, Some(config), false)
                .await
                .expect("Can't connect");

        Ok(socket)
    }

    /// Run the websocket subscriber
    pub async fn run(mut self) -> Result<()> {
        // Establish the stream
        let mut s = self.connect().await.expect("when establishing connection");
        // Loop over the stream, processing messages as they come in.
        loop {
            tokio::select!(
            Some(msg) = futures::StreamExt::next(&mut s) => {
                let msg = match msg {
                    Err(tungstenite::Error::Protocol(
                        tungstenite::error::ProtocolError::ResetWithoutClosingHandshake,
                    )) => {
                        eprintln!(
                            "connection was sent an unexpected frame or was reset, reestablishing it"
                        );
                        s = self
                            .connect()
                            .await
                            .expect("when reestablishing connection");
                        continue
                    }
                    _ => msg.expect("when getting message"),
                };
                self.process_message(msg).await?
            })
        }
    }

    /// Process a message from the websocket
    pub async fn process_message(&mut self, msg: tungstenite::Message) -> Result<()> {
        match msg {
            tungstenite::Message::Text(s) => {
                // Parse the message into a [twitch_api::eventsub::EventsubWebsocketData]
                match Event::parse_websocket(&s)? {
                    EventsubWebsocketData::Welcome {
                        payload: WelcomePayload { session },
                        ..
                    }
                    | EventsubWebsocketData::Reconnect {
                        payload: ReconnectPayload { session },
                        ..
                    } => {
                        self.process_welcome_message(session).await?;
                        Ok(())
                    }
                    // Here is where you would handle the events you want to listen to
                    EventsubWebsocketData::Notification {
                        metadata: _,
                        payload,
                    } => {
                        match payload {
                            Event::StreamOnlineV1(eventsub::Payload {
                                message:
                                    Message::Notification(StreamOnlineV1Payload {
                                        broadcaster_user_name,
                                        ..
                                    }),
                                ..
                            }) => {
                                if let Err(e) = self.live_rx.send(broadcaster_user_name.to_string())
                                {
                                    eprintln!("{}", e);
                                }
                            }
                            _ => {}
                        }
                        Ok(())
                    }
                    EventsubWebsocketData::Revocation {
                        metadata,
                        payload: _,
                    } => {
                        println!("got revocation event: {metadata:?}");
                        Ok(())
                    }
                    EventsubWebsocketData::Keepalive {
                        metadata: _,
                        payload: _,
                    } => Ok(()),
                    _ => Ok(()),
                }
            }
            tungstenite::Message::Close(_) => todo!(),
            _ => Ok(()),
        }
    }

    pub async fn process_welcome_message(&mut self, data: SessionData<'_>) -> Result<()> {
        self.session_id = Some(data.id.to_string());
        if let Some(url) = data.reconnect_url {
            self.connect_url = url.parse()?;
        }
        // check if the token is expired, if it is, request a new token. This only works if using a oauth service for getting a token
        let token = UserToken::from_token(self.client.get_client(), self.token.clone()).await?;
        let transport = eventsub::Transport::websocket(data.id);
        let client = self.client.clone();
        let streamer_rx = self.streamer_rx.clone();
        tokio::spawn(async {
            Self::streamer_listener(client, streamer_rx, transport, token).await
        });
        Ok(())
    }

    async fn streamer_listener(
        client: HelixClient<'static, reqwest::Client>,
        streamer_rx: Receiver<StreamerMessage>,
        transport: Transport,
        token: UserToken,
    ) -> Result<()> {
        let mut subscriptions: HashMap<String, Streamer> = HashMap::new();
        while let Ok((streamer_action, action)) = streamer_rx.recv_async().await {
            match action {
                Action::ADD => {
                    if let Some(streamer) = subscriptions.get_mut(&streamer_action) {
                        streamer.subscriptions += 1;
                    } else {
                        subscriptions.insert(
                            streamer_action.clone(),
                            Streamer {
                                id: client
                                    .create_eventsub_subscription(
                                        eventsub::stream::StreamOnlineV1::broadcaster_user_id(
                                            client
                                                .get_user_from_login(&streamer_action, &token)
                                                .await?
                                                .expect("Could not get user from login {key}")
                                                .id,
                                        ),
                                        transport.clone(),
                                        &token,
                                    )
                                    .await?
                                    .id,
                                subscriptions: 1,
                            },
                        );
                    }
                }
                Action::REMOVE => {
                    if let Some(streamer) = subscriptions.get_mut(&streamer_action) {
                        if streamer.subscriptions == 1 {
                            client
                                .delete_eventsub_subscription(streamer.id.clone(), &token)
                                .await?;
                            subscriptions.remove(&streamer_action);
                        } else {
                            streamer.subscriptions -= 1;
                        }
                    }
                }
            }
        }
        Ok(())
    }
}
