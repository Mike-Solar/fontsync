use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use log::info;
use std::path::PathBuf;
use crate::utils::scan_font_directory;

mod client;
mod font_installer;
mod font_monitor;
#[cfg(feature = "gui")]
mod gui;
mod server;
mod utils;
mod websocket_client;
mod websocket_server;

#[derive(Parser)]
#[command(name = "fontsync")]
#[command(about = "Font synchronization tool with real-time monitoring and WebSocket notifications")]
#[command(version = "1.0.0")]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,
    
    #[arg(long, global = true, help = "Enable verbose logging")]
    verbose: bool,
    
    #[arg(long, global = true, help = "Disable GUI mode")]
    no_gui: bool,
}

#[derive(Subcommand)]
enum Commands {
    /// 启动用于字体同步的 HTTP/WebSocket 服务器
    Serve {
        /// 服务器主机地址
        #[arg(long, default_value = "127.0.0.1")]
        host: String,
        
        /// 服务器端口
        #[arg(long, default_value_t = 8080)]
        port: u16,
        
        /// 字体存储目录
        #[arg(long, default_value = "./fonts")]
        font_dir: String,
        
        /// 启用 WebSocket 通知
        #[arg(
            long,
            default_value_t = true,
            action = clap::ArgAction::Set,
            value_parser = clap::builder::BoolishValueParser::new(),
            num_args = 0..=1,
            default_missing_value = "true"
        )]
        websocket: bool,
    },
    
    /// 启动字体监控客户端
    Monitor {
        /// WebSocket 连接的服务器 URL
        #[arg(long, default_value = "ws://localhost:8080")]
        server_url: String,
        
        /// 监控目录（默认使用系统字体目录）
        #[arg(long, value_delimiter = ',')]
        watch_dirs: Option<Vec<String>>,
        
        /// 用于识别的客户端 ID
        #[arg(long, default_value = "default_client")]
        client_id: String,
        
        /// 启用交互模式用于冲突处理
        #[arg(
            long,
            default_value_t = false,
            action = clap::ArgAction::Set,
            value_parser = clap::builder::BoolishValueParser::new(),
            num_args = 0..=1,
            default_missing_value = "true"
        )]
        interactive: bool,
    },
    
    /// 执行一次性字体同步
    Sync {
        /// 服务器 URL
        #[arg(long, default_value = "http://localhost:8080")]
        server_url: String,
        
        /// 本地字体目录
        #[arg(long, default_value = "./local_fonts")]
        local_dir: String,
        
        /// 启用交互模式用于冲突处理
        #[arg(
            long,
            default_value_t = true,
            action = clap::ArgAction::Set,
            value_parser = clap::builder::BoolishValueParser::new(),
            num_args = 0..=1,
            default_missing_value = "true"
        )]
        interactive: bool,
        
        /// 上传本地字体到服务器
        #[arg(
            long,
            default_value_t = true,
            action = clap::ArgAction::Set,
            value_parser = clap::builder::BoolishValueParser::new(),
            num_args = 0..=1,
            default_missing_value = "true"
        )]
        upload: bool,
        
        /// 从服务器下载字体
        #[arg(
            long,
            default_value_t = true,
            action = clap::ArgAction::Set,
            value_parser = clap::builder::BoolishValueParser::new(),
            num_args = 0..=1,
            default_missing_value = "true"
        )]
        download: bool,
        
        /// 安装已下载字体
        #[arg(
            long,
            default_value_t = true,
            action = clap::ArgAction::Set,
            value_parser = clap::builder::BoolishValueParser::new(),
            num_args = 0..=1,
            default_missing_value = "true"
        )]
        install: bool,
    },
    
    /// 从目录安装字体
    Install {
        /// 包含字体文件的目录
        #[arg(long, default_value = "./fonts")]
        font_dir: String,
        
        /// 启用详细安装日志
        #[arg(
            long,
            default_value_t = false,
            action = clap::ArgAction::Set,
            value_parser = clap::builder::BoolishValueParser::new(),
            num_args = 0..=1,
            default_missing_value = "true"
        )]
        verbose: bool,
    },
    
    /// 列出系统字体目录
    ListFonts {
        /// 显示包含 SHA256 的详细信息
        #[arg(
            long,
            default_value_t = false,
            action = clap::ArgAction::Set,
            value_parser = clap::builder::BoolishValueParser::new(),
            num_args = 0..=1,
            default_missing_value = "true"
        )]
        detailed: bool,
    },
    
    /// 启动 GUI 界面（需要编译 GUI 支持）
    #[cfg(feature = "gui")]
    Gui {
        /// 以服务器模式启动
        #[arg(long)]
        server: bool,
        
        /// 以客户端模式启动
        #[arg(long)]
        client: bool,
        
        /// 客户端模式的服务器 URL
        #[arg(long, default_value = "http://localhost:8080")]
        server_url: String,
    },
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let command = cli.command;
    
    // 初始化日志
    if cli.verbose {
        env_logger::Builder::from_default_env()
            .filter_level(log::LevelFilter::Debug)
            .init();
    } else {
        env_logger::init();
    }
    
    // 处理 GUI 模式
    #[cfg(feature = "gui")]
    {
        if !cli.no_gui {
            if let Some(Commands::Gui { .. }) = &command {
                info!("Starting GUI interface...");
                return gui::run_gui().map_err(|e| anyhow::anyhow!("GUI error: {}", e));
            }

            if command.is_none() {
                info!("Starting GUI interface (default)...");
                return gui::run_gui().map_err(|e| anyhow::anyhow!("GUI error: {}", e));
            }

            // 检查是否需要默认启动 GUI
            if std::env::var("FONT_SYNC_GUI").is_ok() {
                info!("Starting GUI interface (via environment variable)...");
                return gui::run_gui().map_err(|e| anyhow::anyhow!("GUI error: {}", e));
            }
        } else {
            if command.is_none() {
                return Err(anyhow::anyhow!("No command provided. Use --help for usage."));
            }

            if let Some(Commands::Gui { .. }) = &command {
                return Err(anyhow::anyhow!("GUI disabled via --no-gui"));
            }
        }
    }
    
    // 在非 GUI 构建中处理 GUI 检查
    #[cfg(not(feature = "gui"))]
    {
        if !cli.no_gui {
            // 检查是否需要默认启动 GUI
            if std::env::var("FONT_SYNC_GUI").is_ok() {
                return Err(anyhow::anyhow!("GUI support not compiled. Build with --features gui"));
            }
        }
    }
    
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;

    runtime.block_on(async move {
        match command {
            Some(Commands::Serve { host, port, font_dir, websocket }) => {
                info!("Starting font server on {}:{}", host, port);
                info!("Font directory: {}", font_dir);
                info!("WebSocket enabled: {}", websocket);
                
                if websocket {
                    server::start_server_with_websocket(host, port, font_dir, true).await?;
                } else {
                    server::start_server(host, port, font_dir, false).await?;
                }
            }
            
            Some(Commands::Monitor { server_url, watch_dirs, client_id, interactive: _ }) => {
                info!("Starting font monitor client");
                info!("Server URL: {}", server_url);
                info!("Client ID: {}", client_id);
                info!("Interactive mode: {}", false);
                
                let watch_paths = if let Some(dirs) = watch_dirs {
                    dirs.into_iter().map(PathBuf::from).collect()
                } else {
                    utils::get_system_font_directories()
                };
                
                info!("Monitoring directories: {:?}", watch_paths);
                
                run_monitor_client(server_url, watch_paths, client_id, false).await?;
            }
            
            Some(Commands::Sync { server_url, local_dir, interactive, upload, download, install }) => {
                info!("Performing one-time font synchronization");
                info!("Server URL: {}", server_url);
                info!("Local directory: {}", local_dir);
                info!("Interactive mode: {}", interactive);
                info!("Upload: {}", upload);
                info!("Download: {}", download);
                info!("Install: {}", install);
                
                run_sync_command(server_url, local_dir, interactive, upload, download, install).await?;
            }
            
            Some(Commands::Install { font_dir, verbose }) => {
                info!("Installing fonts from directory: {}", font_dir);
                run_install_command(font_dir, verbose).await?;
            }
            
            Some(Commands::ListFonts { detailed }) => {
                run_list_fonts_command(detailed).await?;
            }

            None => {
                return Err(anyhow::anyhow!("No command provided. Use --help for usage."));
            }

            #[cfg(feature = "gui")]
            Some(Commands::Gui { .. }) => {
                unreachable!("GUI command handled before async runtime");
            }
            
            #[cfg(not(feature = "gui"))]
            _ => {
                // 非 GUI 构建不支持 Gui 命令，这里不应触发
                unreachable!("GUI command received in non-GUI build");
            }
        }
        
        Ok(())
    })
}

async fn run_monitor_client(
    server_url: String,
    watch_paths: Vec<PathBuf>,
    client_id: String,
    _interactive: bool,
) -> Result<()> {
    info!("Starting real-time font monitoring...");
    
    // 创建字体监控器
    let mut monitor = font_monitor::FontMonitor::new();
    for path in watch_paths {
        monitor.add_watch_path(path);
    }
    
    // 初始扫描
    let initial_fonts = monitor.scan_fonts().await?;
    info!("Found {} fonts during initial scan", initial_fonts.len());
    
    // 连接 WebSocket 服务器
    let _ws_client = websocket_client::start_websocket_client(server_url, client_id).await?;
    
    // 开始监控
    let mut event_receiver = monitor.take_event_receiver()
        .context("Failed to get event receiver")?;
    
    monitor.start_monitoring().await?;
    
    // 处理字体事件
    tokio::spawn(async move {
        while let Some(event) = event_receiver.recv().await {
            match event {
                font_monitor::FontEvent::Added(path, sha256) => {
                    info!("Font added: {:?} (SHA256: {}...)", 
                        path.file_name().unwrap_or_default(), 
                        &sha256[..8]
                    );
                }
                font_monitor::FontEvent::Modified(path, sha256) => {
                    info!("Font modified: {:?} (SHA256: {}...)", 
                        path.file_name().unwrap_or_default(), 
                        &sha256[..8]
                    );
                }
                font_monitor::FontEvent::Removed(path) => {
                    info!("Font removed: {:?}", path.file_name().unwrap_or_default());
                }
            }
        }
    });
    
    info!("Font monitoring started. Press Ctrl+C to stop.");
    
    // 持续运行直到被中断
    tokio::signal::ctrl_c().await?;
    info!("Shutting down font monitor...");
    
    Ok(())
}

async fn run_sync_command(
    server_url: String,
    local_dir: String,
    interactive: bool,
    upload: bool,
    download: bool,
    install: bool,
) -> Result<()> {
    let local_dir_path = PathBuf::from(&local_dir);
    
    // 本地目录不存在时创建
    if !local_dir_path.exists() {
        tokio::fs::create_dir_all(&local_dir_path).await
            .context("Failed to create local directory")?;
        info!("Created local directory: {}", local_dir);
    }
    
    let mut total_uploaded = 0;
    let mut total_downloaded = 0;
    
    if upload {
        info!("Uploading local fonts to server...");
        let (uploaded, _) = client::upload_local_fonts(&server_url, &local_dir_path, interactive).await?;
        total_uploaded += uploaded;
        info!("Upload complete: {} fonts uploaded", uploaded);
    }
    
    if download {
        info!("Downloading fonts from server...");
        let (downloaded, _) = client::download_server_fonts(&server_url, &local_dir_path, interactive).await?;
        total_downloaded += downloaded;
        info!("Download complete: {} fonts downloaded", downloaded);
    }
    
    if install && total_downloaded > 0 {
        info!("Installing downloaded fonts...");
        let (installed, failed) = client::install_downloaded_fonts(&local_dir_path).await?;
        info!("Installation complete: {} installed, {} failed", installed, failed);
    }
    
    info!("Synchronization complete: {} uploaded, {} downloaded", total_uploaded, total_downloaded);
    
    Ok(())
}

async fn run_install_command(font_dir: String, verbose: bool) -> Result<()> {
    let font_dir_path = PathBuf::from(&font_dir);
    
    if !font_dir_path.exists() {
        return Err(anyhow::anyhow!("Font directory does not exist: {}", font_dir));
    }
    
    info!("Installing fonts from directory: {}", font_dir);
    
    let (installed, failed) = font_installer::install_fonts_from_directory(&font_dir_path).await?;
    
    if verbose {
        info!("Installation details:");
        info!("  Successfully installed: {} fonts", installed);
        info!("  Failed to install: {} fonts", failed);
    } else {
        info!("Installation complete: {} installed, {} failed", installed, failed);
    }
    
    Ok(())
}

async fn run_list_fonts_command(detailed: bool) -> Result<()> {
    let font_dirs = utils::get_system_font_directories();
    
    println!("System font directories:");
    for (i, dir) in font_dirs.iter().enumerate() {
        println!("  {}. {}", i + 1, dir.display());
        
        if detailed && dir.exists() {
            match scan_font_directory(dir).await {
                Ok(fonts) => {
                    for font in fonts {
                        println!("     - {} ({})", 
                            font.path.file_name().unwrap_or_default().to_string_lossy(),
                            utils::format_file_size(font.size)
                        );
                        if detailed {
                            println!("       SHA256: {}...", &font.sha256[..16]);
                        }
                    }
                }
                Err(e) => {
                    println!("     Error scanning directory: {}", e);
                }
            }
        }
    }
    
    Ok(())
}
