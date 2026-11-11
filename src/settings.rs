/// Terrain persistence and world loot regeneration settings.
#[enshwick::config]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Settings {
    /// Regenerate mined ore and clay voxels after the delay.
    pub respawn_ores: bool,
    /// Respawn delay in seconds. Nearby mining restarts the local countdown.
    /// Timers survive restarts, and time spent offline counts toward respawning.
    #[config(range = 1..=31536000)]
    pub ore_respawn_seconds: u64,
    /// Optional extra resource material IDs. Leave empty for default ores and clay.
    /// Available IDs are listed in ore-materials.txt in this world's mod-data folder.
    #[config(range = 1..=127)]
    pub extra_ore_materials: Vec<u8>,
    /// Refill empty world loot chests. Player-built storage is excluded.
    pub respawn_chests: bool,
    /// Seconds after a world chest is observed empty before attempting a refill.
    #[config(range = 1..=31536000)]
    pub chest_respawn_seconds: u64,
    /// Respawn world pickups (including bandages) and supported harvested nodes outside claims.
    pub respawn_lootables: bool,
    /// Seconds after collecting a world pickup or harvesting a supported node.
    #[config(range = 1..=31536000)]
    pub lootable_respawn_seconds: u64,
}

/// Import the old global settings once. Keep explicitly configured new-world
/// values and retain the old file for rollback.
pub fn upgrade_legacy(data: &std::path::Path, current: &mut Settings) -> std::io::Result<()> {
    let old = std::env::current_dir()?.join("mod-configs/kulcr.terrain-persistence.toml");
    upgrade_from(data, current, &old)
}
fn upgrade_from(
    data: &std::path::Path,
    current: &mut Settings,
    old: &std::path::Path,
) -> std::io::Result<()> {
    use enshwick::config::Config;
    let marker = data.join(".legacy-settings-imported");
    if marker.exists() {
        return Ok(());
    }
    if old.is_file() && *current == Settings::default() {
        let text = std::fs::read_to_string(&old)?;
        let settings: Settings = toml::from_str(&text).map_err(std::io::Error::other)?;
        settings.validate().map_err(std::io::Error::other)?;
        let world_data = data
            .parent()
            .ok_or_else(|| std::io::Error::other("Missing world data parent"))?;
        let stem = world_data
            .file_name()
            .and_then(|v| v.to_str())
            .and_then(|v| v.strip_suffix("-mod-data"))
            .ok_or_else(|| std::io::Error::other("Unrecognized world data folder"))?;
        let folder = world_data
            .parent()
            .unwrap()
            .join(format!("{stem}-mod-settings/kulcr.terrain-persistence"));
        std::fs::create_dir_all(&folder)?;
        let temp = folder.join("settings.import.tmp");
        std::fs::write(&temp, &text)?;
        std::fs::rename(temp, folder.join("settings.toml"))?;
        *current = settings;
        enshwick::info!("Imported legacy settings from {}", old.display());
    }
    std::fs::create_dir_all(data)?;
    std::fs::write(marker, b"Legacy settings checked\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn imports_legacy_settings_once_and_keeps_new_world_customizations() {
        let root = std::env::temp_dir().join(format!(
            "terrain-settings-{}-{}",
            std::process::id(),
            crate::ores::now()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let old = root.join("old.toml");
        let text = "respawn_ores = false\nore_respawn_seconds = 90\nextra_ore_materials = [25]\n";
        std::fs::write(&old, text).unwrap();
        let data = root.join("world0-mod-data/kulcr.terrain-persistence");
        let mut current = Settings::default();
        upgrade_from(&data, &mut current, &old).unwrap();
        assert!(!current.respawn_ores);
        assert_eq!(current.ore_respawn_seconds, 90);
        assert_eq!(std::fs::read_to_string(&old).unwrap(), text);
        let imported = root.join("world0-mod-settings/kulcr.terrain-persistence/settings.toml");
        assert_eq!(
            toml::from_str::<Settings>(&std::fs::read_to_string(imported).unwrap()).unwrap(),
            current
        );
        current.ore_respawn_seconds = 45;
        upgrade_from(&data, &mut current, &old).unwrap();
        assert_eq!(current.ore_respawn_seconds, 45);
        let other = root.join("world1-mod-data/kulcr.terrain-persistence");
        upgrade_from(&other, &mut current, &old).unwrap();
        assert_eq!(current.ore_respawn_seconds, 45);
        assert!(
            !root
                .join("world1-mod-settings/kulcr.terrain-persistence/settings.toml")
                .exists()
        );
        std::fs::remove_dir_all(root).unwrap();
    }
}
impl Default for Settings {
    fn default() -> Self {
        Self {
            respawn_ores: true,
            ore_respawn_seconds: 1800,
            extra_ore_materials: Vec::new(),
            respawn_chests: false,
            chest_respawn_seconds: 1800,
            respawn_lootables: false,
            lootable_respawn_seconds: 1800,
        }
    }
}
