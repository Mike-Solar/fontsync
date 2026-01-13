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
    /// Start HTTP/WebSocket server for font synchronization
    Serve {
        /// Server host address
        #[arg(long, default_value = "127.0.0.1")]
        host: String,
        
        /// Server port
        #[arg(long, default_value_t = 8080)]
        port: u16,
        
        /// Font storage directory
        #[arg(long, default_value = "./fonts")]
        font_dir: String,
        
        /// Enable WebSocket notifications
        #[arg(long, default_value_t = true)]
        websocket: bool,
    },
    
    /// Start font monitoring client
    Monitor {
        /// Server URL for WebSocket connection
        #[arg(long, default_value = "ws://localhost:8080")]
        server_url: String,
        
        /// Directories to monitor (defaults to system font directories)
        #[arg(long, value_delimiter = ',')]
        watch_dirs: Option<Vec<String>>,
        
        /// Client ID for identification
        #[arg(long, default_value = "default_client")]
        client_id: String,
        
        /// Enable interactive mode for conflict resolution
        #[arg(long, default_value_t = false)]
        interactive: bool,
    },
    
    /// Perform one-time font synchronization
    Sync {
        /// Server URL
        #[arg(long, default_value = "http://localhost:8080")]
        server_url: String,
        
        /// Local font directory
        #[arg(long, default_value = "./local_fonts")]
        local_dir: String,
        
        /// Enable interactive mode for conflict resolution
        #[arg(long, default_value_t = true)]
        interactive: bool,
        
        /// Upload local fonts to server
        #[arg(long, default_value_t = true)]
        upload: bool,
        
        /// Download fonts from server
        #[arg(long, default_value_t = true)]
        download: bool,
        
        /// Install downloaded fonts
        #[arg(long, default_value_t = true)]
        install: bool,
    },
    
    /// Install fonts from a directory
    Install {
        /// Directory containing font files
        #[arg(long, default_value = "./fonts")]
        font_dir: String,
        
        /// Enable verbose installation logging
        #[arg(long, default_value_t = false)]
        verbose: bool,
    },
    
    /// List system font directories
    ListFonts {
        /// Show detailed information including SHA256
        #[arg(long, default_value_t = false)]
        detailed: bool,
    },
    
    /// Start GUI interface (if compiled with GUI support)
    #[cfg(feature = "gui")]
    Gui {
        /// Start in server mode
        #[arg(long)]
        server: bool,
        
        /// Start in client mode
        #[arg(long)]
        client: bool,
        
        /// Server URL for client mode
        #[arg(long, default_value = "http://localhost:8080")]
        server_url: String,
    },
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let command = cli.command;
    
    // Initialize logging
    if cli.verbose {
        env_logger::Builder::from_default_env()
            .filter_level(log::LevelFilter::Debug)
            .init();
    } else {
        env_logger::init();
    }
    
    // Handle GUI mode
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

            // Check if GUI should be started by default
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
    
    // Handle GUI checks for non-GUI builds
    #[cfg(not(feature = "gui"))]
    {
        if !cli.no_gui {
            // Check if GUI should be started by default
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
                // This should not happen because Gui command is not available in non-gui builds
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
    
    // Create font monitor
    let mut monitor = font_monitor::FontMonitor::new();
    for path in watch_paths {
        monitor.add_watch_path(path);
    }
    
    // Initial scan
    let initial_fonts = monitor.scan_fonts().await?;
    info!("Found {} fonts during initial scan", initial_fonts.len());
    
    // Connect to WebSocket server
    let _ws_client = websocket_client::start_websocket_client(server_url, client_id).await?;
    
    // Start monitoring
    let mut event_receiver = monitor.take_event_receiver()
        .context("Failed to get event receiver")?;
    
    monitor.start_monitoring().await?;
    
    // Handle font events
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
    
    // Keep running until interrupted
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
    
    // Create local directory if it doesn't exist
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
