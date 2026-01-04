use anyhow::{Context, Result};
use chrono::Local;
use log::{error, info, warn};
use notify::{Config, Event, RecommendedWatcher, RecursiveMode, Watcher};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::mpsc;
use tokio::time::{interval, timeout};
use walkdir::WalkDir;

use crate::utils::{calculate_sha256, is_font_file};

#[derive(Debug, Clone)]
pub enum FontEvent {
    Added(PathBuf, String), // path, sha256
    Modified(PathBuf, String),
    Removed(PathBuf),
}

#[derive(Debug, Clone)]
pub struct FontInfo {
    pub path: PathBuf,
    pub sha256: String,
    pub size: u64,
    pub modified: std::time::SystemTime,
}

pub struct FontMonitor {
    watch_paths: Vec<PathBuf>,
    font_cache: Arc<parking_lot::RwLock<HashMap<PathBuf, FontInfo>>>,
    event_sender: mpsc::UnboundedSender<FontEvent>,
    event_receiver: Option<mpsc::UnboundedReceiver<FontEvent>>,
}

impl FontMonitor {
    pub fn new() -> Self {
        let (sender, receiver) = mpsc::unbounded_channel();
        Self {
            watch_paths: Vec::new(),
            font_cache: Arc::new(parking_lot::RwLock::new(HashMap::new())),
            event_sender: sender,
            event_receiver: Some(receiver),
        }
    }

    pub fn add_watch_path(&mut self, path: PathBuf) {
        self.watch_paths.push(path);
    }

    // 获取当前系统的默认字体目录列表
    pub fn get_system_font_paths() -> Vec<PathBuf> {
        let mut paths = Vec::new();

        #[cfg(target_os = "windows")]
        {
            if let Some(font_dir) = dirs::font_dir() {
                paths.push(font_dir);
            }
            if let Some(local_app_data) = dirs::data_local_dir() {
                paths.push(local_app_data.join("Microsoft\\Windows\\Fonts"));
            }
        }

        #[cfg(target_os = "linux")]
        {
            if let Some(home_dir) = dirs::home_dir() {
                paths.push(home_dir.join(".fonts"));
                paths.push(home_dir.join(".local/share/fonts"));
            }
            paths.push(PathBuf::from("/usr/share/fonts"));
            paths.push(PathBuf::from("/usr/local/share/fonts"));
        }

        #[cfg(target_os = "macos")]
        {
            if let Some(home_dir) = dirs::home_dir() {
                paths.push(home_dir.join("Library/Fonts"));
            }
            paths.push(PathBuf::from("/System/Library/Fonts"));
            paths.push(PathBuf::from("/Library/Fonts"));
        }

        // 过滤不存在的路径
        paths.into_iter().filter(|p| p.exists()).collect()
    }

    // 扫描所有监控路径，初始化缓存并返回字体列表
    pub async fn scan_fonts(&self) -> Result<Vec<FontInfo>> {
        let mut fonts = Vec::new();
        let mut cache = self.font_cache.write();
        cache.clear();

        for watch_path in &self.watch_paths {
            if !watch_path.exists() {
                warn!("Watch path does not exist: {:?}", watch_path);
                continue;
            }

            for entry in WalkDir::new(watch_path)
                .follow_links(true)
                .into_iter()
                .filter_map(|e| e.ok())
            {
                let path = entry.path();
                if path.is_file() && is_font_file(path) {
                    match self.scan_font_file(path).await {
                        Ok(font_info) => {
                            cache.insert(path.to_path_buf(), font_info.clone());
                            fonts.push(font_info);
                        }
                        Err(e) => {
                            error!("Failed to scan font file {:?}: {}", path, e);
                        }
                    }
                }
            }
        }

        info!("Scanned {} fonts", fonts.len());
        Ok(fonts)
    }

    async fn scan_font_file(&self, path: &Path) -> Result<FontInfo> {
        let metadata = tokio::fs::metadata(path)
            .await
            .context("Failed to get file metadata")?;

        let sha256 = calculate_sha256(path)?;
        
        Ok(FontInfo {
            path: path.to_path_buf(),
            sha256,
            size: metadata.len(),
            modified: metadata.modified()?,
        })
    }

    pub async fn start_monitoring(&mut self) -> Result<()> {
        let event_sender = self.event_sender.clone();
        let font_cache = Arc::clone(&self.font_cache);
        
        // 初始扫描：建立缓存
        self.scan_fonts().await?;
        
        // 创建文件系统监控器
        let watcher = notify::recommended_watcher(move |res: Result<Event, notify::Error>| {
            match res {
                Ok(event) => {
                    let event_sender = event_sender.clone();
                    let font_cache = Arc::clone(&font_cache);
                    
                    // 同步处理事件，避免跨线程 Send 问题
                    Self::handle_file_event_sync(event, event_sender, font_cache);
                }
                Err(e) => {
                    error!("File watcher error: {}", e);
                }
            }
        })?;

        let mut watcher = Some(watcher);

        // 监听所有路径
        for watch_path in &self.watch_paths {
            if watch_path.exists() {
                watcher.as_mut().unwrap().watch(watch_path, RecursiveMode::Recursive)?;
                info!("Started monitoring: {:?}", watch_path);
            }
        }

        // 保持 watcher 存活（直到 Ctrl+C）
        tokio::spawn(async move {
            let _watcher = watcher; // Keep watcher in scope
            tokio::signal::ctrl_c().await.ok();
            info!("File monitoring stopped");
        });

        Ok(())
    }

    async fn handle_file_event(
        event: Event,
        event_sender: mpsc::UnboundedSender<FontEvent>,
        font_cache: Arc<parking_lot::RwLock<HashMap<PathBuf, FontInfo>>>,
    ) {
        // 异步版本：对文件变更进行去重与哈希对比
        for path in event.paths {
            if !is_font_file(&path) {
                continue;
            }

            match event.kind {
                notify::EventKind::Create(_) => {
                    if let Ok(font_info) = Self::scan_single_font(&path).await {
                        let sha256 = font_info.sha256.clone();
                        font_cache.write().insert(path.clone(), font_info);
                        
                        info!(
                            "[{}] Font added: {:?} (SHA256: {})",
                            Local::now().format("%Y-%m-%d %H:%M:%S"),
                            path.file_name().unwrap_or_default(),
                            &sha256[..8]
                        );
                        
                        let _ = event_sender.send(FontEvent::Added(path, sha256));
                    }
                }
                notify::EventKind::Modify(_) => {
                    let cache = font_cache.read();
                    
                    if let Some(existing_info) = cache.get(&path) {
                        if let Ok(font_info) = Self::scan_single_font(&path).await {
                            if font_info.sha256 != existing_info.sha256 {
                                let sha256 = font_info.sha256.clone();
                                drop(cache); // Release read lock
                                font_cache.write().insert(path.clone(), font_info);
                                
                                info!(
                                    "[{}] Font modified: {:?} (SHA256: {})",
                                    Local::now().format("%Y-%m-%d %H:%M:%S"),
                                    path.file_name().unwrap_or_default(),
                                    &sha256[..8]
                                );
                                
                                let _ = event_sender.send(FontEvent::Modified(path, sha256));
                            }
                        }
                    } else {
                        // New file not in cache
                        if let Ok(font_info) = Self::scan_single_font(&path).await {
                            let sha256 = font_info.sha256.clone();
                            drop(cache); // Release read lock
                            font_cache.write().insert(path.clone(), font_info);
                            
                            info!(
                                "[{}] Font added: {:?} (SHA256: {})",
                                Local::now().format("%Y-%m-%d %H:%M:%S"),
                                path.file_name().unwrap_or_default(),
                                &sha256[..8]
                            );
                            
                            let _ = event_sender.send(FontEvent::Added(path, sha256));
                        }
                    }
                }
                notify::EventKind::Remove(_) => {
                    font_cache.write().remove(&path);
                    
                    info!(
                        "[{}] Font removed: {:?}",
                        Local::now().format("%Y-%m-%d %H:%M:%S"),
                        path.file_name().unwrap_or_default()
                    );
                    
                    let _ = event_sender.send(FontEvent::Removed(path));
                }
                _ => {}
            }
        }
    }

    fn handle_file_event_sync(
        event: Event,
        event_sender: mpsc::UnboundedSender<FontEvent>,
        font_cache: Arc<parking_lot::RwLock<HashMap<PathBuf, FontInfo>>>,
    ) {
        // 同步版本：尽量轻量处理，避免阻塞通知线程
        for path in event.paths {
            if !is_font_file(&path) {
                continue;
            }

            match event.kind {
                notify::EventKind::Create(_) => {
                    // For new files, we'll handle them in the next scan
                    info!("Font file created: {:?}", path.file_name().unwrap_or_default());
                }
                notify::EventKind::Modify(_) => {
                    info!("Font file modified: {:?}", path.file_name().unwrap_or_default());
                }
                notify::EventKind::Remove(_) => {
                    font_cache.write().remove(&path);
                    
                    info!(
                        "[{}] Font removed: {:?}",
                        Local::now().format("%Y-%m-%d %H:%M:%S"),
                        path.file_name().unwrap_or_default()
                    );
                    
                    if let Some(filename) = path.file_name().and_then(|n| n.to_str()) {
                        let _ = event_sender.send(FontEvent::Removed(path));
                    }
                }
                _ => {}
            }
        }
    }

    async fn scan_single_font(path: &Path) -> Result<FontInfo> {
        let metadata = tokio::fs::metadata(path)
            .await
            .context("Failed to get file metadata")?;

        let sha256 = calculate_sha256(path)?;
        
        Ok(FontInfo {
            path: path.to_path_buf(),
            sha256,
            size: metadata.len(),
            modified: metadata.modified()?,
        })
    }

    pub fn take_event_receiver(&mut self) -> Option<mpsc::UnboundedReceiver<FontEvent>> {
        self.event_receiver.take()
    }

    pub fn get_font_cache(&self) -> Arc<parking_lot::RwLock<HashMap<PathBuf, FontInfo>>> {
        Arc::clone(&self.font_cache)
    }
}

pub async fn monitor_font_changes(
    watch_paths: Vec<PathBuf>,
    mut event_handler: impl FnMut(FontEvent) + Send + 'static,
) -> Result<()> {
    let mut monitor = FontMonitor::new();
    
    for path in watch_paths {
        monitor.add_watch_path(path);
    }

    let mut event_receiver = monitor.take_event_receiver()
        .context("Failed to get event receiver")?;

    monitor.start_monitoring().await?;

    // Handle events
    tokio::spawn(async move {
        while let Some(event) = event_receiver.recv().await {
            event_handler(event);
        }
    });

    Ok(())
}
