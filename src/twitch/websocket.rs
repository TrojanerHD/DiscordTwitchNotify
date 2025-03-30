use anyhow::Result;
use flume::{Receiver, Sender};
use futures::StreamExt;
use std::{collections::HashMap, error::Error, io};
use tokio::task::JoinHandle;

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

#[derive(Clone)]
struct Streamer {
    subscriptions: u32,
    id: EventSubId,
}
impl Streamer {
    async fn get_streamer(
        streamer_action: String,
        token: &UserToken,
        client: &HelixClient<'static, reqwest::Client>,
        transport: &Transport,
    ) -> Result<Self> {
        Ok(Self {
            id: client
                .create_eventsub_subscription(
                    eventsub::stream::StreamOnlineV1::broadcaster_user_id(
                        client
                            .get_user_from_login(&streamer_action, token)
                            .await?
                            .expect("Could not get user from login {key}")
                            .id,
                    ),
                    transport.clone(),
                    token,
                )
                .await?
                .id,
            subscriptions: 1,
        })
    }
}

type AllRemovedHandle = JoinHandle<Result<Vec<String>>>;

// This code is mostly copy-pasted from
// https://github.com/twitch-rs/twitch_api/blob/1158b7d840d626af5d68498828e39a209a7f322a/examples/eventsub_websocket/src/websocket.rs
pub struct WebsocketClient {
    connect_url: url::Url,
    session_id: Option<String>,
    pub token: AccessToken,
    pub client: HelixClient<'static, reqwest::Client>,
    streamer_rx: Receiver<StreamerMessage>,
    streamer_tx: Sender<StreamerMessage>,
    live_tx: Sender<String>,
    init_streamers: Vec<String>,
}
impl WebsocketClient {
    pub fn new(
        connect_url: url::Url,
        session_id: Option<String>,
        token: AccessToken,
        client: HelixClient<'static, reqwest::Client>,
        streamer_rx: Receiver<StreamerMessage>,
        streamer_tx: Sender<StreamerMessage>,
        live_tx: Sender<String>,
    ) -> Self {
        Self {
            connect_url,
            session_id,
            token,
            client,
            streamer_rx,
            streamer_tx,
            live_tx,
            init_streamers: Vec::new(),
        }
    }

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
    pub async fn run(mut self, streamer: String) -> Result<()> {
        self.init_streamers = vec![streamer];
        // Establish the stream
        let mut s = self.connect().await.expect("when establishing connection");
        let mut all_removed = None;
        // Loop over the stream, processing messages as they come in.
        loop {
            tokio::select!(
                Some(msg) = s.next() => {
                    let msg = match msg {
                        Err(tungstenite::Error::Protocol(
                            tungstenite::error::ProtocolError::ResetWithoutClosingHandshake,
                        )) | Err(tungstenite::Error::Io(io::Error { .. } )) => {
                            eprintln!(
                                "connection was sent an unexpected frame or was reset, reestablishing it"
                            );
                            self.streamer_tx.send_async(("".to_owned(), Action::Abort)).await?; // Can be empty string because the receiver ignores the string on Action::Abort
                            continue
                        }
                        _ => msg.expect("when getting message"),
                    };
                    all_removed = all_removed.or(self.process_message(msg).await?);
                },
                subscriptions = async {
                    if let Some(act_all_removed) = &mut all_removed {
                        act_all_removed.await?
                    } else {
                        std::future::pending().await
                    }
                } => {
                    let Ok(act_subscriptions) = subscriptions else {
                        break;
                    };
                    if act_subscriptions.is_empty() {
                        break;
                    }
                    self.init_streamers = act_subscriptions;
                    s = self
                        .connect()
                        .await
                        .expect("when reestablishing connection");
                },

            )
        }
        s.close(None).await.map_err(|err| err.into())
    }

    /// Process a message from the websocket
    pub async fn process_message(
        &mut self,
        msg: tungstenite::Message,
    ) -> Result<Option<AllRemovedHandle>> {
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
                    } => Ok(Some(self.process_welcome_message(session).await?)),
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
                                if let Err(e) = self.live_tx.send(broadcaster_user_name.to_string())
                                {
                                    eprintln!("{}", e);
                                }
                            }
                            _ => {}
                        }
                        Ok(None)
                    }
                    EventsubWebsocketData::Revocation {
                        metadata,
                        payload: _,
                    } => {
                        println!("got revocation event: {metadata:?}");
                        Ok(None)
                    }
                    EventsubWebsocketData::Keepalive {
                        metadata: _,
                        payload: _,
                    } => Ok(None),
                    _ => Ok(None),
                }
            }
            tungstenite::Message::Close(e) => {
                panic!("Close message received with content: {:?}", e)
            }
            _ => Ok(None),
        }
    }

    // Returns JoinHandle that completes when the websocket shall be closed because all streamers were removed
    pub async fn process_welcome_message(
        &mut self,
        data: SessionData<'_>,
    ) -> Result<AllRemovedHandle> {
        self.session_id = Some(data.id.to_string());
        if let Some(url) = data.reconnect_url {
            self.connect_url = url.parse()?;
        }
        // check if the token is expired, if it is, request a new token. This only works if using a oauth service for getting a token
        let token = UserToken::from_token(self.client.get_client(), self.token.clone()).await?;
        let transport = eventsub::Transport::websocket(data.id);
        let client = self.client.clone();
        let streamer_rx = self.streamer_rx.clone();
        let init_streamers = self.init_streamers.clone();
        Ok(tokio::spawn(async {
            Self::streamer_listener(client, streamer_rx, transport, token, init_streamers).await
        }))
    }

    async fn streamer_listener(
        client: HelixClient<'static, reqwest::Client>,
        streamer_rx: Receiver<StreamerMessage>,
        transport: Transport,
        token: UserToken,
        init_streamers: Vec<String>,
    ) -> Result<Vec<String>> {
        let mut subscriptions: HashMap<String, Streamer> = HashMap::new();
        for init_streamer in init_streamers {
            subscriptions.insert(
                init_streamer.clone(),
                Streamer::get_streamer(init_streamer, &token, &client, &transport).await?,
            );
        }

        while let Ok((streamer_action, action)) = streamer_rx.recv_async().await {
            match action {
                Action::Add => {
                    if let Some(streamer) = subscriptions.get_mut(&streamer_action) {
                        streamer.subscriptions += 1;
                    } else {
                        subscriptions.insert(
                            streamer_action.clone(),
                            Streamer::get_streamer(streamer_action, &token, &client, &transport)
                                .await?,
                        );
                    }
                }
                Action::Remove => {
                    if subscriptions.len() == 1 {
                        break;
                    }
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
                Action::Abort => break,
            }
        }
        Ok(subscriptions.into_keys().collect::<Vec<_>>())
    }
}
