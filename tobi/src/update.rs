use crate::manifest::{Catalog, ImageEntry};

/// Return a newer stable TOBI release with complete integrity metadata.
///
/// Unknown boards, incomplete release metadata, and conflicting entries for the
/// newest version do not produce an automatic update offer.
pub fn available_update<'a>(
    catalog: &'a Catalog,
    board_id: Option<&str>,
) -> Option<&'a ImageEntry> {
    select_update(catalog, board_id, env!("CARGO_PKG_VERSION"))
}

fn select_update<'a>(
    catalog: &'a Catalog,
    board_id: Option<&str>,
    current_version: &str,
) -> Option<&'a ImageEntry> {
    let board_id = board_id.filter(|id| !id.is_empty())?;
    if catalog.schema_version != 1 || !catalog.devices.iter().any(|device| device.id == board_id) {
        return None;
    }
    let current = ReleaseVersion::parse(current_version)?;
    let mut newest = None;
    let mut duplicate = false;

    for image in &catalog.images {
        if image.category_label() != "TOBI"
            || image.channel != "stable"
            || !image.devices.iter().any(|device| device == board_id)
        {
            continue;
        }
        let Some(version) = ReleaseVersion::parse(&image.version) else {
            continue;
        };
        if version <= current {
            continue;
        }
        match newest {
            Some((best, _)) if version < best => {}
            Some((best, _)) if version == best => duplicate = true,
            _ => {
                newest = Some((version, image));
                duplicate = false;
            }
        }
    }

    let (_, image) = newest?;
    (!duplicate && has_update_integrity(image)).then_some(image)
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct ReleaseVersion([u64; 3]);

impl ReleaseVersion {
    fn parse(value: &str) -> Option<Self> {
        let mut components = value.split('.');
        let mut numbers = [0; 3];
        for number in &mut numbers {
            let component = components.next()?;
            if component.is_empty()
                || !component.bytes().all(|byte| byte.is_ascii_digit())
                || (component.len() > 1 && component.starts_with('0'))
            {
                return None;
            }
            *number = component.parse().ok()?;
        }
        components.next().is_none().then_some(Self(numbers))
    }
}

fn has_update_integrity(image: &ImageEntry) -> bool {
    let valid_hash = |hash: Option<&str>| {
        hash.is_some_and(|hash| {
            hash.len() == 64 && hash.bytes().all(|byte| byte.is_ascii_hexdigit())
        })
    };
    valid_hash(image.image_download_sha256.as_deref())
        && valid_hash(image.extract_sha256.as_deref())
        && image.image_download_size.is_some_and(|size| size > 0)
        && image.extract_size.is_some_and(|size| size > 0)
        && reqwest::Url::parse(&image.url).is_ok_and(|url| {
            url.scheme() == "https"
                && url.host_str().is_some()
                && url.username().is_empty()
                && url.password().is_none()
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::manifest::{DeviceEntry, ImageFormat};

    const BOARD: &str = "sk-am64b";

    fn image(version: &str) -> ImageEntry {
        ImageEntry {
            id: format!("tobi-{BOARD}-{version}"),
            name: format!("TOBI {version}"),
            description: "TOBI recovery image".to_string(),
            devices: vec![BOARD.to_string()],
            category: Some("TOBI".to_string()),
            recommended: false,
            version: version.to_string(),
            release_date: "2026-10-02".to_string(),
            channel: "stable".to_string(),
            url: format!("https://example.com/TOBI-{version}.img.xz"),
            format: ImageFormat::ImgXz,
            image_download_sha256: Some("a".repeat(64)),
            extract_sha256: Some("b".repeat(64)),
            image_download_size: Some(35_000_000),
            extract_size: Some(222_507_008),
            decoder_memory_size: Some(8_454_776),
            bmap_url: None,
            signature_url: None,
        }
    }

    fn catalog(images: Vec<ImageEntry>) -> Catalog {
        Catalog {
            schema_version: 1,
            generated_at: None,
            devices: vec![DeviceEntry {
                id: BOARD.to_string(),
                name: "SK-AM64B".to_string(),
                compatible: vec!["ti,am642-sk".to_string()],
            }],
            images,
        }
    }

    #[test]
    fn selects_newest_numeric_version_regardless_of_catalog_order() {
        let mut catalog = catalog(vec![
            image("2026.10.2"),
            image("2026.9.30"),
            image("0.5.0"),
            image("2026.10.1"),
        ]);
        for _ in 0..catalog.images.len() {
            assert_eq!(
                select_update(&catalog, Some(BOARD), "0.4.0")
                    .expect("newer release")
                    .version,
                "2026.10.2"
            );
            catalog.images.rotate_left(1);
        }
    }

    #[test]
    fn equal_and_older_versions_are_not_offered() {
        let catalog = catalog(vec![image("2026.10.2"), image("0.4.0")]);
        assert!(select_update(&catalog, Some(BOARD), "2026.10.2").is_none());
        assert!(select_update(&catalog, Some(BOARD), "2026.10.3").is_none());
    }

    #[test]
    fn public_entry_point_compares_against_running_package_version() {
        let mut catalog = catalog(vec![image(env!("CARGO_PKG_VERSION"))]);
        assert!(available_update(&catalog, Some(BOARD)).is_none());
        let current = ReleaseVersion::parse(env!("CARGO_PKG_VERSION")).expect("release version");
        catalog.images[0].version = format!(
            "{}.{}.{}",
            current.0[0],
            current.0[1],
            current.0[2].checked_add(1).expect("next release version")
        );
        assert!(available_update(&catalog, Some(BOARD)).is_some());
    }

    #[test]
    fn only_stable_tobi_images_for_exact_known_board_are_eligible() {
        let mut wrong_category = image("2030.1.1");
        wrong_category.category = Some("TI SDK".to_string());
        let mut wrong_board = image("2031.1.1");
        wrong_board.devices = vec!["tmds64evm".to_string()];
        let mut generic = image("2032.1.1");
        generic.devices.clear();
        let mut prerelease = image("2033.1.1");
        prerelease.channel = "testing".to_string();
        let catalog = catalog(vec![
            wrong_category,
            wrong_board,
            generic,
            prerelease,
            image("2026.10.2"),
        ]);
        assert_eq!(
            select_update(&catalog, Some(BOARD), "0.4.0")
                .expect("matching release")
                .version,
            "2026.10.2"
        );
        assert!(select_update(&catalog, None, "0.4.0").is_none());
        assert!(select_update(&catalog, Some(""), "0.4.0").is_none());
        assert!(select_update(&catalog, Some("tmds64evm"), "0.4.0").is_none());
        assert!(select_update(&catalog, Some("unknown"), "0.4.0").is_none());
    }

    #[test]
    fn malformed_or_ambiguous_versions_are_excluded() {
        let invalid = [
            "",
            "2027",
            "2027.1",
            "2027.1.1.0",
            "v2027.1.1",
            "2027.1.1-rc1",
            "2027.1.1+build1",
            "2027.01.1",
            "2027.1.01",
            "+2027.1.1",
            "2027.1.1 ",
            " 2027.1.1",
            "２０２７.1.1",
            "18446744073709551616.1.1",
        ];
        for version in invalid {
            assert!(ReleaseVersion::parse(version).is_none(), "{version:?}");
            assert!(
                select_update(&catalog(vec![image(version)]), Some(BOARD), "0.4.0").is_none(),
                "{version:?}"
            );
            assert!(
                select_update(&catalog(vec![image("2026.10.2")]), Some(BOARD), version).is_none(),
                "ambiguous running version {version:?}"
            );
        }
    }

    #[test]
    fn duplicate_newest_releases_do_not_choose_by_catalog_order() {
        let mut second = image("2026.10.2");
        second.url = "https://example.com/different.img.xz".to_string();
        let mut catalog = catalog(vec![image("2026.10.1"), image("2026.10.2"), second]);
        for _ in 0..catalog.images.len() {
            assert!(select_update(&catalog, Some(BOARD), "0.4.0").is_none());
            catalog.images.rotate_left(1);
        }
        catalog.images.push(image("2026.10.3"));
        assert_eq!(
            select_update(&catalog, Some(BOARD), "0.4.0")
                .expect("unambiguous newer release")
                .version,
            "2026.10.3"
        );
    }

    #[test]
    fn newest_release_requires_complete_valid_integrity_metadata() {
        let changes: &[fn(&mut ImageEntry)] = &[
            |image| image.image_download_sha256 = None,
            |image| image.extract_sha256 = None,
            |image| image.image_download_sha256 = Some("a".repeat(63)),
            |image| image.extract_sha256 = Some("g".repeat(64)),
            |image| image.image_download_size = None,
            |image| image.extract_size = None,
            |image| image.image_download_size = Some(0),
            |image| image.extract_size = Some(0),
        ];
        for change in changes {
            let mut newest = image("2026.10.3");
            change(&mut newest);
            assert!(
                select_update(
                    &catalog(vec![image("2026.10.2"), newest]),
                    Some(BOARD),
                    "0.4.0"
                )
                .is_none(),
                "incomplete newest release must not fall back to an older image"
            );
        }
        let mut valid = image("2026.10.3");
        valid.extract_sha256 = Some("B".repeat(64));
        assert!(select_update(&catalog(vec![valid]), Some(BOARD), "0.4.0").is_some());
    }

    #[test]
    fn update_download_must_have_a_valid_https_url_without_credentials() {
        for url in [
            "http://example.com/tobi.img.xz",
            "file:///media/tobi.img.xz",
            "/media/tobi.img.xz",
            "https://",
            "https://user:password@example.com/tobi.img.xz",
        ] {
            let mut candidate = image("2026.10.2");
            candidate.url = url.to_string();
            assert!(
                select_update(&catalog(vec![candidate]), Some(BOARD), "0.4.0").is_none(),
                "{url}"
            );
        }
    }

    #[test]
    fn unsupported_catalog_schema_does_not_offer_an_update() {
        let mut catalog = catalog(vec![image("2026.10.2")]);
        catalog.schema_version = 2;
        assert!(select_update(&catalog, Some(BOARD), "0.4.0").is_none());
    }
}
