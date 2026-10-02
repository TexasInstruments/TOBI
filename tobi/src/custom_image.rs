use std::collections::VecDeque;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::Context;

use crate::manifest::{ImageEntry, ImageFormat};

const MAX_SCAN_DEPTH: usize = 5;

#[derive(Clone, Debug)]
pub struct CustomImage {
    pub name: String,
    pub path: PathBuf,
    pub size_bytes: Option<u64>,
    pub format: ImageFormat,
}

pub fn scan_custom_images() -> anyhow::Result<Vec<CustomImage>> {
    let mut images = Vec::new();
    for root in scan_roots() {
        if root.is_dir() {
            scan_root(&root, &mut images)
                .with_context(|| format!("failed to scan {}", root.display()))?;
        }
    }
    images.sort_by(|a, b| a.name.cmp(&b.name).then_with(|| a.path.cmp(&b.path)));
    Ok(images)
}

pub fn custom_placeholder() -> ImageEntry {
    ImageEntry {
        id: "custom-attached-media".to_string(),
        name: "Custom image from attached media".to_string(),
        description: "Choose a local OS image from a mounted USB drive, SD card, or other attached storage media."
            .to_string(),
        devices: Vec::new(),
        category: Some("Custom".to_string()),
        recommended: false,
        version: "local".to_string(),
        release_date: "local".to_string(),
        channel: "custom".to_string(),
        url: "custom://attached-media".to_string(),
        format: ImageFormat::Raw,
        image_download_sha256: None,
        extract_sha256: None,
        image_download_size: None,
        extract_size: None,
        decoder_memory_size: None,
        bmap_url: None,
        signature_url: None,
    }
}

pub fn is_custom_placeholder(image: &ImageEntry) -> bool {
    image.url == "custom://attached-media"
}

impl CustomImage {
    pub fn into_image_entry(self) -> ImageEntry {
        let size_bytes = self.size_bytes;
        ImageEntry {
            id: format!("custom-{}", sanitize_id(&self.name)),
            name: self.name,
            description: format!("Custom local image selected from {}.", self.path.display()),
            devices: Vec::new(),
            category: Some("Custom".to_string()),
            recommended: false,
            version: "local".to_string(),
            release_date: "local".to_string(),
            channel: "custom".to_string(),
            url: format!("file://{}", self.path.display()),
            format: self.format,
            image_download_sha256: None,
            extract_sha256: None,
            image_download_size: size_bytes,
            extract_size: None,
            decoder_memory_size: None,
            bmap_url: None,
            signature_url: None,
        }
    }
}

fn scan_roots() -> Vec<PathBuf> {
    if let Ok(roots) = std::env::var("TOBI_CUSTOM_IMAGE_ROOTS") {
        return roots
            .split(':')
            .filter(|root| !root.trim().is_empty())
            .map(PathBuf::from)
            .collect();
    }

    let mut roots = Vec::new();
    #[cfg(target_os = "macos")]
    {
        roots.push(PathBuf::from("/Volumes"));
    }
    #[cfg(target_os = "linux")]
    {
        roots.push(PathBuf::from("/run/media"));
        roots.push(PathBuf::from("/media"));
        roots.push(PathBuf::from("/mnt"));
        roots.push(PathBuf::from("/var/run/media"));
    }
    roots
}

fn scan_root(root: &Path, images: &mut Vec<CustomImage>) -> anyhow::Result<()> {
    let mut queue = VecDeque::from([(root.to_path_buf(), 0_usize)]);
    while let Some((dir, depth)) = queue.pop_front() {
        if depth > MAX_SCAN_DEPTH {
            continue;
        }

        let Ok(entries) = fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries {
            let entry = entry?;
            let path = entry.path();
            let metadata = match entry.metadata() {
                Ok(metadata) => metadata,
                Err(_) => continue,
            };

            if metadata.is_dir() {
                queue.push_back((path, depth + 1));
                continue;
            }

            if !metadata.is_file() {
                continue;
            }

            let Some(file_name) = path.file_name().and_then(|name| name.to_str()) else {
                continue;
            };
            let Some(format) = ImageFormat::from_filename(file_name) else {
                continue;
            };

            images.push(CustomImage {
                name: file_name.to_string(),
                path,
                size_bytes: Some(metadata.len()),
                format,
            });
        }
    }
    Ok(())
}

fn sanitize_id(value: &str) -> String {
    value
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() {
                ch.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect::<String>()
        .split('-')
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join("-")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recognizes_supported_image_extensions() {
        assert_eq!(
            ImageFormat::from_filename("image.wic.xz"),
            Some(ImageFormat::WicXz)
        );
        assert_eq!(
            ImageFormat::from_filename("image.img.zst"),
            Some(ImageFormat::ImgZst)
        );
        assert_eq!(
            ImageFormat::from_filename("image.raw"),
            Some(ImageFormat::Raw)
        );
        assert_eq!(ImageFormat::from_filename("notes.txt"), None);
    }

    #[test]
    fn converts_custom_image_to_file_url() {
        let image = CustomImage {
            name: "demo.wic.xz".to_string(),
            path: "/media/usb/demo.wic.xz".into(),
            size_bytes: Some(42),
            format: ImageFormat::WicXz,
        }
        .into_image_entry();

        assert_eq!(image.name, "demo.wic.xz");
        assert_eq!(image.url, "file:///media/usb/demo.wic.xz");
        assert_eq!(image.image_download_size, Some(42));
    }
}
