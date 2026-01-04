use anyhow::Result;
use clap::{Parser, Subcommand};
use log::info;

mod client;
mod server;
mod font_installer;

#[derive(Parser)]
#[command(name = "fontsync")]
#[command(about = "Font synchronization tool with HTTP server and client")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Start HTTP server for font upload/download
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
    },
    
    /// Start HTTP client to sync fonts with server
    Client {
        /// Server URL
        #[arg(long, default_value = "http://127.0.0.1:8080")]
        server_url: String,
        
        /// Local font directory to scan and upload
        #[arg(long, default_value = "./local_fonts")]
        local_dir: String,
        
        /// Install downloaded fonts
        #[arg(long, default_value_t = true)]
        install: bool,
        
        /// Upload local fonts to server
        #[arg(long, default_value_t = true)]
        upload: bool,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    env_logger::init();
    
    let cli = Cli::parse();
    
    match cli.command {
        Commands::Serve { host, port, font_dir } => {
            info!("Starting font server on {}:{}", host, port);
            info!("Font directory: {}", font_dir);
            server::start_server(host, port, font_dir, true).await?;
        }
        Commands::Client { server_url, local_dir, install, upload } => {
            info!("Starting font client");
            info!("Server URL: {}", server_url);
            info!("Local directory: {}", local_dir);
            info!("Install fonts: {}", install);
            info!("Upload fonts: {}", upload);
            client::run_client(server_url.clone(), local_dir, install, upload, false, server_url.replace("http", "ws"), false, true).await?;
        }
    }
    
    Ok(())
}