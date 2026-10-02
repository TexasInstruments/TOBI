use anyhow::bail;

use crate::manifest::{ImageEntry, ImageFormat};

const MIB: u64 = 1024 * 1024;

#[derive(Clone, Debug)]
pub struct MemoryCheck {
    pub available_bytes: Option<u64>,
    pub required_bytes: u64,
    pub enough: Option<bool>,
}

pub fn check_image_memory(image: &ImageEntry) -> MemoryCheck {
    let required_bytes = required_working_memory(image);
    let available_bytes = available_memory_bytes();
    let enough = available_bytes.map(|available| available >= required_bytes);

    MemoryCheck {
        available_bytes,
        required_bytes,
        enough,
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

fn required_working_memory(image: &ImageEntry) -> u64 {
    let base = 96 * MIB;
    let decoder = decoder_working_memory(image);
    base + decoder
}

fn decoder_working_memory(image: &ImageEntry) -> u64 {
    if let Some(measured_decoder_memory) = image.decoder_memory_size {
        return measured_decoder_memory
            .saturating_add(32 * MIB)
            .max(measured_decoder_memory_floor(image.format));
    }

    decoder_memory_floor(image.format)
}

fn measured_decoder_memory_floor(format: ImageFormat) -> u64 {
    match format {
        ImageFormat::Raw => 32 * MIB,
        ImageFormat::ImgGz | ImageFormat::WicGz => 64 * MIB,
        ImageFormat::ImgZst | ImageFormat::WicZst => 96 * MIB,
        ImageFormat::ImgXz | ImageFormat::WicXz => 32 * MIB,
    }
}

fn decoder_memory_floor(format: ImageFormat) -> u64 {
    match format {
        ImageFormat::Raw => 32 * MIB,
        ImageFormat::ImgGz | ImageFormat::WicGz => 64 * MIB,
        ImageFormat::ImgZst | ImageFormat::WicZst => 160 * MIB,
        ImageFormat::ImgXz | ImageFormat::WicXz => 256 * MIB,
    }
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
        self.enough == Some(false)
    }

    pub fn summary(&self) -> String {
        let required = format_bytes(self.required_bytes);
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

    #[test]
    fn xz_images_require_working_ram_but_not_full_image_size() {
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
    }

    #[test]
    fn measured_decoder_memory_reduces_xz_budget_with_margin() {
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
    }
}
