use anyhow::{Context, Result};
use futures::{SinkExt, StreamExt};
use log::{error, info};
use std::path::{Path, PathBuf};
use tokio_tungstenite::{connect_async, tungstenite::Message, MaybeTlsStream, WebSocketStream};
use tokio::net::TcpStream;

use crate::client::{download_server_fonts, upload_local_fonts};
use crate::font_installer;
use crate::utils::{calculate_sha256, get_system_font_directories};
use crate::websocket_server::WebSocketMessage;

#[derive(Clone)]
pub struct WebSocketClient {
    server_url: String,
    client_id: String,
    local_font_dirs: Vec<PathBuf>,
    download_dir: PathBuf,
}

impl WebSocketClient {
    pub fn new(server_url: String, client_id: String) -> Self {
        Self {
            server_url,
            client_id,
            local_font_dirs: get_system_font_directories(),
            download_dir: dirs::cache_dir()
                .unwrap_or_else(|| PathBuf::from("."))
                .join("fontsync/downloads"),
        }
    }

    pub async fn connect(&mut self) -> Result<()> {
        let (ws_stream, ws_url) = self.connect_ws().await?;
        info!("Connected to WebSocket server: {}", ws_url);
        
        let (mut ws_sender, _ws_receiver) = ws_stream.split();
        
        // Send sync request
        let sync_request = WebSocketMessage::SyncRequest {
            client_id: self.client_id.clone(),
        };
        
        let json_msg = serde_json::to_string(&sync_request)
            .context("Failed to serialize sync request")?;
        
        ws_sender.send(Message::Text(json_msg))
            .await
            .context("Failed to send sync request")?;
        
        Ok(())
    }

    pub async fn connect_and_run(&mut self) -> Result<()> {
        let (ws_stream, ws_url) = self.connect_ws().await?;
        self.run_with_stream(ws_stream, ws_url).await
    }

    async fn run_with_stream(
        &mut self,
        ws_stream: WebSocketStream<MaybeTlsStream<TcpStream>>,
        ws_url: String,
    ) -> Result<()> {
        // Create download directory
        tokio::fs::create_dir_all(&self.download_dir)
            .await
            .context("Failed to create download directory")?;

        info!("Connected to WebSocket server: {}", ws_url);

        let (mut ws_sender, _ws_receiver) = ws_stream.split();
        
        // Send initial sync request
        let sync_request = WebSocketMessage::SyncRequest {
            client_id: self.client_id.clone(),
        };
        
        let json_msg = serde_json::to_string(&sync_request)
            .context("Failed to serialize sync request")?;
        
        ws_sender.send(Message::Text(json_msg))
            .await
            .context("Failed to send sync request")?;

        // Perform initial sync
        self.perform_initial_sync().await?;

        info!("WebSocket client operations completed");
        Ok(())
    }

    async fn handle_server_message(
        &self,
        msg: WebSocketMessage,
        ws_sender: &mut futures::stream::SplitSink<WebSocketStream<MaybeTlsStream<TcpStream>>, Message>,
    ) -> Result<()> {
        match msg {
            WebSocketMessage::FontAdded { filename, sha256, size } => {
                info!("Server notified font added: {} ({} bytes, SHA256: {}...)", 
                    filename, size, &sha256[..16]);
                
                // Auto-download the new font
                self.download_font(&filename, &sha256).await?;
            }
            WebSocketMessage::FontModified { filename, sha256, size } => {
                info!("Server notified font modified: {} ({} bytes, SHA256: {}...)", 
                    filename, size, &sha256[..16]);
                
                // Download the updated font
                self.download_font(&filename, &sha256).await?;
            }
            WebSocketMessage::FontRemoved { filename } => {
                info!("Server notified font removed: {}", filename);
                
                // Remove local copy if it exists and has the same SHA256
                self.handle_font_removal(&filename).await?;
            }
            WebSocketMessage::SyncComplete { client_id, success, message } => {
                if client_id == self.client_id {
                    info!("Sync completed: {} - {}", success, message);
                    if success {
                        // Perform initial sync
                        self.perform_initial_sync().await?;
                    }
                }
            }
            WebSocketMessage::Heartbeat => {
                // Respond with heartbeat
                let heartbeat_msg = WebSocketMessage::Heartbeat;
                let json_msg = serde_json::to_string(&heartbeat_msg)
                    .context("Failed to serialize heartbeat response")?;
                
                let _ = ws_sender.send(Message::Text(json_msg))
                    .await
                    .context("Failed to send heartbeat response")?;
            }
            _ => {
                // Handle other message types
                info!("Received message from server: {:?}", msg);
            }
        }
        
        Ok(())
    }

    async fn download_font(&self, filename: &str, expected_sha256: &str) -> Result<()> {
        let font_path = self.download_dir.join(filename);
        
        // Check if font already exists with correct SHA256
        if font_path.exists() {
            if let Ok(local_sha256) = calculate_sha256(&font_path) {
                if local_sha256 == expected_sha256 {
                    info!("Font {} already exists with correct SHA256, skipping download", filename);
                    return Ok(());
                }
            }
        }

        info!("Downloading font: {}", filename);
        
        // Download from server
        let server_url = self.server_url.clone();
        let client = reqwest::Client::new();
        let url = format!("{}/fonts/{}", server_url, filename);
        
        let response = client.get(&url).send().await
            .context("Failed to download font")?;
        
        if !response.status().is_success() {
            return Err(anyhow::anyhow!("Failed to download font: HTTP {}", response.status()));
        }
        
        let bytes = response.bytes().await
            .context("Failed to read font data")?;
        
        // Verify SHA256
        let downloaded_sha256 = calculate_sha256_from_bytes(&bytes)?;
        if downloaded_sha256 != expected_sha256 {
            return Err(anyhow::anyhow!(
                "SHA256 mismatch for downloaded font {}: expected={}, got={}",
                filename, expected_sha256, downloaded_sha256
            ));
        }
        
        // Save font file
        tokio::fs::write(&font_path, bytes)
            .await
            .context("Failed to save font file")?;
        
        info!("Successfully downloaded and verified font: {}", filename);
        
        // Install the font
        self.install_downloaded_font(&font_path).await?;
        
        Ok(())
    }

    async fn handle_font_removal(&self, filename: &str) -> Result<()> {
        // Find and remove the font from system font directories
        for font_dir in &self.local_font_dirs {
            let font_path = font_dir.join(filename);
            if font_path.exists() {
                // Verify it's the same file by checking if it's also in our download directory
                let download_path = self.download_dir.join(filename);
                if download_path.exists() {
                    let system_sha256 = calculate_sha256(&font_path)?;
                    let download_sha256 = calculate_sha256(&download_path)?;
                    
                    if system_sha256 == download_sha256 {
                        // Remove from system fonts
                        tokio::fs::remove_file(&font_path)
                            .await
                            .context("Failed to remove font from system")?;
                        
                        info!("Removed font from system: {}", filename);
                        
                        // Also remove from download directory
                        tokio::fs::remove_file(&download_path)
                            .await
                            .context("Failed to remove font from download directory")?;
                    }
                }
            }
        }
        
        Ok(())
    }

    async fn install_downloaded_font(&self, font_path: &Path) -> Result<()> {
        info!("Installing downloaded font: {:?}", font_path.file_name().unwrap_or_default());
        
        match font_installer::install_font(font_path).await {
            Ok(_) => {
                info!("Successfully installed font");
                Ok(())
            }
            Err(e) => {
                error!("Failed to install font: {}", e);
                Err(e)
            }
        }
    }

    async fn perform_initial_sync(&self) -> Result<()> {
        info!("Performing initial font sync...");
        
        // Upload local fonts to server
        let mut total_uploaded = 0;
        
        for font_dir in &self.local_font_dirs {
            if font_dir.exists() {
                let (uploaded, _) = upload_local_fonts(
                    &self.server_url,
                    font_dir,
                    false, // Non-interactive mode for auto-sync
                ).await?;
                
                total_uploaded += uploaded;
            }
        }
        
        info!("Upload sync complete: {} uploaded, {} skipped", total_uploaded, 0);
        
        // Download fonts from server
        let (downloaded, skipped) = download_server_fonts(
            &self.server_url,
            &self.download_dir,
            false, // Non-interactive mode for auto-sync
        ).await?;
        
        info!("Download sync complete: {} downloaded, {} skipped", downloaded, skipped);
        
        // Install downloaded fonts
        if downloaded > 0 {
            let (installed, failed) = font_installer::install_fonts_from_directory(&self.download_dir).await?;
            info!("Installation complete: {} installed, {} failed", installed, failed);
        }
        
        Ok(())
    }

    async fn connect_ws(&self) -> Result<(WebSocketStream<MaybeTlsStream<TcpStream>>, String)> {
        let ws_urls = build_ws_urls(&self.server_url)?;
        let mut last_err = None;

        for ws_url in ws_urls {
            info!("Connecting to WebSocket server: {}", ws_url);
            match connect_async(&ws_url).await {
                Ok((ws_stream, _)) => return Ok((ws_stream, ws_url)),
                Err(e) => last_err = Some(e),
            }
        }

        Err(anyhow::anyhow!(
            "Failed to connect to WebSocket server: {}",
            last_err
                .map(|e| e.to_string())
                .unwrap_or_else(|| "unknown error".to_string())
        ))
    }
}

fn calculate_sha256_from_bytes(data: &[u8]) -> Result<String> {
    use sha2::{Digest, Sha256};
    
    let mut hasher = Sha256::new();
    hasher.update(data);
    let result = hasher.finalize();
    
    Ok(hex::encode(result))
}

pub async fn start_websocket_client(
    server_url: String,
    client_id: String,
) -> Result<WebSocketClient> {
    let client = WebSocketClient::new(server_url, client_id);

    let (ws_stream, ws_url) = match client.connect_ws().await {
        Ok(result) => result,
        Err(e) => {
            error!("WebSocket client error: {}", e);
            return Err(e);
        }
    };

    // Connect and run in background
    let mut client_clone = client.clone();
    tokio::spawn(async move {
        if let Err(e) = client_clone.run_with_stream(ws_stream, ws_url).await {
            error!("WebSocket client error: {}", e);
        }
    });
    
    Ok(client)
}

fn build_ws_urls(server_url: &str) -> Result<Vec<String>> {
    let mut url = reqwest::Url::parse(server_url)
        .or_else(|_| reqwest::Url::parse(&format!("ws://{}", server_url)))
        .context("Invalid server URL")?;

    match url.scheme() {
        "http" => {
            url.set_scheme("ws").map_err(|_| anyhow::anyhow!("Invalid URL scheme"))?;
        }
        "https" => {
            url.set_scheme("wss").map_err(|_| anyhow::anyhow!("Invalid URL scheme"))?;
        }
        "ws" | "wss" => {}
        _ => return Err(anyhow::anyhow!("Unsupported URL scheme")),
    }

    let mut urls = vec![url.to_string()];
    if let Some(port) = url.port() {
        if let Some(next_port) = port.checked_add(1) {
            let mut alt = url.clone();
            if alt.set_port(Some(next_port)).is_ok() {
                let alt_str = alt.to_string();
                if alt_str != urls[0] {
                    urls.push(alt_str);
                }
            }
        }
    }

    Ok(urls)
}
