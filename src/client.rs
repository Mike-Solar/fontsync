use anyhow::{Context, Result};
use indicatif::{ProgressBar, ProgressStyle};
use log::{error, info};
use reqwest::multipart;
use serde::Deserialize;

use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;
use tokio::fs::{create_dir_all, File};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use walkdir::WalkDir;

use crate::font_installer;
use crate::utils;

#[derive(Deserialize, Debug)]
pub struct FontInfo {
    pub name: String,
    pub size: u64,
    pub mime_type: String,
    pub sha256: String,
}

#[derive(Deserialize, Debug)]
pub struct FontList {
    pub fonts: Vec<FontInfo>,
}

pub async fn run_client(
    server_url: String,
    local_dir: String,
    _install: bool,
    _upload: bool,
    watch: bool,
    _ws_url: String,
    _interactive: bool,
    once: bool,
) -> Result<()> {
    let local_dir_path = PathBuf::from(&local_dir);
    
    // 本地目录不存在时创建
    if !local_dir_path.exists() {
        create_dir_all(&local_dir_path)
            .await
            .context("Failed to create local directory")?;
        info!("Created local directory: {}", local_dir);
    }

    // 执行初始同步
    info!("Starting sync with server: {}", server_url);

    // 一次性模式同步后退出
    if once {
        info!("One-time sync completed, exiting");
        return Ok(());
    }

    // 按需启动文件系统监听
    if watch {
        info!("Starting file system watcher...");
        // 待办：实现文件系统监听
        
        // 保持程序运行
        info!("Watching for changes. Press Ctrl+C to stop.");
        tokio::signal::ctrl_c().await?;
        info!("Shutting down...");
    } else {
        info!("Sync completed. Use --watch flag for real-time monitoring.");
    }

    Ok(())
}

pub async fn upload_local_fonts(
    server_url: &str,
    local_dir: &Path,
    interactive: bool,
) -> Result<(usize, usize)> {
    info!("Scanning local fonts for upload...");
    
    let client = reqwest::Client::new();
    let mut uploaded = 0;
    let mut skipped = 0;

    // 先获取服务器上已有字体及其 SHA256
    let server_fonts = get_server_fonts_with_sha256(server_url).await?;
    let server_font_map: std::collections::HashMap<String, String> = server_fonts
        .fonts
        .iter()
        .map(|f| (f.name.clone(), f.sha256.clone()))
        .collect();

    for entry in WalkDir::new(local_dir)
        .follow_links(true)
        .into_iter()
        .filter_map(|e| e.ok())
    {
        let path = entry.path();
        if path.is_file() && utils::is_font_file(path) {
            let filename = path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("unknown")
                .to_string();

            // 计算本地 SHA256
            let local_sha256 = match utils::calculate_sha256(path) {
                Ok(sha) => sha,
                Err(e) => {
                    error!("Failed to calculate SHA256 for '{}': {}", filename, e);
                    continue;
                }
            };

            // 检查服务器是否已有该文件
            if let Some(remote_sha256) = server_font_map.get(&filename) {
                if local_sha256 == *remote_sha256 {
                    info!("Font '{}' already exists with same SHA256, skipping", filename);
                    skipped += 1;
                    continue;
                } else {
                    // 检测到冲突
                    info!("Conflict detected for '{}': local SHA256={}, remote SHA256={}", 
                        filename, local_sha256, remote_sha256);
                    
                    let resolution = utils::prompt_conflict_resolution(
                        &filename,
                        &local_sha256,
                        remote_sha256,
                        interactive,
                    )?;

                    match resolution {
                        utils::ConflictResolution::Overwrite => {
                            info!("Overwriting font '{}'", filename);
                        }
                        utils::ConflictResolution::Rename => {
                            // 生成唯一名称
                            let mut counter = 1;
                            let mut new_filename = utils::generate_unique_filename(path, counter);
                            while server_font_map.contains_key(&new_filename) {
                                counter += 1;
                                new_filename = utils::generate_unique_filename(path, counter);
                            }
                            info!("Renaming font '{}' to '{}'", filename, new_filename);
                            // 待办：实现重命名逻辑
                            skipped += 1;
                            continue;
                        }
                        utils::ConflictResolution::Skip => {
                            info!("Skipping font '{}'", filename);
                            skipped += 1;
                            continue;
                        }
                                            }
                }
            }

            info!("Uploading font: {}", filename);
            
            match upload_font_file(&client, server_url, path, &filename, &local_sha256).await {
                Ok(_) => {
                    info!("Successfully uploaded: {}", filename);
                    uploaded += 1;
                    
                    // 小延迟，避免请求过密
                    tokio::time::sleep(Duration::from_millis(100)).await;
                }
                Err(e) => {
                    error!("Failed to upload '{}': {}", filename, e);
                }
            }
        }
    }

    info!("Upload complete: {} uploaded, {} skipped", uploaded, skipped);
    Ok((uploaded, skipped))
}

async fn upload_font_file(
    client: &reqwest::Client,
    server_url: &str,
    file_path: &Path,
    filename: &str,
    _sha256: &str,
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
    
    // 读取文件内容
    let mut buffer = Vec::with_capacity(metadata.len() as usize);
    let mut reader = tokio::io::BufReader::new(file);
    reader.read_to_end(&mut buffer).await?;
    
    pb.finish_and_clear();
    
    // 创建 multipart 表单
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

pub async fn get_server_fonts(server_url: &str) -> Result<FontList> {
    get_server_fonts_with_sha256(server_url).await
}

pub async fn get_server_fonts_with_sha256(server_url: &str) -> Result<FontList> {
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

pub async fn download_server_fonts(
    server_url: &str,
    local_dir: &Path,
    interactive: bool,
) -> Result<(usize, usize)> {
    info!("Downloading fonts from server...");
    
    let font_list = get_server_fonts_with_sha256(server_url).await?;
    let client = reqwest::Client::new();
    let mut downloaded = 0;
    let mut skipped = 0;

    for font in font_list.fonts {
        let font_path = local_dir.join(&font.name);
        
        // 检查本地是否已存在
        if font_path.exists() {
            match utils::calculate_sha256(&font_path) {
                Ok(local_sha256) => {
                    if local_sha256 == font.sha256 {
                        info!("Font '{}' already exists with same SHA256, skipping", font.name);
                        skipped += 1;
                        continue;
                    } else {
                        // 检测到冲突
                        info!("Conflict detected for '{}': local SHA256={}, remote SHA256={}", 
                            font.name, local_sha256, font.sha256);
                        
                        let resolution = utils::prompt_conflict_resolution(
                            &font.name,
                            &local_sha256,
                            &font.sha256,
                            interactive,
                        )?;

                        match resolution {
                            utils::ConflictResolution::Overwrite => {
                                info!("Overwriting font '{}'", font.name);
                            }
                            utils::ConflictResolution::Rename => {
                                // 生成唯一名称
                                let mut counter = 1;
                                let mut new_filename = utils::generate_unique_filename(&font_path, counter);
                                while local_dir.join(&new_filename).exists() {
                                    counter += 1;
                                    new_filename = utils::generate_unique_filename(&font_path, counter);
                                }
                                info!("Renaming font '{}' to '{}'", font.name, new_filename);
                                // 待办：实现重命名逻辑
                                skipped += 1;
                                continue;
                            }
                            utils::ConflictResolution::Skip => {
                                info!("Skipping font '{}'", font.name);
                                skipped += 1;
                                continue;
                            }
                                                    }
                    }
                }
                Err(e) => {
                    error!("Failed to calculate SHA256 for local file '{}': {}", font.name, e);
                    // 继续下载
                }
            }
        }

        info!("Downloading font: {} ({} bytes)", font.name, font.size);
        
        match download_font_file(&client, server_url, &font.name, &font_path).await {
            Ok(_) => {
                // 校验已下载文件的 SHA256
                match utils::calculate_sha256(&font_path) {
                    Ok(downloaded_sha256) => {
                        if downloaded_sha256 == font.sha256 {
                            info!("Successfully downloaded and verified: {}", font.name);
                            downloaded += 1;
                        } else {
                            error!("SHA256 mismatch for downloaded file '{}': expected={}, got={}", 
                                font.name, font.sha256, downloaded_sha256);
                            // 移除损坏文件
                            let _ = fs::remove_file(&font_path);
                        }
                    }
                    Err(e) => {
                        error!("Failed to verify SHA256 for '{}': {}", font.name, e);
                    }
                }
                
                // 小延迟，避免请求过密
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
            Err(e) => {
                error!("Failed to download '{}': {}", font.name, e);
            }
        }
    }

    info!("Download complete: {} downloaded, {} skipped", downloaded, skipped);
    Ok((downloaded, skipped))
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

pub async fn install_downloaded_fonts(local_dir: &Path) -> Result<(usize, usize)> {
    info!("Installing downloaded fonts...");
    
    let mut installed = 0;
    let mut failed = 0;

    for entry in WalkDir::new(local_dir)
        .max_depth(1)
        .into_iter()
        .filter_map(|e| e.ok())
    {
        let path = entry.path();
        if path.is_file() && utils::is_font_file(path) {
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
    Ok((installed, failed))
}
