//! Runtime mapping from terrain voxel materials to the game's item names.

use std::collections::BTreeMap;

use enshwick::terrain::Terrain;
use enshwick::{
    Load, Off,
    content::{
        Blob, Guid,
        items::{ITEM_REGISTRY, ItemLayout},
        types,
    },
};

pub fn is_ore_item(name: &str) -> bool {
    name.split('_')
        .any(|token| token.eq_ignore_ascii_case("ore") || token.eq_ignore_ascii_case("clay"))
}

#[cfg(test)]
mod tests {
    #[test]
    fn ore_and_clay_do_not_match_forest_terrain() {
        for name in [
            "Material_T2_Ore_Copper",
            "Material_T6_Gold_Ore",
            "Material_T2_Clay",
        ] {
            assert!(super::is_ore_item(name));
        }
        for name in [
            "Build_TerrainMaterial_T2_Stone_DeepForest",
            "Material_T1_Fognest_Ressource",
            "Build_TerrainMaterial_T1_Stone",
        ] {
            assert!(!super::is_ore_item(name));
        }
    }
}

/// Maps the game's numeric item ids to names from its item registry.
pub struct Catalog {
    names: BTreeMap<u32, String>,
}

impl Catalog {
    /// Reads the live item registry and each item's numeric id from ItemInfo.
    pub fn resolve(load: &Load<'_>) -> Result<Self, Off> {
        let layout = ItemLayout::resolve(load)?;
        let registry_key = ITEM_REGISTRY.key(types::ITEM_REGISTRY_RESOURCE)?;
        let registry = load
            .host()
            .game_resource(registry_key)
            .ok_or_else(|| Off::new("could not read the game item registry"))?;
        let blob = Blob::new(&registry);
        let (refs_start, refs_count) = blob
            .array(layout.refs)
            .ok_or_else(|| Off::new("item registry references are unreadable"))?;
        let (names_start, _) = blob
            .array(layout.names)
            .filter(|(_, count)| *count == refs_count)
            .ok_or_else(|| Off::new("item registry debug names are unreadable"))?;
        let item_type = types::ITEM_INFO.hash()?;
        let host = load.host();
        let mut names = BTreeMap::new();

        for index in 0..usize::try_from(refs_count).unwrap_or(0) {
            let ref_at = refs_start
                .checked_add(
                    index
                        .checked_mul(16)
                        .ok_or_else(|| Off::new("item registry is too large"))?,
                )
                .ok_or_else(|| Off::new("item registry is too large"))?;
            let Some(guid_bytes) = registry.get(ref_at..ref_at + 16) else {
                continue;
            };
            let Ok(guid_bytes) = <[u8; 16]>::try_from(guid_bytes) else {
                continue;
            };
            let guid = Guid(guid_bytes);

            let name_at = names_start
                .checked_add(
                    index
                        .checked_mul(8)
                        .ok_or_else(|| Off::new("item registry is too large"))?,
                )
                .ok_or_else(|| Off::new("item registry is too large"))?;
            let Some((text_at, text_len)) = blob.array(name_at) else {
                continue;
            };
            let Ok(text_len) = usize::try_from(text_len) else {
                continue;
            };
            let Some(name) = registry
                .get(text_at..text_at.saturating_add(text_len))
                .and_then(|text| std::str::from_utf8(text).ok())
            else {
                continue;
            };

            let key = guid.key_by_hash(item_type);
            let Some(item) = host.game_resource(key) else {
                continue;
            };
            let Some(id_at) = layout.id.checked_add(4) else {
                continue;
            };
            let Some(id_bytes) = item.get(layout.id..id_at) else {
                continue;
            };
            let Ok(id_bytes) = <[u8; 4]>::try_from(id_bytes) else {
                continue;
            };
            names.insert(u32::from_le_bytes(id_bytes), name.to_owned());
        }

        Ok(Self { names })
    }

    /// Lists terrain voxel ids paired with the registry names of their yield items.
    pub fn materials(&self, terrain: Terrain) -> Vec<(u8, String)> {
        terrain
            .materials()
            .into_iter()
            .filter_map(|material| {
                self.names
                    .get(&material.terrain_item.get())
                    .map(|name| (material.terrain_material, name.clone()))
            })
            .collect()
    }
}
