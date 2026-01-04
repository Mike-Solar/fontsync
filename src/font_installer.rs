use anyhow::{Context, Result};
use log::{error, info};
use std::path::Path;
use std::process::Command;

pub async fn install_font(font_path: &Path) -> Result<()> {
    #[cfg(target_os = "windows")]
    return install_font_windows(font_path).await;
    
    #[cfg(target_os = "linux")]
    return install_font_linux(font_path).await;
    
    #[cfg(target_os = "macos")]
    return install_font_macos(font_path).await;
    
    #[cfg(not(any(target_os = "windows", target_os = "linux", target_os = "macos")))]
    return Err(anyhow::anyhow!("Font installation not supported on this OS"));
}

pub async fn install_fonts_from_directory(dir_path: &Path) -> Result<(usize, usize)> {
    let mut installed = 0;
    let mut failed = 0;
    
    use walkdir::WalkDir;
    
    for entry in WalkDir::new(dir_path)
        .max_depth(1)
        .into_iter()
        .filter_map(|e| e.ok())
    {
        let path = entry.path();
        if path.is_file() && is_font_file(path) {
            match install_font(path).await {
                Ok(_) => {
                    info!("Successfully installed font: {:?}", path.file_name().unwrap_or_default());
                    installed += 1;
                }
                Err(e) => {
                    error!("Failed to install font {:?}: {}", path.file_name().unwrap_or_default(), e);
                    failed += 1;
                }
            }
        }
    }
    
    Ok((installed, failed))
}

#[cfg(target_os = "windows")]
async fn install_font_windows(font_path: &Path) -> Result<()> {
    use std::fs;
    use windows::Win32::UI::WindowsAndMessaging::{AddFontResourceW, FR_PRIVATE};

    info!("Installing font on Windows: {:?}", font_path);

    // Get Windows fonts directory
    let fonts_dir = dirs::font_dir()
        .context("Failed to get fonts directory")?;

    let font_filename = font_path
        .file_name()
        .context("Failed to get font filename")?;
    
    let target_path = fonts_dir.join(font_filename);

    // Copy font to fonts directory
    fs::copy(font_path, &target_path)
        .context("Failed to copy font to fonts directory")?;

    info!("Font copied to: {:?}", target_path);

    // Add font resource
    let font_path_wide: Vec<u16> = target_path
        .to_string_lossy()
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect();

    unsafe {
        let result = AddFontResourceW(font_path_wide.as_ptr());
        if result == 0 {
            error!("Failed to add font resource");
            return Err(anyhow::anyhow!("AddFontResourceW failed"));
        }
        info!("Font resource added successfully");
    }

    // Notify other applications about font change
    // Broadcast WM_FONTCHANGE message
    use windows::Win32::UI::WindowsAndMessaging::{HWND_BROADCAST, SendMessageW, WM_FONTCHANGE};
    
    unsafe {
        SendMessageW(HWND_BROADCAST, WM_FONTCHANGE, None, None);
        info!("Font change notification sent");
    }

    Ok(())
}

fn is_font_file(path: &Path) -> bool {
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

#[cfg(target_os = "linux")]
async fn install_font_linux(font_path: &Path) -> Result<()> {
    use std::fs;
    
    info!("Installing font on Linux: {:?}", font_path);

    // Get user fonts directory
    let home_dir = dirs::home_dir()
        .context("Failed to get home directory")?;
    
    let user_fonts_dir = home_dir.join(".local/share/fonts");
    
    // Create fonts directory if it doesn't exist
    if !user_fonts_dir.exists() {
        fs::create_dir_all(&user_fonts_dir)
            .context("Failed to create fonts directory")?;
    }

    let font_filename = font_path
        .file_name()
        .context("Failed to get font filename")?;
    
    let target_path = user_fonts_dir.join(font_filename);

    // Copy font to fonts directory
    fs::copy(font_path, &target_path)
        .context("Failed to copy font to fonts directory")?;

    info!("Font copied to: {:?}", target_path);

    // Update font cache
    update_font_cache()?;

    Ok(())
}

#[cfg(target_os = "macos")]
async fn install_font_macos(font_path: &Path) -> Result<()> {
    use std::fs;
    
    info!("Installing font on macOS: {:?}", font_path);

    // Get user fonts directory
    let home_dir = dirs::home_dir()
        .context("Failed to get home directory")?;
    
    let user_fonts_dir = home_dir.join("Library/Fonts");
    
    // Create fonts directory if it doesn't exist
    if !user_fonts_dir.exists() {
        fs::create_dir_all(&user_fonts_dir)
            .context("Failed to create fonts directory")?;
    }

    let font_filename = font_path
        .file_name()
        .context("Failed to get font filename")?;
    
    let target_path = user_fonts_dir.join(font_filename);

    // Copy font to fonts directory
    fs::copy(font_path, &target_path)
        .context("Failed to copy font to fonts directory")?;

    info!("Font copied to: {:?}", target_path);

    // Update font cache on macOS
    Command::new("atsutil")
        .args(["databases", "-remove"])
        .status()
        .context("Failed to update font cache")?;

    Ok(())
}

#[cfg(target_os = "linux")]
fn update_font_cache() -> Result<()> {
    info!("Updating font cache...");
    
    // Try fc-cache first (most common)
    if Command::new("fc-cache").arg("-f").status().is_ok() {
        info!("Font cache updated using fc-cache");
        return Ok(());
    }
    
    // Try mkfontdir and mkfontscale for older systems
    if Command::new("mkfontdir").status().is_ok() {
        info!("Font cache updated using mkfontdir");
    }
    
    if Command::new("mkfontscale").status().is_ok() {
        info!("Font cache updated using mkfontscale");
    }
    
    // Try xset for X11 systems
    if Command::new("xset").args(["fp", "rehash"]).status().is_ok() {
        info!("Font cache updated using xset");
    }
    
    Ok(())
}