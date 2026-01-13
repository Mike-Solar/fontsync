#[cfg(feature = "gui")]
use fltk::{
    app,
    button::Button,
    enums::{Align, Color, Event, Font, FrameType},
    frame::Frame,
    group::{Group, Pack, PackType},
    input::{Input, IntInput},
    prelude::*,
    text::{TextBuffer, TextDisplay},
    window::Window,
};
use anyhow::Result;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use tokio::runtime::Runtime;
use tray_item::{IconSource, TrayItem};

#[cfg(target_os = "windows")]
fn tray_icon_source() -> Option<IconSource> {
    use windows_sys::Win32::UI::WindowsAndMessaging::{LoadIconW, IDI_APPLICATION};
    let icon = unsafe { LoadIconW(0, IDI_APPLICATION) };
    if icon == 0 {
        None
    } else {
        Some(IconSource::RawIcon(icon))
    }
}

#[cfg(not(target_os = "windows"))]
fn tray_icon_source() -> Option<IconSource> {
    Some(IconSource::Resource("preferences-desktop-font"))
}

use crate::utils::get_system_font_directories;

#[derive(Clone)]
struct AppState {
    server_running: Arc<Mutex<bool>>,
    client_connected: Arc<Mutex<bool>>,
    sync_in_progress: Arc<Mutex<bool>>,
    server_url: Arc<Mutex<String>>,
    status_message: Arc<Mutex<String>>,
}

impl AppState {
    fn new() -> Self {
        Self {
            server_running: Arc::new(Mutex::new(false)),
            client_connected: Arc::new(Mutex::new(false)),
            sync_in_progress: Arc::new(Mutex::new(false)),
            server_url: Arc::new(Mutex::new("http://localhost:8080".to_string())),
            status_message: Arc::new(Mutex::new("Ready".to_string())),
        }
    }
}

#[derive(Debug, Clone, Copy)]
enum TrayEvent {
    Show,
    Hide,
    Quit,
}

pub fn run_gui() -> Result<()> {
    let app = app::App::default();
    app::set_scheme(app::Scheme::Gtk);
    
    let mut wind = Window::default()
        .with_size(800, 600)
        .with_label("FontSync - Font Synchronization Tool");
    wind.set_color(Color::from_rgb(247, 244, 236));
    
    let state = AppState::new();
    let runtime = Arc::new(Runtime::new()?);
    
    // Main layout
    let mut main_pack = Pack::default()
        .with_pos(10, 10)
        .with_size(780, 580);
    main_pack.set_type(PackType::Vertical);
    main_pack.set_spacing(8);
    
    // Server section title + divider
    let mut server_title = Frame::default()
        .with_size(0, 24)
        .with_label("服务端");
    server_title.set_label_size(17);
    server_title.set_label_font(Font::HelveticaBold);
    server_title.set_label_color(Color::from_rgb(40, 40, 40));
    server_title.set_align(Align::Left | Align::Inside);

    let mut server_divider = Frame::default().with_size(0, 1);
    server_divider.set_frame(FrameType::FlatBox);
    server_divider.set_color(Color::from_rgb(200, 200, 200));
    
    let mut server_pack = Pack::default()
        .with_size(0, 140);
    server_pack.set_type(PackType::Vertical);
    server_pack.set_spacing(6);
    
    let mut server_row1 = Pack::default().with_size(0, 28);
    server_row1.set_type(PackType::Horizontal);
    server_row1.set_spacing(16);
    let mut server_host_label = Frame::default()
        .with_size(90, 28)
        .with_label("监听地址");
    server_host_label.set_label_size(12);
    server_host_label.set_align(Align::Left | Align::Inside);
    let mut server_host_input = Input::default()
        .with_size(220, 28);
    server_host_input.set_text_size(13);
    server_host_input.set_value("127.0.0.1");
    server_row1.end();
    
    let mut server_row2 = Pack::default().with_size(0, 28);
    server_row2.set_type(PackType::Horizontal);
    server_row2.set_spacing(16);
    let mut server_port_label = Frame::default()
        .with_size(90, 28)
        .with_label("监听端口");
    server_port_label.set_label_size(12);
    server_port_label.set_align(Align::Left | Align::Inside);
    let mut server_port_input = IntInput::default()
        .with_size(220, 28);
    server_port_input.set_text_size(13);
    server_port_input.set_value("8080");
    server_row2.end();
    
    let mut server_row3 = Pack::default().with_size(0, 28);
    server_row3.set_type(PackType::Horizontal);
    server_row3.set_spacing(16);
    let mut server_dir_label = Frame::default()
        .with_size(90, 28)
        .with_label("字体目录");
    server_dir_label.set_label_size(12);
    server_dir_label.set_align(Align::Left | Align::Inside);
    let mut server_font_dir_input = Input::default()
        .with_size(360, 28);
    server_font_dir_input.set_text_size(13);
    server_font_dir_input.set_value("./fonts");
    server_row3.end();
    
    let mut server_button_pack = Pack::default().with_size(0, 30);
    server_button_pack.set_type(PackType::Horizontal);
    server_button_pack.set_spacing(16);
    
    let mut start_server_btn = Button::default()
        .with_size(96, 28)
        .with_label("开启服务");
    start_server_btn.set_color(Color::from_rgb(255, 255, 255));
    start_server_btn.set_label_color(Color::from_rgb(49, 99, 239));
    start_server_btn.set_frame(FrameType::BorderBox);
    
    let mut stop_server_btn = Button::default()
        .with_size(96, 28)
        .with_label("停止服务");
    stop_server_btn.set_color(Color::from_rgb(255, 255, 255));
    stop_server_btn.set_label_color(Color::from_rgb(49, 99, 239));
    stop_server_btn.set_frame(FrameType::BorderBox);
    stop_server_btn.deactivate();

    let mut stop_server_btn_for_start = stop_server_btn.clone();
    
    server_button_pack.end();
    server_pack.end();
    
    let mut section_spacer = Frame::default().with_size(0, 6);
    section_spacer.set_frame(FrameType::NoBox);
    
    let mut client_title = Frame::default()
        .with_size(0, 24)
        .with_label("客户端");
    client_title.set_label_size(17);
    client_title.set_label_font(Font::HelveticaBold);
    client_title.set_label_color(Color::from_rgb(40, 40, 40));
    client_title.set_align(Align::Left | Align::Inside);

    let mut client_divider = Frame::default().with_size(0, 1);
    client_divider.set_frame(FrameType::FlatBox);
    client_divider.set_color(Color::from_rgb(200, 200, 200));
    
    let mut client_pack = Pack::default()
        .with_size(0, 120);
    client_pack.set_type(PackType::Vertical);
    client_pack.set_spacing(6);
    
    let mut client_row1 = Pack::default().with_size(0, 28);
    client_row1.set_type(PackType::Horizontal);
    client_row1.set_spacing(16);
    let mut client_host_label = Frame::default()
        .with_size(90, 28)
        .with_label("服务器地址");
    client_host_label.set_label_size(12);
    client_host_label.set_align(Align::Left | Align::Inside);
    let mut client_host_input = Input::default()
        .with_size(220, 28);
    client_host_input.set_text_size(13);
    client_host_input.set_value("127.0.0.1");
    client_row1.end();

    let mut client_row2 = Pack::default().with_size(0, 28);
    client_row2.set_type(PackType::Horizontal);
    client_row2.set_spacing(16);
    let mut client_port_label = Frame::default()
        .with_size(90, 28)
        .with_label("服务器端口");
    client_port_label.set_label_size(12);
    client_port_label.set_align(Align::Left | Align::Inside);
    let mut client_port_input = IntInput::default()
        .with_size(220, 28);
    client_port_input.set_text_size(13);
    client_port_input.set_value("8080");
    client_row2.end();
    
    let mut client_button_pack = Pack::default().with_size(0, 30);
    client_button_pack.set_type(PackType::Horizontal);
    client_button_pack.set_spacing(16);
    
    let mut connect_client_btn = Button::default()
        .with_size(96, 28)
        .with_label("开始同步");
    connect_client_btn.set_color(Color::from_rgb(255, 255, 255));
    connect_client_btn.set_label_color(Color::from_rgb(49, 99, 239));
    connect_client_btn.set_frame(FrameType::BorderBox);
    
    let mut disconnect_client_btn = Button::default()
        .with_size(96, 28)
        .with_label("停止同步");
    disconnect_client_btn.set_color(Color::from_rgb(255, 255, 255));
    disconnect_client_btn.set_label_color(Color::from_rgb(49, 99, 239));
    disconnect_client_btn.set_frame(FrameType::BorderBox);
    disconnect_client_btn.deactivate();
    
    let mut sync_once_btn = Button::default()
        .with_size(96, 28)
        .with_label("仅同步一次");
    sync_once_btn.set_color(Color::from_rgb(255, 255, 255));
    sync_once_btn.set_label_color(Color::from_rgb(49, 99, 239));
    sync_once_btn.set_frame(FrameType::BorderBox);

    let mut disconnect_client_btn_for_connect = disconnect_client_btn.clone();
    let mut sync_once_btn_for_connect = sync_once_btn.clone();
    let mut sync_once_btn_for_disconnect = sync_once_btn.clone();
    let client_host_input_for_connect = client_host_input.clone();
    let client_port_input_for_connect = client_port_input.clone();
    let client_host_input_for_sync = client_host_input.clone();
    let client_port_input_for_sync = client_port_input.clone();
    
    client_button_pack.end();
    client_pack.end();
    
    let mut status_title = Frame::default()
        .with_size(0, 20)
        .with_label("日志");
    status_title.set_label_size(14);
    status_title.set_label_font(Font::HelveticaBold);
    status_title.set_label_color(Color::from_rgb(40, 40, 40));
    status_title.set_align(Align::Left | Align::Inside);

    let mut status_group = Group::default()
        .with_size(780, 170);
    status_group.set_frame(FrameType::EngravedBox);
    
    let mut status_text = TextDisplay::default()
        .with_pos(10, 12)
        .with_size(760, 146);
    status_text.set_text_font(Font::Courier);
    status_text.set_text_size(11);
    status_text.set_scrollbar_size(15);
    status_text.set_frame(FrameType::DownBox);
    status_text.set_color(Color::from_rgb(252, 250, 246));
    
    status_group.end();
    
    main_pack.end();
    wind.end();
    wind.show();

    let (tray_sender, tray_receiver) = app::channel::<TrayEvent>();
    let mut tray = match tray_icon_source() {
        Some(icon) => match TrayItem::new("FontSync", icon) {
            Ok(tray) => Some(tray),
            Err(e) => {
                eprintln!("Failed to create tray icon: {}", e);
                None
            }
        },
        None => {
            eprintln!("Failed to create tray icon");
            None
        }
    };
    let tray_enabled = tray.is_some();

    if let Some(tray) = tray.as_mut() {
        let sender = tray_sender;
        let _ = tray.add_menu_item("Show", move || sender.send(TrayEvent::Show));
        let sender = tray_sender;
        let _ = tray.add_menu_item("Hide", move || sender.send(TrayEvent::Hide));
        let sender = tray_sender;
        let _ = tray.add_menu_item("Quit", move || sender.send(TrayEvent::Quit));
    }

    let tray_sender_for_close = tray_sender;
    wind.set_callback(move |w| {
        if app::event() == Event::Close {
            if tray_enabled {
                app::program_should_quit(false);
                w.hide();
                tray_sender_for_close.send(TrayEvent::Hide);
            } else {
                app::quit();
            }
        }
    });
    
    // Create status buffer
    let status_buffer = TextBuffer::default();
    status_text.set_buffer(status_buffer.clone());
    
    // Helper function to update status
    let update_status = {
        let status_buffer = status_buffer.clone();
        move |message: &str| {
            let timestamp = chrono::Local::now().format("%Y-%m-%d %H:%M:%S");
            let log_message = format!("[{}] {}\n", timestamp, message);
            
            let mut buffer = status_buffer.clone();
            let current_text = buffer.text();
            let new_text = format!("{}{}", current_text, log_message);
            
            // Limit log size to prevent memory issues
            let lines: Vec<&str> = new_text.lines().collect();
            let trimmed_text = if lines.len() > 1000 {
                lines[lines.len() - 1000..].join("\n")
            } else {
                new_text
            };
            
            buffer.set_text(&trimmed_text);
        }
    };
    
    // Server button handlers
    let state_clone = state.clone();
    let runtime_clone = runtime.clone();
    let update_status_for_start = update_status.clone();
    
    start_server_btn.set_callback(move |btn| {
        let state = state_clone.clone();
        let runtime = runtime_clone.clone();
        let update_status = update_status_for_start.clone();

        btn.deactivate();
        stop_server_btn_for_start.activate();

        let host = server_host_input.value();
        let port: u16 = server_port_input.value().parse().unwrap_or(8080);
        let font_dir = server_font_dir_input.value();
        
        update_status(&format!("Starting server on {}:{} with font directory: {}", host, port, font_dir));
        *state.server_running.lock().unwrap() = true;

        std::thread::spawn(move || {
            if let Err(e) = runtime.block_on(start_server_internal(host, port, font_dir)) {
                *state.server_running.lock().unwrap() = false;
                eprintln!("Failed to start server: {}", e);
            }
        });
    });
    
    let state_clone = state.clone();
    let update_status_for_stop = update_status.clone();
    stop_server_btn.set_callback(move |btn| {
        let state = state_clone.clone();
        let update_status = update_status_for_stop.clone();
        
        btn.deactivate();
        start_server_btn.activate();
        
        *state.server_running.lock().unwrap() = false;
        update_status("Server stopped");
    });
    
    // Client button handlers
    let state_clone = state.clone();
    let runtime_clone = runtime.clone();
    let update_status_for_connect = update_status.clone();
    
    connect_client_btn.set_callback(move |btn| {
        let state = state_clone.clone();
        let runtime = runtime_clone.clone();
        let update_status = update_status_for_connect.clone();

        btn.deactivate();
        disconnect_client_btn_for_connect.activate();
        sync_once_btn_for_connect.deactivate();
        
        let host_value = client_host_input_for_connect.value();
        let host = if host_value.trim().is_empty() {
            "127.0.0.1".to_string()
        } else {
            host_value
        };
        let port: u16 = client_port_input_for_connect.value().parse().unwrap_or(8080);
        let server_url = format!("http://{}:{}", host.trim(), port);
        *state.server_url.lock().unwrap() = server_url.clone();
        update_status(&format!("Connecting to server: {}", server_url));

        match runtime.block_on(connect_client_internal(server_url)) {
            Ok(_) => {
                *state.client_connected.lock().unwrap() = true;
                update_status("Client connected successfully");
            }
            Err(e) => {
                *state.client_connected.lock().unwrap() = false;
                update_status(&format!("Failed to connect client: {}", e));
                btn.activate();
                disconnect_client_btn_for_connect.deactivate();
                sync_once_btn_for_connect.activate();
            }
        }
    });
    
    let state_clone = state.clone();
    let update_status_for_disconnect = update_status.clone();
    disconnect_client_btn.set_callback(move |btn| {
        let state = state_clone.clone();
        let update_status = update_status_for_disconnect.clone();
        
        btn.deactivate();
        connect_client_btn.activate();
        sync_once_btn_for_disconnect.activate();
        
        *state.client_connected.lock().unwrap() = false;
        update_status("Client disconnected");
    });
    
    let state_clone = state.clone();
    let runtime_clone = runtime.clone();
    let update_status_for_sync = update_status.clone();
    
    sync_once_btn.set_callback(move |_| {
        let state = state_clone.clone();
        let runtime = runtime_clone.clone();
        let update_status = update_status_for_sync.clone();
        
        let host_value = client_host_input_for_sync.value();
        let host = if host_value.trim().is_empty() {
            "127.0.0.1".to_string()
        } else {
            host_value
        };
        let port: u16 = client_port_input_for_sync.value().parse().unwrap_or(8080);
        let server_url = format!("http://{}:{}", host.trim(), port);
        *state.server_url.lock().unwrap() = server_url.clone();
        update_status(&format!("Performing one-time sync with server: {}", server_url));

        match runtime.block_on(perform_one_time_sync(server_url)) {
            Ok((uploaded, downloaded)) => {
                update_status(&format!("One-time sync completed: {} uploaded, {} downloaded", uploaded, downloaded));
            }
            Err(e) => {
                update_status(&format!("One-time sync failed: {}", e));
            }
        }
    });
    
    // Timer for periodic updates
    app::add_timeout3(1.0, {
        let state = state.clone();
        move |handle| {
            let server_running = *state.server_running.lock().unwrap();
            let client_connected = *state.client_connected.lock().unwrap();
            
            if server_running {
                // Update server status
            }
            
            if client_connected {
                // Update client status
            }
            
            if server_running || client_connected {
                app::repeat_timeout3(1.0, handle);
            }
        }
    });
    
    while app.wait() {
        if let Some(event) = tray_receiver.recv() {
            match event {
                TrayEvent::Show => {
                    wind.show();
                    wind.redraw();
                }
                TrayEvent::Hide => {
                    wind.hide();
                }
                TrayEvent::Quit => break,
            }
        }
    }
    Ok(())
}

async fn start_server_internal(host: String, port: u16, font_dir: String) -> Result<()> {
    use crate::server;
    
    server::start_server_with_websocket(host, port, font_dir, true).await
}

async fn connect_client_internal(server_url: String) -> Result<()> {
    use crate::websocket_client;
    
    let client_id = format!("gui_client_{}", uuid::Uuid::new_v4());
    let _client = websocket_client::start_websocket_client(server_url, client_id).await?;
    
    // The client runs in the background
    Ok(())
}

async fn perform_one_time_sync(server_url: String) -> Result<(usize, usize)> {
    use crate::client;
    
    let local_font_dirs = get_system_font_directories();
    let download_dir = dirs::cache_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("fontsync/downloads");
    
    tokio::fs::create_dir_all(&download_dir).await?;
    
    let mut total_uploaded = 0;
    let mut total_downloaded = 0;
    
    // Upload local fonts
    for font_dir in local_font_dirs {
        if font_dir.exists() {
            let (uploaded, _) = client::upload_local_fonts(&server_url, &font_dir, false).await?;
            total_uploaded += uploaded;
        }
    }
    
    // Download server fonts
    let (downloaded, _) = client::download_server_fonts(&server_url, &download_dir, false).await?;
    total_downloaded += downloaded;
    
    // Install downloaded fonts
    if total_downloaded > 0 {
        client::install_downloaded_fonts(&download_dir).await?;
    }
    
    Ok((total_uploaded, total_downloaded))
}
