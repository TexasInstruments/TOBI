use std::sync::LazyLock;

use serde::Deserialize;

/// The application and Pages builder consume the same board instructions.
#[derive(Debug, Deserialize)]
pub struct BootGuide {
    pub board_id: String,
    pub name: String,
    pub summary: String,
    pub steps: Vec<String>,
    pub url: String,
}

#[derive(Deserialize)]
struct GuideCatalog {
    boards: Vec<BootGuide>,
}

static GUIDES: LazyLock<GuideCatalog> = LazyLock::new(|| {
    serde_json::from_str(include_str!("../../docs/boot-guide.json"))
        .expect("embedded board boot guide must be valid JSON")
});

pub fn for_board(board_id: &str) -> Option<&'static BootGuide> {
    GUIDES
        .boards
        .iter()
        .find(|guide| guide.board_id == board_id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_catalog_board_has_one_matching_guide() {
        let catalog: crate::manifest::Catalog =
            serde_json::from_str(include_str!("../../catalog.json")).unwrap();
        assert_eq!(GUIDES.boards.len(), catalog.devices.len());
        for device in catalog.devices {
            let matching = GUIDES
                .boards
                .iter()
                .filter(|guide| guide.board_id == device.id)
                .collect::<Vec<_>>();
            assert_eq!(matching.len(), 1, "{}", device.id);
            let guide = matching[0];
            assert_eq!(guide.name, device.name);
            assert!(!guide.summary.is_empty());
            assert!(!guide.steps.is_empty());
            assert_eq!(
                guide.url,
                format!(
                    "https://texasinstruments.github.io/TOBI/boards/{}/",
                    device.id
                )
            );
        }
    }

    #[test]
    fn unknown_board_never_uses_another_boards_switches() {
        assert!(for_board("unknown").is_none());
    }
}
