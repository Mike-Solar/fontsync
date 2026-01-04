use anyhow::{Context, Result};
use futures::{SinkExt, StreamExt};
use log::{error, info, warn};
use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::broadcast;
use tokio::time::{interval, Duration};
use tokio_tungstenite::{accept_async, tungstenite::Message};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", content = "data")]
pub enum WebSocketMessage {
    FontAdded {
        filename: String,
        sha256: String,
        size: u64,
    },
    FontModified {
        filename: String,
        sha256: String,
        size: u64,
    },
    FontRemoved {
        filename: String,
    },
    FontListRequest,
    FontListResponse {
        fonts: Vec<FontInfo>,
    },
    SyncRequest {
        client_id: String,
    },
    SyncComplete {
        client_id: String,
        success: bool,
        message: String,
    },
    Heartbeat,
    Ack {
        message_id: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FontInfo {
    pub filename: String,
    pub sha256: String,
    pub size: u64,
    pub timestamp: u64,
}

#[derive(Debug)]
struct ClientInfo {
    addr: SocketAddr,
    client_id: String,
    last_heartbeat: Arc<RwLock<std::time::Instant>>,
}

pub struct WebSocketServer {
    clients: Arc<RwLock<HashMap<SocketAddr, ClientInfo>>>,
    event_sender: broadcast::Sender<WebSocketMessage>,
    server_addr: SocketAddr,
}

impl WebSocketServer {
    pub fn new(addr: SocketAddr) -> Self {
        let (event_sender, _) = broadcast::channel(1024);
        Self {
            clients: Arc::new(RwLock::new(HashMap::new())),
            event_sender,
            server_addr: addr,
        }
    }

    pub async fn start(&self) -> Result<()> {
        let listener = TcpListener::bind(self.server_addr)
            .await
            .context("Failed to bind WebSocket server")?;
        
        info!("WebSocket server listening on: {}", self.server_addr);

        // Start heartbeat checker
        let clients = Arc::clone(&self.clients);
        tokio::spawn(async move {
            Self::heartbeat_checker(clients).await;
        });

        // Accept incoming connections
        while let Ok((stream, addr)) = listener.accept().await {
            let clients = Arc::clone(&self.clients);
            let event_sender = self.event_sender.clone();
            let event_receiver = self.event_sender.subscribe();

            tokio::spawn(async move {
                if let Err(e) = Self::handle_connection(stream, addr, clients, event_sender, event_receiver).await {
                    error!("WebSocket connection error for {}: {}", addr, e);
                }
            });
        }

        Ok(())
    }

    async fn handle_connection(
        stream: TcpStream,
        addr: SocketAddr,
        clients: Arc<RwLock<HashMap<SocketAddr, ClientInfo>>>,
        event_sender: broadcast::Sender<WebSocketMessage>,
        mut event_receiver: broadcast::Receiver<WebSocketMessage>,
    ) -> Result<()> {
        let ws_stream = accept_async(stream)
            .await
            .context("Failed to accept WebSocket connection")?;

        info!("New WebSocket connection from: {}", addr);

        let (mut ws_sender, mut ws_receiver) = ws_stream.split();
        
        // Generate client ID
        let client_id = format!("client_{}", uuid::Uuid::new_v4());
        
        // Register client
        let client_info = ClientInfo {
            addr,
            client_id: client_id.clone(),
            last_heartbeat: Arc::new(RwLock::new(std::time::Instant::now())),
        };
        
        clients.write().insert(addr, client_info);
        
        info!("Registered client {} with ID: {}", addr, client_id);

        // Send welcome message
        let welcome_msg = WebSocketMessage::SyncComplete {
            client_id: client_id.clone(),
            success: true,
            message: "Connected to font sync server".to_string(),
        };
        
        let welcome_json = serde_json::to_string(&welcome_msg)
            .context("Failed to serialize welcome message")?;
        
        ws_sender.send(Message::Text(welcome_json))
            .await
            .context("Failed to send welcome message")?;

        // Handle incoming messages and outgoing events
        let clients_clone = Arc::clone(&clients);
        let addr_clone = addr;
        
        let mut heartbeat_interval = interval(Duration::from_secs(30));
        
        loop {
            tokio::select! {
                // Handle incoming WebSocket messages
                msg = ws_receiver.next() => {
                    match msg {
                        Some(Ok(Message::Text(text))) => {
                            if let Ok(ws_msg) = serde_json::from_str::<WebSocketMessage>(&text) {
                                Self::handle_client_message(ws_msg, &mut ws_sender, &event_sender, addr).await?;
                            } else {
                                warn!("Received invalid message from {}: {}", addr, text);
                            }
                        }
                        Some(Ok(Message::Binary(_))) => {
                            // Ignore binary messages
                        }
                        Some(Ok(Message::Close(_))) => {
                            info!("Client {} requested close", addr);
                            break;
                        }
                        Some(Ok(Message::Ping(_))) => {
                            // Pong is handled automatically by tokio-tungstenite
                        }
                        Some(Ok(Message::Pong(_))) => {
                            // Update heartbeat on pong
                            if let Some(client) = clients.read().get(&addr) {
                                *client.last_heartbeat.write() = std::time::Instant::now();
                            }
                        }
                        Some(Ok(Message::Frame(_))) => {
                            // Raw frame, ignore
                        }
                        Some(Err(e)) => {
                            error!("WebSocket error from {}: {}", addr, e);
                            break;
                        }
                        None => {
                            info!("WebSocket stream ended for {}", addr);
                            break;
                        }
                    }
                }
                
                // Handle broadcast events
                event = event_receiver.recv() => {
                    match event {
                        Ok(msg) => {
                            let json_msg = serde_json::to_string(&msg)
                                .context("Failed to serialize broadcast message")?;
                            
                            if let Err(e) = ws_sender.send(Message::Text(json_msg)).await {
                                error!("Failed to send message to {}: {}", addr, e);
                                break;
                            }
                        }
                        Err(broadcast::error::RecvError::Lagged(n)) => {
                            warn!("Client {} lagged by {} messages", addr, n);
                        }
                        Err(broadcast::error::RecvError::Closed) => {
                            error!("Broadcast channel closed for {}", addr);
                            break;
                        }
                    }
                }
                
                // Send heartbeat
                _ = heartbeat_interval.tick() => {
                    let heartbeat_msg = WebSocketMessage::Heartbeat;
                    let json_msg = serde_json::to_string(&heartbeat_msg)
                        .context("Failed to serialize heartbeat message")?;
                    
                    if let Err(e) = ws_sender.send(Message::Text(json_msg)).await {
                        error!("Failed to send heartbeat to {}: {}", addr, e);
                        break;
                    }
                    
                    // Check if client is still alive
                    if let Some(client) = clients.read().get(&addr) {
                        let last_heartbeat = *client.last_heartbeat.read();
                        if last_heartbeat.elapsed() > Duration::from_secs(120) {
                            warn!("Client {} heartbeat timeout", addr);
                            break;
                        }
                    }
                }
            }
        }

        // Remove client on disconnect
        clients.write().remove(&addr);
        info!("Client {} disconnected", addr);

        Ok(())
    }

    async fn handle_client_message(
        msg: WebSocketMessage,
        ws_sender: &mut futures::stream::SplitSink<tokio_tungstenite::WebSocketStream<TcpStream>, Message>,
        event_sender: &broadcast::Sender<WebSocketMessage>,
        addr: SocketAddr,
    ) -> Result<()> {
        match msg {
            WebSocketMessage::FontListRequest => {
                // Respond with current font list
                let response = WebSocketMessage::FontListResponse {
                    fonts: Vec::new(), // This would be populated with actual font data
                };
                
                let json_msg = serde_json::to_string(&response)
                    .context("Failed to serialize font list response")?;
                
                ws_sender.send(Message::Text(json_msg))
                    .await
                    .context("Failed to send font list response")?;
            }
            WebSocketMessage::Heartbeat => {
                // Update client heartbeat
                info!("Received heartbeat from {}", addr);
            }
            WebSocketMessage::SyncRequest { client_id } => {
                info!("Sync request from client: {}", client_id);
                // Handle sync request
                let response = WebSocketMessage::SyncComplete {
                    client_id: client_id.clone(),
                    success: true,
                    message: "Sync started".to_string(),
                };
                
                let json_msg = serde_json::to_string(&response)
                    .context("Failed to serialize sync response")?;
                
                ws_sender.send(Message::Text(json_msg))
                    .await
                    .context("Failed to send sync response")?;
            }
            _ => {
                // Broadcast other messages to all clients
                let _ = event_sender.send(msg);
            }
        }
        
        Ok(())
    }

    async fn heartbeat_checker(clients: Arc<RwLock<HashMap<SocketAddr, ClientInfo>>>) {
        let mut interval = interval(Duration::from_secs(60));
        
        loop {
            interval.tick().await;
            
            let now = std::time::Instant::now();
            let mut disconnected_clients = Vec::new();
            
            {
                let clients_guard = clients.read();
                for (addr, client) in clients_guard.iter() {
                    let last_heartbeat = *client.last_heartbeat.read();
                    if now.duration_since(last_heartbeat) > Duration::from_secs(180) {
                        disconnected_clients.push(*addr);
                    }
                }
            }
            
            // Remove disconnected clients
            if !disconnected_clients.is_empty() {
                let mut clients_guard = clients.write();
                for addr in disconnected_clients {
                    clients_guard.remove(&addr);
                    warn!("Removed disconnected client: {}", addr);
                }
            }
        }
    }

    pub fn broadcast_font_event(&self, event: WebSocketMessage) -> Result<()> {
        self.event_sender.send(event)
            .context("Failed to broadcast font event")?;
        Ok(())
    }

    pub fn get_connected_clients(&self) -> usize {
        self.clients.read().len()
    }
}

pub async fn start_websocket_server(addr: SocketAddr) -> Result<()> {
    let server = WebSocketServer::new(addr);
    server.start().await
}

// Helper function to create font event messages
pub fn create_font_added_event(filename: String, sha256: String, size: u64) -> WebSocketMessage {
    WebSocketMessage::FontAdded {
        filename,
        sha256,
        size,
    }
}

pub fn create_font_modified_event(filename: String, sha256: String, size: u64) -> WebSocketMessage {
    WebSocketMessage::FontModified {
        filename,
        sha256,
        size,
    }
}

pub fn create_font_removed_event(filename: String) -> WebSocketMessage {
    WebSocketMessage::FontRemoved {
        filename,
    }
}