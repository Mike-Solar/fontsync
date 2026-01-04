use anyhow::{Context, Result};
use bytes::Buf;
use futures::StreamExt;
use log::{error, info};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::fs::{create_dir_all, File};
use tokio::io::{AsyncWriteExt, BufWriter};
use warp::{
    hyper::StatusCode,
    multipart::{FormData, Part},
    Filter, Rejection, Reply,
};

mod utils {
    pub fn calculate_sha256(_path: &std::path::Path) -> anyhow::Result<String> {
        Ok("placeholder_sha256".to_string())
    }
    
    pub fn is_font_file(path: &std::path::Path) -> bool {
        if let Some(ext) = path.extension() {
            let ext_str = ext.to_string_lossy().to_lowercase();
            matches!(
                ext_str.as_str(),
                "ttf" | "otf" | "woff" | "woff2" | "eot" | "ttc"
            )
        } else {
            false
        }
    }
}

#[derive(Serialize, Deserialize, Debug)]
struct FontInfo {
    name: String,
    size: u64,
    mime_type: String,
    sha256: String,
}

#[derive(Serialize, Deserialize, Debug)]
struct FontList {
    fonts: Vec<FontInfo>,
}

pub async fn start_server(host: String, port: u16, font_dir: String, ws_enabled: bool) -> Result<()> {
    let font_dir_path = PathBuf::from(&font_dir);
    
    // Create font directory if it doesn't exist
    if !font_dir_path.exists() {
        create_dir_all(&font_dir_path)
            .await
            .context("Failed to create font directory")?;
        info!("Created font directory: {}", font_dir);
    }

    let font_dir_arc = Arc::new(font_dir_path);
    let ws_server: Option<()> = if ws_enabled { Some(()) } else { None };

    // Routes
    let font_dir_filter = warp::any().map(move || Arc::clone(&font_dir_arc));
    let ws_server_filter = warp::any().map(move || ws_server.clone());

    let list_fonts = warp::path!("fonts")
        .and(warp::get())
        .and(font_dir_filter.clone())
        .and_then(list_fonts_handler);

    let download_font = warp::path!("fonts" / String)
        .and(warp::get())
        .and(font_dir_filter.clone())
        .and_then(download_font_handler);

    let upload_font = warp::path!("fonts")
        .and(warp::post())
        .and(warp::multipart::form().max_length(100 * 1024 * 1024)) // 100MB limit
        .and(font_dir_filter.clone())
        .and(ws_server_filter.clone())
        .and_then(upload_font_handler);

    let get_sha256 = warp::path!("fonts" / String / "sha256")
        .and(warp::get())
        .and(font_dir_filter.clone())
        .and_then(get_sha256_handler);
    
    // Add WebSocket route if enabled
    // WebSocket route would be configured here

    let routes = list_fonts
        .or(download_font)
        .or(upload_font)
        .or(get_sha256);
    
    // WebSocket route would be added here if enabled
    
    let routes = routes
        .with(warp::cors().allow_any_origin())
        .with(warp::log("fontsync::server"));

    let addr: std::net::SocketAddr = format!("{}:{}", host, port)
        .parse()
        .context("Failed to parse socket address")?;

    info!("Server listening on http://{}", addr);
    if ws_enabled {
        info!("WebSocket server would be enabled on ws://{}", addr);
    }
    
    warp::serve(routes).run(addr).await;

    Ok(())
}

async fn list_fonts_handler(
    font_dir: Arc<PathBuf>,
) -> Result<Box<dyn Reply>, Rejection> {
    match list_fonts_impl(&font_dir).await {
        Ok(font_list) => Ok(Box::new(warp::reply::json(&font_list))),
        Err(e) => {
            error!("Failed to list fonts: {}", e);
            Ok(Box::new(warp::reply::with_status(
                warp::reply::json(&serde_json::json!({"error": e.to_string()})),
                StatusCode::INTERNAL_SERVER_ERROR,
            )))
        }
    }
}

async fn list_fonts_impl(font_dir: &Path) -> Result<FontList> {
    let mut fonts = Vec::new();

    if !font_dir.exists() {
        return Ok(FontList { fonts });
    }

    let entries = fs::read_dir(font_dir).context("Failed to read font directory")?;

    for entry in entries {
        let entry = entry.context("Failed to read directory entry")?;
        let path = entry.path();

        if path.is_file() && utils::is_font_file(&path) {
            let metadata = fs::metadata(&path).context("Failed to get file metadata")?;
            let name = path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("unknown")
                .to_string();

            let mime_type = mime_guess::from_path(&path)
                .first_or_octet_stream()
                .to_string();

            let sha256 = utils::calculate_sha256(&path)
                .unwrap_or_else(|_| String::new());

            fonts.push(FontInfo {
                name,
                size: metadata.len(),
                mime_type,
                sha256,
            });
        }
    }

    Ok(FontList { fonts })
}

async fn download_font_handler(
    filename: String,
    font_dir: Arc<PathBuf>,
) -> Result<Box<dyn Reply>, Rejection> {
    let font_path = font_dir.join(&filename);

    if !font_path.exists() {
        return Ok(Box::new(warp::reply::with_status(
            format!("Font '{}' not found", filename),
            StatusCode::NOT_FOUND,
        )));
    }

    match File::open(&font_path).await {
        Ok(file) => {
            // Get file size for Content-Length header
            let metadata = match tokio::fs::metadata(&font_path).await {
                Ok(m) => m,
                Err(_) => return Ok(Box::new(warp::reply::with_status(
                    format!("Failed to get metadata for font '{}'", filename),
                    StatusCode::INTERNAL_SERVER_ERROR,
                ))),
            };
            
            // Determine content type
            let content_type = mime_guess::from_path(&font_path)
                .first_or_octet_stream()
                .to_string();

            let stream = tokio_util::io::ReaderStream::new(file);
            let body = warp::hyper::Body::wrap_stream(stream);
            
            let mut response = warp::reply::Response::new(body);
            response.headers_mut().insert(
                "Content-Type",
                content_type.parse().unwrap_or_else(|_| "application/octet-stream".parse().unwrap()),
            );
            response.headers_mut().insert(
                "Content-Disposition",
                format!("attachment; filename=\"{}\"", filename)
                    .parse()
                    .unwrap(),
            );
            response.headers_mut().insert(
                "Content-Length",
                metadata.len().to_string().parse().unwrap(),
            );

            Ok(Box::new(response))
        }
        Err(e) => {
            error!("Failed to open font file '{}': {}", filename, e);
            Ok(Box::new(warp::reply::with_status(
                format!("Failed to open font file: {}", e),
                StatusCode::INTERNAL_SERVER_ERROR,
            )))
        }
    }
}

async fn upload_font_handler(
    mut form: FormData,
    font_dir: Arc<PathBuf>,
    ws_server: Option<()>,
) -> Result<Box<dyn Reply>, Rejection> {
    while let Some(part) = form.next().await {
        match part {
            Ok(p) => {
                if p.name() == "font" {
                    let filename = p.filename().unwrap_or("unknown_font").to_string();
                    let font_path = font_dir.join(&filename);

                    match save_part_to_file(p, &font_path).await {
                        Ok(sha256) => {
                            info!("Uploaded font: {} (SHA256: {})", filename, sha256);
                            
                            // Broadcast WebSocket notification
                            if let Some(_ws_server) = ws_server {
                                // WebSocket notification would be sent here
                            }
                            
                            return Ok(Box::new(warp::reply::with_status(
                                warp::reply::json(&serde_json::json!({
                                    "success": true,
                                    "filename": filename,
                                    "sha256": sha256,
                                    "message": "Successfully uploaded"
                                })),
                                StatusCode::OK,
                            )));
                        }
                        Err(e) => {
                            error!("Failed to save font '{}': {}", filename, e);
                            return Ok(Box::new(warp::reply::with_status(
                                warp::reply::json(&serde_json::json!({
                                    "error": e.to_string(),
                                    "message": "Failed to save font"
                                })),
                                StatusCode::INTERNAL_SERVER_ERROR,
                            )));
                        }
                    }
                }
            }
            Err(e) => {
                error!("Error processing multipart form: {}", e);
                return Ok(Box::new(warp::reply::with_status(
                    warp::reply::json(&serde_json::json!({
                        "error": e.to_string(),
                        "message": "Error processing form"
                    })),
                    StatusCode::BAD_REQUEST,
                )));
            }
        }
    }

    Ok(Box::new(warp::reply::with_status(
        warp::reply::json(&serde_json::json!({
            "error": "No font file found in upload",
            "message": "No font file provided"
        })),
        StatusCode::BAD_REQUEST,
    )))
}

async fn save_part_to_file(part: Part, path: &Path) -> Result<String> {
    let mut file = BufWriter::new(File::create(path).await?);
    
    let mut stream = part.stream();
    while let Some(item) = stream.next().await {
        let data = item?;
        let bytes = data.chunk();
        file.write_all(bytes).await?;
    }
    
    file.flush().await?;
    
    // Calculate SHA256 after saving
    let sha256 = utils::calculate_sha256(path)?;
    Ok(sha256)
}

async fn get_sha256_handler(
    filename: String,
    font_dir: Arc<PathBuf>,
) -> Result<Box<dyn Reply>, Rejection> {
    let font_path = font_dir.join(&filename);

    if !font_path.exists() {
        return Ok(Box::new(warp::reply::with_status(
            warp::reply::json(&serde_json::json!({
                "error": "Font not found",
                "message": format!("Font '{}' not found", filename)
            })),
            StatusCode::NOT_FOUND,
        )));
    }

    match utils::calculate_sha256(&font_path) {
        Ok(sha256) => Ok(Box::new(warp::reply::json(&serde_json::json!({
            "filename": filename,
            "sha256": sha256,
        })))),
        Err(e) => {
            error!("Failed to calculate SHA256 for '{}': {}", filename, e);
            Ok(Box::new(warp::reply::with_status(
                warp::reply::json(&serde_json::json!({
                    "error": e.to_string(),
                    "message": "Failed to calculate SHA256"
                })),
                StatusCode::INTERNAL_SERVER_ERROR,
            )))
        }
    }
}