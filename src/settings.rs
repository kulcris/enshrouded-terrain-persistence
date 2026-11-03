/// Terrain persistence and ore regeneration settings.
#[enshwick::config]
#[derive(Clone, Debug)]
pub struct Settings {
    /// Regenerate mined ore and clay voxels after the delay. Loot boxes are unchanged.
    #[config(restart)]
    pub respawn_ores: bool,
    /// Seconds after the most recent mining in a 4-metre voxel brick.
    /// Deadlines are saved and continue across restarts, including offline time.
    #[config(restart, range = 1..=31536000)]
    pub ore_respawn_seconds: u64,
    /// Extra ore voxel material IDs, in addition to ores identified from game items.
    /// See ore-materials.txt in this mod's data folder for the running game's IDs.
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
