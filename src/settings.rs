/// Terrain persistence and ore regeneration settings.
#[enshwick::config]
#[derive(Clone, Debug)]
pub struct Settings {
    /// Regenerate mined ore and clay voxels after the delay. Loot boxes are unchanged.
    #[config(restart)]
    pub respawn_ores: bool,
    /// Respawn delay in seconds. Nearby mining restarts the local countdown.
    /// Timers survive restarts, and time spent offline counts toward respawning.
    #[config(restart, range = 1..=31536000)]
    pub ore_respawn_seconds: u64,
    /// Optional extra resource material IDs. Leave empty for default ores and clay.
    /// Available IDs are listed in mod-data/kulcr.terrain-persistence/ore-materials.txt.
    #[config(restart, range = 1..=127)]
    pub extra_ore_materials: Vec<u8>,
}
impl Default for Settings {
    fn default() -> Self {
        Self {
            respawn_ores: true,
            ore_respawn_seconds: 1800,
            extra_ore_materials: Vec::new(),
        }
    }
}
