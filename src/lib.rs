use enshwick::{Load, Mod, Off};
mod hooks;
mod journal;
mod native;
mod ore_materials;
mod ores;
mod runtime;
mod settings;
mod terrain_editor;

struct TerrainPersistence;
impl Mod for TerrainPersistence {
    fn init(load: &mut Load<'_>) -> Result<Self, Off> {
        terrain_editor::install(load)?;
        hooks::install(load)?;
        runtime::install(load)?;
        enshwick::info!("Terrain persistence loaded: capture/replay enabled");
        Ok(Self)
    }
}
enshwick::declare_mod!(TerrainPersistence);
