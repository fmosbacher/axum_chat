use std::sync::Arc;

use axum::{
    extract::{
        ws::{Message, WebSocket},
        State, WebSocketUpgrade,
    },
    response::Response,
    routing::get,
    Router, Server,
};
use futures::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use tokio::sync::{
    broadcast::{self, Sender},
    Mutex,
};
use uuid::Uuid;

#[derive(Debug)]
struct Chat {
    broadcast_tx: Sender<WsRequest>,
    rooms: Mutex<Vec<Room>>,
}

impl Chat {
    async fn answer(&mut self, ws_request: &WsRequest) -> WsResponse {
        match ws_request {
            WsRequest::Login(username) => {
                if self
                    .rooms
                    .lock()
                    .await
                    .iter()
                    .any(|r| r.users.iter().any(|u| u == username))
                {
                    return WsResponse::DuplicatedUsername;
                };
                self.rooms.lock().await.push(Room {
                    id: Uuid::new_v4(),
                    users: vec![username.clone()],
                });
                WsResponse::LoggedIn(self.rooms.lock().await.clone())
            }
            WsRequest::SendMessage(..) => WsResponse::MessageSent,
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "type", content = "content")]
enum WsRequest {
    Login(Username),
    SendMessage(String),
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
struct Username(String);

#[derive(Debug, Serialize)]
#[serde(tag = "type", content = "content")]
enum WsResponse {
    LoggedIn(Vec<Room>),
    DuplicatedUsername,
    MessageSent,
}

#[derive(Debug, Clone, Serialize)]
struct Room {
    id: Uuid,
    users: Vec<Username>,
}

#[tokio::main]
async fn main() {
    let (tx, _) = broadcast::channel(16);
    let app_state = Arc::new(Mutex::new(Chat {
        broadcast_tx: tx,
        rooms: Mutex::new(vec![]),
    }));
    let app = Router::new()
        .route("/", get(handle_ws_handshake))
        .with_state(app_state);
    Server::bind(&"127.0.0.1:8000".parse().unwrap())
        .serve(app.into_make_service())
        .await
        .unwrap();
}

async fn handle_ws_handshake(
    ws: WebSocketUpgrade,
    State(state): State<Arc<Mutex<Chat>>>,
) -> Response {
    ws.on_upgrade(|socket| handle_ws(socket, state))
}

async fn handle_ws(socket: WebSocket, chat: Arc<Mutex<Chat>>) {
    let (mut socket_tx, mut socket_rx) = socket.split();
    let broadcast_tx = chat.lock().await.broadcast_tx.clone();
    tokio::spawn(async move {
        while let Some(Ok(Message::Text(text_message))) = socket_rx.next().await {
            let ws_request = serde_json::from_str(&text_message).unwrap();
            broadcast_tx.send(ws_request).unwrap();
        }
    });
    let mut broadcast_rx = chat.lock().await.broadcast_tx.subscribe();
    tokio::spawn(async move {
        while let Ok(ws_request) = broadcast_rx.recv().await {
            let ws_response = chat.lock().await.answer(&ws_request).await;
            let ws_response_text = serde_json::to_string(&ws_response).unwrap();
            let message = Message::Text(ws_response_text);
            socket_tx.send(message).await.unwrap();
        }
    });
}
