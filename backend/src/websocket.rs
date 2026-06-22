use crate::alarm_ws::AlarmWsService;
use crate::metrics;
use crate::models::WebSocketMessage;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::State;
use axum::response::IntoResponse;
use futures_util::{SinkExt, StreamExt};
use std::sync::Arc;
use tracing::{debug, error, info};

pub async fn websocket_handler(
    ws: WebSocketUpgrade,
    State(state): State<AppState>,
) -> impl IntoResponse {
    let alarm_manager = state.alarm_manager.clone();
    ws.on_upgrade(|socket| handle_socket(socket, alarm_manager))
}

async fn handle_socket<AM: WsSenderProvider>(socket: WebSocket, alarm_manager: AM) {
    let (mut sender, mut receiver) = socket.split();
    let mut rx = alarm_manager.broadcast_rx();

    info!("新WebSocket客户端已连接");
    metrics::ws_client_connected();

    let send_task = tokio::spawn(async move {
        while let Ok(msg) = rx.recv().await {
            let json = match serde_json::to_string(&msg) {
                Ok(j) => j,
                Err(e) => {
                    error!("序列化WebSocket消息失败: {}", e);
                    continue;
                }
            };

            if let Err(e) = sender.send(Message::Text(json.into())).await {
                debug!("WebSocket发送失败: {}", e);
                break;
            }
        }
    });

    let recv_task = tokio::spawn(async move {
        while let Some(Ok(msg)) = receiver.next().await {
            match msg {
                Message::Text(text) => {
                    debug!("收到WebSocket文本消息: {}", text);
                }
                Message::Close(_) => {
                    info!("WebSocket客户端请求关闭连接");
                    break;
                }
                _ => {}
            }
        }
    });

    tokio::select! {
        _ = send_task => {},
        _ = recv_task => {},
    }

    info!("WebSocket客户端已断开");
    metrics::ws_client_disconnected();
}

pub trait WsSenderProvider: Send + Sync + 'static {
    fn broadcast_rx(&self) -> tokio::sync::broadcast::Receiver<WebSocketMessage>;
}

impl WsSenderProvider for std::sync::Arc<crate::alarm_ws::AlarmWsService> {
    fn broadcast_rx(&self) -> tokio::sync::broadcast::Receiver<WebSocketMessage> {
        self.sender().subscribe()
    }
}

pub fn create_ws_message(message_type: &str, data: serde_json::Value) -> WebSocketMessage {
    WebSocketMessage {
        message_type: message_type.to_string(),
        data,
        timestamp: chrono::Utc::now(),
    }
}
