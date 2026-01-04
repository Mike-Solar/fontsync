use anyhow::{Context, Result};
use log::error;
use sha2::{Digest, Sha256};
use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

pub fn calculate_sha256(path: &Path) -> Result<String> {
    let mut file = File::open(path)
        .with_context(|| format!("Failed to open file: {:?}", path))?;
    
    let mut hasher = Sha256::new();
    let mut buffer = [0; 8192];
    
    loop {
        let bytes_read = file.read(&mut buffer)
            .with_context(|| format!("Failed to read file: {:?}", path))?;
        
        if bytes_read == 0 {
            break;
        }
        
        hasher.update(&buffer[..bytes_read]);
    }
    
    let result = hasher.finalize();
    Ok(hex::encode(result))
}

pub fn is_font_file(path: &Path) -> bool {
    if let Some(ext) = path.extension() {
        let ext_str = ext.to_string_lossy().to_lowercase();
        matches!(
            ext_str.as_str(),
            "ttf" | "otf" | "woff" | "woff2" | "eot" | "ttc" | "pfa" | "pfb" | "afm" | "pfm"
        )
    } else {
        false
    }
}

pub fn get_font_mime_type(path: &Path) -> String {
    if let Some(ext) = path.extension() {
        match ext.to_string_lossy().to_lowercase().as_str() {
            "ttf" => "font/ttf".to_string(),
            "otf" => "font/otf".to_string(),
            "woff" => "font/woff".to_string(),
            "woff2" => "font/woff2".to_string(),
            "eot" => "application/vnd.ms-fontobject".to_string(),
            "ttc" => "font/collection".to_string(),
            "pfa" | "pfb" => "application/x-font-type1".to_string(),
            "afm" => "application/x-font-afm".to_string(),
            "pfm" => "application/x-font-pfm".to_string(),
            _ => "application/octet-stream".to_string(),
        }
    } else {
        "application/octet-stream".to_string()
    }
}

pub async fn scan_font_directory(dir: &Path) -> Result<Vec<FontInfo>> {
    let mut fonts = Vec::new();
    
    if !dir.exists() {
        return Ok(fonts);
    }
    
    for entry in walkdir::WalkDir::new(dir)
        .follow_links(true)
        .into_iter()
        .filter_map(|e| e.ok())
    {
        let path = entry.path();
        if path.is_file() && is_font_file(path) {
            match scan_single_font(path).await {
                Ok(font_info) => fonts.push(font_info),
                Err(e) => error!("Failed to scan font file {:?}: {}", path, e),
            }
        }
    }
    
    Ok(fonts)
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

#[derive(Debug, Clone)]
pub struct FontInfo {
    pub path: PathBuf,
    pub sha256: String,
    pub size: u64,
    pub modified: std::time::SystemTime,
}

pub fn get_system_font_directories() -> Vec<PathBuf> {
    let mut dirs = Vec::new();

    #[cfg(target_os = "windows")]
    {
        if let Some(font_dir) = dirs::font_dir() {
            dirs.push(font_dir);
        }
        if let Some(local_app_data) = dirs::data_local_dir() {
            dirs.push(local_app_data.join("Microsoft\\Windows\\Fonts"));
        }
    }

    #[cfg(target_os = "linux")]
    {
        if let Some(home_dir) = dirs::home_dir() {
            dirs.push(home_dir.join(".fonts"));
            dirs.push(home_dir.join(".local/share/fonts"));
        }
        dirs.push(PathBuf::from("/usr/share/fonts"));
        dirs.push(PathBuf::from("/usr/local/share/fonts"));
    }

    #[cfg(target_os = "macos")]
    {
        if let Some(home_dir) = dirs::home_dir() {
            dirs.push(home_dir.join("Library/Fonts"));
        }
        dirs.push(PathBuf::from("/System/Library/Fonts"));
        dirs.push(PathBuf::from("/Library/Fonts"));
    }

    // Filter out non-existent directories
    dirs.into_iter().filter(|p| p.exists()).collect()
}

pub fn get_file_timestamp(path: &Path) -> Result<u64> {
    let metadata = std::fs::metadata(path)
        .with_context(|| format!("Failed to get metadata for: {:?}", path))?;
    
    let modified = metadata.modified()
        .with_context(|| format!("Failed to get modified time for: {:?}", path))?;
    
    let duration = modified.duration_since(UNIX_EPOCH)
        .with_context(|| format!("Failed to convert modified time for: {:?}", path))?;
    
    Ok(duration.as_secs())
}

pub fn format_file_size(size: u64) -> String {
    const UNITS: &[&str] = &["B", "KB", "MB", "GB", "TB"];
    let mut size = size as f64;
    let mut unit_index = 0;
    
    while size >= 1024.0 && unit_index < UNITS.len() - 1 {
        size /= 1024.0;
        unit_index += 1;
    }
    
    if unit_index == 0 {
        format!("{} {}", size as u64, UNITS[unit_index])
    } else {
        format!("{:.2} {}", size, UNITS[unit_index])
    }
}

pub fn validate_font_file(path: &Path) -> Result<bool> {
    if !path.exists() {
        return Ok(false);
    }
    
    if !is_font_file(path) {
        return Ok(false);
    }
    
    // Try to read the first few bytes to check if it's a valid font file
    match File::open(path) {
        Ok(mut file) => {
            let mut header = [0u8; 4];
            match file.read_exact(&mut header) {
                Ok(_) => {
                    // Basic validation for common font formats
                    let is_valid = match &header {
                        [0x00, 0x01, 0x00, 0x00] => true, // TTF
                        [0x4F, 0x54, 0x54, 0x4F] => true, // OTF
                        [0x77, 0x4F, 0x46, 0x46] => true, // WOFF
                        [0x77, 0x4F, 0x46, 0x32] => true, // WOFF2
                        [0x74, 0x74, 0x63, 0x66] => true, // TTC
                        _ => false,
                    };
                    Ok(is_valid)
                }
                Err(_) => Ok(false),
            }
        }
        Err(_) => Ok(false),
    }
}

pub fn sanitize_filename(filename: &str) -> String {
    filename
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '.' || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect()
}

#[derive(Debug, Clone, PartialEq)]
pub enum ConflictResolution {
    Overwrite,
    Rename,
    Skip,
}

pub fn prompt_conflict_resolution(
    filename: &str,
    local_sha256: &str,
    remote_sha256: &str,
    interactive: bool,
) -> Result<ConflictResolution> {
    if !interactive {
        error!(
            "Font conflict detected for '{}': local SHA256={}, remote SHA256={}. Skipping due to non-interactive mode.",
            filename, local_sha256, remote_sha256
        );
        return Ok(ConflictResolution::Skip);
    }

    use dialoguer::{theme::ColorfulTheme, Select};
    
    println!("\n⚠️  Font file conflict detected!");
    println!("Filename: {}", filename);
    println!("Local SHA256:  {}...", &local_sha256[..16]);
    println!("Remote SHA256: {}...", &remote_sha256[..16]);
    println!("\nWhat would you like to do?");
    println!("1) Overwrite local file with remote version");
    println!("2) Rename remote file");
    println!("3) Skip this file");
    
    let items = vec!["Overwrite", "Rename", "Skip"];
    let selection = Select::with_theme(&ColorfulTheme::default())
        .items(&items)
        .default(2)
        .interact()?;
    
    match selection {
        0 => Ok(ConflictResolution::Overwrite),
        1 => Ok(ConflictResolution::Rename),
        2 => Ok(ConflictResolution::Skip),
        _ => Ok(ConflictResolution::Skip),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::NamedTempFile;
    use tempfile::tempdir;

    #[test]
    fn test_calculate_sha256() {
        let mut temp_file = NamedTempFile::new().unwrap();
        writeln!(temp_file, "Hello, world!").unwrap();
        
        let result = calculate_sha256(temp_file.path()).unwrap();
        assert!(!result.is_empty());
        assert_eq!(result.len(), 64); // SHA256 hex string is 64 characters
    }

    #[test]
    fn test_is_font_file() {
        assert!(is_font_file(Path::new("test.ttf")));
        assert!(is_font_file(Path::new("test.otf")));
        assert!(is_font_file(Path::new("test.woff")));
        assert!(is_font_file(Path::new("test.woff2")));
        assert!(!is_font_file(Path::new("test.txt")));
        assert!(!is_font_file(Path::new("test")));
    }

    #[test]
    fn test_get_font_mime_type() {
        assert_eq!(get_font_mime_type(Path::new("test.ttf")), "font/ttf");
        assert_eq!(get_font_mime_type(Path::new("test.otf")), "font/otf");
        assert_eq!(get_font_mime_type(Path::new("test.woff")), "font/woff");
        assert_eq!(get_font_mime_type(Path::new("test.woff2")), "font/woff2");
        assert_eq!(get_font_mime_type(Path::new("test.txt")), "application/octet-stream");
    }

    #[test]
    fn test_format_file_size() {
        assert_eq!(format_file_size(0), "0 B");
        assert_eq!(format_file_size(1023), "1023 B");
        assert_eq!(format_file_size(1024), "1.00 KB");
        assert_eq!(format_file_size(1536), "1.50 KB");
        assert_eq!(format_file_size(1024 * 1024), "1.00 MB");
    }

    #[test]
    fn test_sanitize_filename() {
        let sanitized = sanitize_filename("My Font (v1).ttf");
        assert_eq!(sanitized, "My_Font__v1_.ttf");
    }

    #[test]
    fn test_validate_font_file() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("sample.ttf");
        let mut file = File::create(&path).unwrap();
        file.write_all(&[0x00, 0x01, 0x00, 0x00]).unwrap();
        assert!(validate_font_file(&path).unwrap());
    }

    #[test]
    fn test_get_file_timestamp() {
        let mut temp_file = NamedTempFile::new().unwrap();
        writeln!(temp_file, "timestamp").unwrap();
        let timestamp = get_file_timestamp(temp_file.path()).unwrap();
        assert!(timestamp > 0);
    }
}
