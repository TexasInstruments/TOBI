use std::fs;

use anyhow::{Context, bail};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Catalog {
    pub schema_version: u32,
    pub generated_at: Option<String>,
    #[serde(default)]
    pub devices: Vec<DeviceEntry>,
    pub images: Vec<ImageEntry>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct DeviceEntry {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub compatible: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ImageEntry {
    pub id: String,
    pub name: String,
    pub description: String,
    #[serde(default)]
    pub devices: Vec<String>,
    #[serde(default)]
    pub category: Option<String>,
    #[serde(default)]
    pub recommended: bool,
    pub version: String,
    pub release_date: String,
    pub channel: String,
    pub url: String,
    pub format: ImageFormat,
    #[serde(default)]
    pub image_download_sha256: Option<String>,
    #[serde(default)]
    pub extract_sha256: Option<String>,
    #[serde(default)]
    pub image_download_size: Option<u64>,
    #[serde(default)]
    pub extract_size: Option<u64>,
    #[serde(default)]
    pub decoder_memory_size: Option<u64>,
    #[serde(default)]
    pub bmap_url: Option<String>,
    #[serde(default)]
    pub signature_url: Option<String>,
}

impl ImageEntry {
    pub fn category_label(&self) -> &str {
        self.category
            .as_deref()
            .map(str::trim)
            .filter(|category| !category.is_empty())
            .unwrap_or("Other")
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ImageFormat {
    Raw,
    #[serde(rename = "img.xz")]
    ImgXz,
    #[serde(rename = "wic.xz")]
    WicXz,
    #[serde(rename = "img.zst")]
    ImgZst,
    #[serde(rename = "wic.zst")]
    WicZst,
    #[serde(rename = "img.gz")]
    ImgGz,
    #[serde(rename = "wic.gz")]
    WicGz,
}

impl ImageFormat {
    pub fn label(self) -> &'static str {
        match self {
            Self::Raw => "raw",
            Self::ImgXz => "img.xz",
            Self::WicXz => "wic.xz",
            Self::ImgZst => "img.zst",
            Self::WicZst => "wic.zst",
            Self::ImgGz => "img.gz",
            Self::WicGz => "wic.gz",
        }
    }

    pub fn is_xz(self) -> bool {
        matches!(self, Self::ImgXz | Self::WicXz)
    }

    pub fn is_zstd(self) -> bool {
        matches!(self, Self::ImgZst | Self::WicZst)
    }

    pub fn is_gzip(self) -> bool {
        matches!(self, Self::ImgGz | Self::WicGz)
    }

    pub fn is_compressed(self) -> bool {
        self.is_xz() || self.is_zstd() || self.is_gzip()
    }

    pub fn from_filename(name: &str) -> Option<Self> {
        let name = name.to_ascii_lowercase();
        if name.ends_with(".img.xz") {
            Some(Self::ImgXz)
        } else if name.ends_with(".wic.xz") {
            Some(Self::WicXz)
        } else if name.ends_with(".img.zst") {
            Some(Self::ImgZst)
        } else if name.ends_with(".wic.zst") {
            Some(Self::WicZst)
        } else if name.ends_with(".img.gz") {
            Some(Self::ImgGz)
        } else if name.ends_with(".wic.gz") {
            Some(Self::WicGz)
        } else if name.ends_with(".wic")
            || name.ends_with(".img")
            || name.ends_with(".raw")
            || name.ends_with(".bin")
        {
            Some(Self::Raw)
        } else {
            None
        }
    }
}

#[cfg(test)]
pub fn load_catalog(source: &str) -> anyhow::Result<Catalog> {
    load_catalog_with_proxy(source, None)
}

pub fn load_catalog_with_proxy(source: &str, proxy: Option<&str>) -> anyhow::Result<Catalog> {
    let text = if source.starts_with("http://") || source.starts_with("https://") {
        let mut client = reqwest::blocking::Client::builder();
        if let Some(proxy) = proxy.filter(|proxy| !proxy.trim().is_empty()) {
            client = client.proxy(reqwest::Proxy::all(proxy).context("proxy URL is invalid")?);
        }
        client
            .build()
            .context("failed to build HTTP client")?
            .get(source)
            .send()
            .with_context(|| format!("failed to request {source}"))?
            .error_for_status()
            .with_context(|| format!("manifest request failed for {source}"))?
            .text()
            .with_context(|| format!("failed to read manifest body from {source}"))?
    } else {
        fs::read_to_string(source)
            .with_context(|| format!("failed to read manifest file {source}"))?
    };

    let catalog: Catalog = serde_json::from_str(&text).context("manifest JSON is invalid")?;
    validate_catalog(&catalog)?;
    Ok(catalog)
}

pub fn fallback_catalog() -> Catalog {
    Catalog {
        schema_version: 1,
        generated_at: None,
        devices: supported_devices(),
        images: Vec::new(),
    }
}

fn supported_devices() -> Vec<DeviceEntry> {
    vec![
        DeviceEntry {
            id: "sk-am62p-lp".to_string(),
            name: "SK-AM62P-LP".to_string(),
            compatible: vec![
                "ti,am62p5-sk".to_string(),
                "ti,am62px-sk".to_string(),
                "ti,am62pxx-evm".to_string(),
            ],
        },
        DeviceEntry {
            id: "sk-am62-lp".to_string(),
            name: "SK-AM62-LP".to_string(),
            compatible: vec!["ti,am62-lp-sk".to_string()],
        },
        DeviceEntry {
            id: "sk-am62-sip".to_string(),
            name: "SK-AM62-SIP".to_string(),
            compatible: vec![
                "ti,am6254atl-sk".to_string(),
                "ti,am6254xxl-sk".to_string(),
                "ti,am6254atl".to_string(),
                "ti,am6254xxl".to_string(),
            ],
        },
        DeviceEntry {
            id: "sk-am62b".to_string(),
            name: "SK-AM62B".to_string(),
            compatible: vec!["ti,am625-sk".to_string()],
        },
        DeviceEntry {
            id: "beagleplay".to_string(),
            name: "BeaglePlay".to_string(),
            compatible: vec!["beagle,am625-beagleplay".to_string()],
        },
        DeviceEntry {
            id: "sk-am62a-lp".to_string(),
            name: "SK-AM62A-LP".to_string(),
            compatible: vec!["ti,am62a7-sk".to_string(), "ti,am62a7".to_string()],
        },
        DeviceEntry {
            id: "tmds62levm".to_string(),
            name: "TMDS62LEVM".to_string(),
            compatible: vec!["ti,am62l3-evm".to_string(), "ti,am62l3".to_string()],
        },
        DeviceEntry {
            id: "sk-am64b".to_string(),
            name: "SK-AM64B".to_string(),
            compatible: vec!["ti,am642-sk".to_string(), "ti,am642".to_string()],
        },
        DeviceEntry {
            id: "tmds64evm".to_string(),
            name: "TMDS64EVM".to_string(),
            compatible: vec!["ti,am642-evm".to_string(), "ti,am642".to_string()],
        },
        DeviceEntry {
            id: "sk-am68".to_string(),
            name: "SK-AM68".to_string(),
            compatible: vec!["ti,am68-sk".to_string(), "ti,j721s2".to_string()],
        },
        DeviceEntry {
            id: "sk-am69".to_string(),
            name: "SK-AM69".to_string(),
            compatible: vec!["ti,am69-sk".to_string(), "ti,j784s4".to_string()],
        },
    ]
}

pub fn is_remote_source(source: &str) -> bool {
    source.starts_with("http://") || source.starts_with("https://")
}

pub fn validate_catalog(catalog: &Catalog) -> anyhow::Result<()> {
    if catalog.schema_version != 1 {
        bail!(
            "unsupported manifest schema version {}",
            catalog.schema_version
        );
    }
    if catalog.images.is_empty() {
        bail!("manifest does not contain any images");
    }
    for image in &catalog.images {
        if image.id.trim().is_empty() || image.name.trim().is_empty() {
            bail!("manifest contains an image with an empty id or name");
        }
        if image.url.trim().is_empty() {
            bail!("image {} has an empty URL", image.id);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_sample_catalog() {
        let catalog = load_catalog("sample/catalog.json").expect("sample catalog should parse");
        assert_eq!(catalog.schema_version, 1);
        assert!(
            catalog
                .images
                .iter()
                .any(|image| image.id == "tisdk-default-am62pxx-12.00.00.07.04")
        );
        assert!(catalog.images.len() > 4);
    }
}
