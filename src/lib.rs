use enshwick::{Load, Mod, Off};
mod hooks;
mod journal;
#[cfg(feature = "runtime-test")]
mod mip_probe;
mod native;
mod ore_materials;
mod ores;
mod runtime;
#[cfg(feature = "runtime-test")]
mod selftest;
mod settings;
mod terrain_editor;

struct TerrainPersistence;
impl Mod for TerrainPersistence {
    fn init(load: &mut Load<'_>) -> Result<Self, Off> {
        let enabled = !cfg!(feature = "runtime-test")
            || std::env::var("TERRAIN_TEST_PROBE_ONLY").as_deref() != Ok("1");
        if enabled {
            terrain_editor::install(load)?;
            hooks::install(load)?;
            runtime::install(load)?;
        }
        #[cfg(feature = "runtime-test")]
        selftest::install(load)?;
        enshwick::info!(
            "Terrain persistence development build loaded: capture/replay enabled={enabled}"
        );
        Ok(Self)
    }
}
enshwick::declare_mod!(TerrainPersistence);
