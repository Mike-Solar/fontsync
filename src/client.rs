use anyhow::{Context, Result};
use indicatif::{ProgressBar, ProgressStyle};
use log::{error, info};
use reqwest::multipart;
use serde::Deserialize;
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;
use tokio::fs::{create_dir_all, File};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use walkdir::WalkDir;

use crate::font_installer;

#[derive(Deserialize, Debug)]
struct FontInfo {
    name: String,
    size: u64,
    mime_type: String,
}

#[derive(Deserialize, Debug)]
struct FontList {
    fonts: Vec<FontInfo>,
}

pub async fn run_client(
    server_url: String,
    local_dir: String,
    install: bool,
    upload: bool,
) -> Result<()> {
    let local_dir_path = PathBuf::from(&local_dir);
    
    // Create local directory if it doesn't exist
    if !local_dir_path.exists() {
        create_dir_all(&local_dir_path)
            .await
            .context("Failed to create local directory")?;
        info!("Created local directory: {}", local_dir);
    }

    if upload {
        upload_local_fonts(&server_url, &local_dir_path).await?;
    }

    download_server_fonts(&server_url, &local_dir_path).await?;

    if install {
        install_downloaded_fonts(&local_dir_path).await?;
    }

    Ok(())
}

async fn upload_local_fonts(server_url: &str, local_dir: &Path) -> Result<()> {
    info!("Scanning local fonts for upload...");
    
    let client = reqwest::Client::new();
    let mut uploaded = 0;
    let mut skipped = 0;

    // First, get list of fonts already on server
    let server_fonts = get_server_fonts(server_url).await?;
    let server_font_set: HashSet<String> = server_fonts
        .fonts
        .iter()
        .map(|f| f.name.clone())
        .collect();

    for entry in WalkDir::new(local_dir)
        .follow_links(true)
        .into_iter()
        .filter_map(|e| e.ok())
    {
        let path = entry.path();
        if path.is_file() && is_font_file(path) {
            let filename = path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("unknown")
                .to_string();

            if server_font_set.contains(&filename) {
                info!("Font '{}' already exists on server, skipping", filename);
                skipped += 1;
                continue;
            }

            info!("Uploading font: {}", filename);
            
            match upload_font_file(&client, server_url, path, &filename).await {
                Ok(_) => {
                    info!("Successfully uploaded: {}", filename);
                    uploaded += 1;
                    
                    // Small delay to avoid overwhelming server
                    tokio::time::sleep(Duration::from_millis(100)).await;
                }
                Err(e) => {
                    error!("Failed to upload '{}': {}", filename, e);
                }
            }
        }
    }

    info!("Upload complete: {} uploaded, {} skipped", uploaded, skipped);
    Ok(())
}

async fn upload_font_file(
    client: &reqwest::Client,
    server_url: &str,
    file_path: &Path,
    filename: &str,
) -> Result<()> {
    let file = File::open(file_path).await?;
    let metadata = file.metadata().await?;
    
    let pb = ProgressBar::new(metadata.len());
    pb.set_style(
        ProgressStyle::default_bar()
            .template("{spinner:.green} [{elapsed_precise}] [{bar:40.cyan/blue}] {bytes}/{total_bytes} ({eta})")
            .unwrap()
            .progress_chars("#>-"),
    );
    
    // Read file content
    let mut buffer = Vec::with_capacity(metadata.len() as usize);
    let mut reader = tokio::io::BufReader::new(file);
    reader.read_to_end(&mut buffer).await?;
    
    pb.finish_and_clear();
    
    // Create multipart form
    let part = multipart::Part::bytes(buffer)
        .file_name(filename.to_string())
        .mime_str("application/octet-stream")?;
    
    let form = multipart::Form::new().part("font", part);
    
    let url = format!("{}/fonts", server_url);
    let response = client.post(&url).multipart(form).send().await?;
    
    if !response.status().is_success() {
        let error_text = response.text().await?;
        return Err(anyhow::anyhow!("Server error: {}", error_text));
    }
    
    Ok(())
}

async fn get_server_fonts(server_url: &str) -> Result<FontList> {
    let client = reqwest::Client::new();
    let url = format!("{}/fonts", server_url);
    
    let response = client.get(&url).send().await?;
    
    if !response.status().is_success() {
        let error_text = response.text().await?;
        return Err(anyhow::anyhow!("Failed to get font list: {}", error_text));
    }
    
    let font_list: FontList = response.json().await?;
    Ok(font_list)
}

async fn download_server_fonts(server_url: &str, local_dir: &Path) -> Result<()> {
    info!("Downloading fonts from server...");
    
    let font_list = get_server_fonts(server_url).await?;
    let client = reqwest::Client::new();
    let mut downloaded = 0;
    let mut skipped = 0;

    for font in font_list.fonts {
        let font_path = local_dir.join(&font.name);
        
        // Skip if file already exists and size matches
        if font_path.exists() {
            if let Ok(metadata) = fs::metadata(&font_path) {
                if metadata.len() == font.size {
                    info!("Font '{}' already downloaded, skipping", font.name);
                    skipped += 1;
                    continue;
                }
            }
        }

        info!("Downloading font: {} ({} bytes)", font.name, font.size);
        
        match download_font_file(&client, server_url, &font.name, &font_path).await {
            Ok(_) => {
                info!("Successfully downloaded: {}", font.name);
                downloaded += 1;
                
                // Small delay to avoid overwhelming server
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
            Err(e) => {
                error!("Failed to download '{}': {}", font.name, e);
            }
        }
    }

    info!("Download complete: {} downloaded, {} skipped", downloaded, skipped);
    Ok(())
}

async fn download_font_file(
    client: &reqwest::Client,
    server_url: &str,
    filename: &str,
    output_path: &Path,
) -> Result<()> {
    let url = format!("{}/fonts/{}", server_url, filename);
    
    let response = client.get(&url).send().await?;
    
    if !response.status().is_success() {
        let error_text = response.text().await?;
        return Err(anyhow::anyhow!("Failed to download font: {}", error_text));
    }
    
    let total_size = response
        .content_length()
        .unwrap_or(0);
    
    let pb = ProgressBar::new(total_size);
    pb.set_style(
        ProgressStyle::default_bar()
            .template("{spinner:.green} [{elapsed_precise}] [{bar:40.cyan/blue}] {bytes}/{total_bytes} ({eta})")
            .unwrap()
            .progress_chars("#>-"),
    );
    
    let mut file = File::create(output_path).await?;
    let bytes = response.bytes().await?;
    
    file.write_all(&bytes).await?;
    pb.inc(bytes.len() as u64);
    
    pb.finish_and_clear();
    file.flush().await?;
    
    Ok(())
}

async fn install_downloaded_fonts(local_dir: &Path) -> Result<()> {
    info!("Installing downloaded fonts...");
    
    let mut installed = 0;
    let mut failed = 0;

    for entry in WalkDir::new(local_dir)
        .max_depth(1)
        .into_iter()
        .filter_map(|e| e.ok())
    {
        let path = entry.path();
        if path.is_file() && is_font_file(path) {
            info!("Installing font: {:?}", path.file_name().unwrap_or_default());
            
            match font_installer::install_font(path).await {
                Ok(_) => {
                    info!("Successfully installed font");
                    installed += 1;
                }
                Err(e) => {
                    error!("Failed to install font: {}", e);
                    failed += 1;
                }
            }
        }
    }

    info!("Installation complete: {} installed, {} failed", installed, failed);
    Ok(())
}

fn is_font_file(path: &Path) -> bool {
    if let Some(ext) = path.extension() {
        let ext_str = ext.to_string_lossy().to_lowercase();
        matches!(
            ext_str.as_str(),
            "ttf" | "otf" | "woff" | "woff2" | "eot" | "ttc" | "pfb" | "pfm" | "afm" | "pfa" | "dfont" | "fon" | "fnt"
        )
    } else {
        false
    }
}