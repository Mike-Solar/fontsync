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
    use windows_sys::Win32::System::Registry::{
        RegCloseKey, RegCreateKeyExW, RegSetValueExW, HKEY, HKEY_LOCAL_MACHINE, KEY_SET_VALUE,
        REG_OPTION_NON_VOLATILE, REG_SZ,
    };

    info!("Installing font on Windows: {:?}", font_path);

    // 获取 Windows 字体目录（固定为 %WINDIR%\\Fonts）
    let fonts_dir = std::env::var_os("WINDIR")
        .map(|win_dir| std::path::PathBuf::from(win_dir).join("Fonts"))
        .context("Failed to get fonts directory")?;

    let font_filename = font_path
        .file_name()
        .context("Failed to get font filename")?;
    
    let target_path = fonts_dir.join(font_filename);

    // 复制字体到字体目录
    fs::copy(font_path, &target_path)
        .context("Failed to copy font to fonts directory")?;

    info!("Font copied to: {:?}", target_path);

    // 写入注册表，确保字体对系统可见
    let mut key: HKEY = 0;
    let subkey = "SOFTWARE\\Microsoft\\Windows NT\\CurrentVersion\\Fonts";
    let subkey_wide: Vec<u16> = subkey.encode_utf16().chain(std::iter::once(0)).collect();
    let status = unsafe {
        RegCreateKeyExW(
            HKEY_LOCAL_MACHINE,
            subkey_wide.as_ptr(),
            0,
            std::ptr::null_mut(),
            REG_OPTION_NON_VOLATILE,
            KEY_SET_VALUE,
            std::ptr::null(),
            &mut key,
            std::ptr::null_mut(),
        )
    };
    if status != 0 {
        return Err(anyhow::anyhow!("Failed to open fonts registry key: {}", status));
    }

    let value_name = target_path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("FontSyncFont");
    let value_name_wide: Vec<u16> = value_name.encode_utf16().chain(std::iter::once(0)).collect();
    let value_data = format!("{}", target_path.file_name().unwrap_or_default().to_string_lossy());
    let mut value_data_wide: Vec<u16> = value_data.encode_utf16().collect();
    value_data_wide.push(0);
    let status = unsafe {
        RegSetValueExW(
            key,
            value_name_wide.as_ptr(),
            0,
            REG_SZ,
            value_data_wide.as_ptr() as *const u8,
            (value_data_wide.len() * 2) as u32,
        )
    };
    unsafe {
        RegCloseKey(key);
    }
    if status != 0 {
        return Err(anyhow::anyhow!("Failed to write font registry value: {}", status));
    }
    info!("Font registered in registry");

    // 通知其他应用字体发生变化
    // 广播 WM_FONTCHANGE 消息
    use windows_sys::Win32::Graphics::Gdi::GdiFlush;
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        SendMessageTimeoutW, HWND_BROADCAST, SMTO_ABORTIFHUNG, WM_FONTCHANGE,
    };
    
    unsafe {
        let mut result = 0;
        SendMessageTimeoutW(
            HWND_BROADCAST,
            WM_FONTCHANGE,
            0,
            0,
            SMTO_ABORTIFHUNG,
            1000,
            &mut result,
        );
        GdiFlush();
        info!("Font change notification sent");
    }
    // 等待系统刷新字体列表，避免安装后立即检查失败
    std::thread::sleep(std::time::Duration::from_secs(2));

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

    // 获取用户字体目录
    let home_dir = dirs::home_dir()
        .context("Failed to get home directory")?;
    
    let user_fonts_dir = home_dir.join(".local/share/fonts");
    
    // 字体目录不存在时创建
    if !user_fonts_dir.exists() {
        fs::create_dir_all(&user_fonts_dir)
            .context("Failed to create fonts directory")?;
    }

    let font_filename = font_path
        .file_name()
        .context("Failed to get font filename")?;
    
    let target_path = user_fonts_dir.join(font_filename);

    // 复制字体到字体目录
    fs::copy(font_path, &target_path)
        .context("Failed to copy font to fonts directory")?;

    info!("Font copied to: {:?}", target_path);

    // 更新字体缓存
    update_font_cache()?;

    Ok(())
}

#[cfg(target_os = "macos")]
async fn install_font_macos(font_path: &Path) -> Result<()> {
    use std::fs;
    
    info!("Installing font on macOS: {:?}", font_path);

    // 获取用户字体目录
    let home_dir = dirs::home_dir()
        .context("Failed to get home directory")?;
    
    let user_fonts_dir = home_dir.join("Library/Fonts");
    
    // 字体目录不存在时创建
    if !user_fonts_dir.exists() {
        fs::create_dir_all(&user_fonts_dir)
            .context("Failed to create fonts directory")?;
    }

    let font_filename = font_path
        .file_name()
        .context("Failed to get font filename")?;
    
    let target_path = user_fonts_dir.join(font_filename);

    // 复制字体到字体目录
    fs::copy(font_path, &target_path)
        .context("Failed to copy font to fonts directory")?;

    info!("Font copied to: {:?}", target_path);

    // 在 macOS 上更新字体缓存
    Command::new("atsutil")
        .args(["databases", "-remove"])
        .status()
        .context("Failed to update font cache")?;

    Ok(())
}

#[cfg(target_os = "linux")]
fn update_font_cache() -> Result<()> {
    info!("Updating font cache...");
    
    // 优先尝试 fc-cache（最常见）
    if Command::new("fc-cache").arg("-f").status().is_ok() {
        info!("Font cache updated using fc-cache");
        return Ok(());
    }
    
    // 尝试 mkfontdir 和 mkfontscale（旧系统）
    if Command::new("mkfontdir").status().is_ok() {
        info!("Font cache updated using mkfontdir");
    }
    
    if Command::new("mkfontscale").status().is_ok() {
        info!("Font cache updated using mkfontscale");
    }
    
    // 尝试 xset（X11 系统）
    if Command::new("xset").args(["fp", "rehash"]).status().is_ok() {
        info!("Font cache updated using xset");
    }
    
    Ok(())
}
