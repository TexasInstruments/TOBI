use anyhow::bail;
use std::sync::atomic::{AtomicBool, Ordering};

use crate::manifest::{ImageEntry, ImageFormat};

const MIB: u64 = 1024 * 1024;
static LITE_XZ_MEMORY_GUARD: AtomicBool = AtomicBool::new(false);

#[derive(Clone, Debug)]
pub struct MemoryCheck {
    pub available_bytes: Option<u64>,
    pub required_bytes: u64,
    pub enough: Option<bool>,
    pub guard_relaxed: bool,
}

pub fn check_image_memory(image: &ImageEntry) -> MemoryCheck {
    let guard_relaxed = lite_xz_memory_guard() && image.format.is_xz();
    let required_bytes = required_working_memory(image, guard_relaxed);
    let available_bytes = available_memory_bytes();
    let enough = available_bytes.map(|available| available >= required_bytes);

    MemoryCheck {
        available_bytes,
        required_bytes,
        enough,
        guard_relaxed,
    }
}

pub fn ensure_image_memory(image: &ImageEntry) -> anyhow::Result<()> {
    let check = check_image_memory(image);
    if check.blocks_install() {
        let available = check.available_bytes.unwrap_or_default();
        bail!(
            "not enough available RAM for the installer working set: need at least {}, have {}",
            format_bytes(check.required_bytes),
            format_bytes(available)
        );
    }
    Ok(())
}

pub fn set_lite_xz_memory_guard(enabled: bool) {
    LITE_XZ_MEMORY_GUARD.store(enabled, Ordering::Relaxed);
}

fn required_working_memory(image: &ImageEntry, guard_relaxed: bool) -> u64 {
    let base = 96 * MIB;
    let decoder = decoder_working_memory(image, guard_relaxed);
    base + decoder
}

fn decoder_working_memory(image: &ImageEntry, guard_relaxed: bool) -> u64 {
    if let Some(measured_decoder_memory) = image.decoder_memory_size {
        return measured_decoder_memory
            .saturating_add(32 * MIB)
            .max(measured_decoder_memory_floor(image.format));
    }

    decoder_memory_floor(image.format, guard_relaxed)
}

fn measured_decoder_memory_floor(format: ImageFormat) -> u64 {
    match format {
        ImageFormat::Raw => 32 * MIB,
        ImageFormat::ImgGz | ImageFormat::WicGz => 64 * MIB,
        ImageFormat::ImgZst | ImageFormat::WicZst => 96 * MIB,
        ImageFormat::ImgXz | ImageFormat::WicXz => 32 * MIB,
    }
}

fn decoder_memory_floor(format: ImageFormat, guard_relaxed: bool) -> u64 {
    match format {
        ImageFormat::Raw => 32 * MIB,
        ImageFormat::ImgGz | ImageFormat::WicGz => 64 * MIB,
        ImageFormat::ImgZst | ImageFormat::WicZst => 160 * MIB,
        ImageFormat::ImgXz | ImageFormat::WicXz if guard_relaxed => 64 * MIB,
        ImageFormat::ImgXz | ImageFormat::WicXz => 256 * MIB,
    }
}

fn lite_xz_memory_guard() -> bool {
    LITE_XZ_MEMORY_GUARD.load(Ordering::Relaxed)
        || std::env::var("TOBI_LITE")
            .map(|value| matches!(value.as_str(), "1" | "true" | "TRUE" | "yes" | "YES"))
            .unwrap_or(false)
}

fn available_memory_bytes() -> Option<u64> {
    #[cfg(target_os = "linux")]
    {
        linux_mem_available()
    }

    #[cfg(not(target_os = "linux"))]
    {
        None
    }
}

#[cfg(target_os = "linux")]
fn linux_mem_available() -> Option<u64> {
    let meminfo = std::fs::read_to_string("/proc/meminfo").ok()?;
    for line in meminfo.lines() {
        let Some(rest) = line.strip_prefix("MemAvailable:") else {
            continue;
        };
        let kb = rest
            .split_whitespace()
            .next()
            .and_then(|value| value.parse::<u64>().ok())?;
        return Some(kb * 1024);
    }
    None
}

fn format_bytes(bytes: u64) -> String {
    if bytes >= 1024 * MIB {
        format!("{:.1} GiB", bytes as f64 / (1024.0 * MIB as f64))
    } else {
        format!("{:.0} MiB", bytes as f64 / MIB as f64)
    }
}

impl MemoryCheck {
    pub fn blocks_install(&self) -> bool {
        self.enough == Some(false) && !self.guard_relaxed
    }

    pub fn summary(&self) -> String {
        let required = format_bytes(self.required_bytes);
        if self.guard_relaxed {
            return match self.available_bytes {
                Some(available) => format!(
                    "TOBI-lite test: {} available, xz guard relaxed to {} working RAM and not enforced",
                    format_bytes(available),
                    required
                ),
                None => format!(
                    "TOBI-lite test: xz guard relaxed to {required} working RAM and not enforced"
                ),
            };
        }
        match (self.available_bytes, self.enough) {
            (Some(available), Some(true)) => {
                format!(
                    "OK: {} available, {} working RAM needed",
                    format_bytes(available),
                    required
                )
            }
            (Some(available), Some(false)) => {
                format!(
                    "Insufficient: {} available, {} working RAM needed",
                    format_bytes(available),
                    required
                )
            }
            _ => format!("{required} working RAM needed; available RAM unknown"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::manifest::ImageEntry;
    use std::sync::Mutex;

    static MEMORY_GUARD_TEST_LOCK: Mutex<()> = Mutex::new(());

    #[test]
    fn xz_images_require_working_ram_but_not_full_image_size() {
        let _guard = MEMORY_GUARD_TEST_LOCK.lock().unwrap();
        set_lite_xz_memory_guard(false);
        let image = ImageEntry {
            id: "large".to_string(),
            name: "large".to_string(),
            description: "large".to_string(),
            devices: Vec::new(),
            category: Some("Test".to_string()),
            recommended: false,
            version: "local".to_string(),
            release_date: "local".to_string(),
            channel: "test".to_string(),
            url: "file:///image.wic.xz".to_string(),
            format: ImageFormat::WicXz,
            image_download_sha256: None,
            extract_sha256: None,
            image_download_size: Some(16 * 1024 * MIB),
            extract_size: Some(64 * 1024 * MIB),
            decoder_memory_size: None,
            bmap_url: None,
            signature_url: None,
        };

        let check = check_image_memory(&image);
        assert!(check.required_bytes < image.image_download_size.unwrap());
        assert!(!check.guard_relaxed);
    }

    #[test]
    fn lite_mode_relaxes_xz_guard_for_hardware_testing() {
        let _guard = MEMORY_GUARD_TEST_LOCK.lock().unwrap();
        set_lite_xz_memory_guard(true);
        let image = ImageEntry {
            id: "large".to_string(),
            name: "large".to_string(),
            description: "large".to_string(),
            devices: Vec::new(),
            category: Some("Test".to_string()),
            recommended: false,
            version: "local".to_string(),
            release_date: "local".to_string(),
            channel: "test".to_string(),
            url: "file:///image.wic.xz".to_string(),
            format: ImageFormat::WicXz,
            image_download_sha256: None,
            extract_sha256: None,
            image_download_size: Some(16 * 1024 * MIB),
            extract_size: Some(64 * 1024 * MIB),
            decoder_memory_size: None,
            bmap_url: None,
            signature_url: None,
        };

        let check = check_image_memory(&image);
        assert_eq!(check.required_bytes, 160 * MIB);
        assert!(check.guard_relaxed);
        assert!(!check.blocks_install());
        set_lite_xz_memory_guard(false);
    }

    #[test]
    fn measured_decoder_memory_reduces_xz_budget_with_margin() {
        let _guard = MEMORY_GUARD_TEST_LOCK.lock().unwrap();
        set_lite_xz_memory_guard(false);
        let image = ImageEntry {
            id: "measured".to_string(),
            name: "measured".to_string(),
            description: "measured".to_string(),
            devices: Vec::new(),
            category: Some("Test".to_string()),
            recommended: false,
            version: "local".to_string(),
            release_date: "local".to_string(),
            channel: "test".to_string(),
            url: "file:///image.wic.xz".to_string(),
            format: ImageFormat::WicXz,
            image_download_sha256: None,
            extract_sha256: None,
            image_download_size: Some(1024 * MIB),
            extract_size: Some(16 * 1024 * MIB),
            decoder_memory_size: Some(8 * MIB),
            bmap_url: None,
            signature_url: None,
        };

        let check = check_image_memory(&image);
        assert_eq!(check.required_bytes, 136 * MIB);
        assert!(!check.guard_relaxed);
    }
}
