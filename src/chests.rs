//! Restore an empty world chest in place using the same
//! ECS command layout Enshwick uses for adding/removing DoNotSave.
//! Command ABI adapted from Enshwick's MIT-licensed entities.rs; see notices.
use enshwick::{
    Entity, Host, Load, Off,
    content::types,
    ecs::ComponentType,
    inventory::Inventories,
    resources::{Registry, Value},
    world::{World, WritableType},
};
type Append = unsafe extern "system" fn(*mut u8, *mut u8, u8, u32, u32) -> *mut u8;
fn valid_layout(
    hash: usize,
    size: usize,
    data: usize,
    remove_hash: usize,
    remove_size: usize,
) -> bool {
    let Some(hash_end) = hash.checked_add(4) else {
        return false;
    };
    let Some(size_end) = size.checked_add(2) else {
        return false;
    };
    data <= 1024
        && hash_end <= data
        && size_end <= data
        && (hash_end <= size || size_end <= hash)
        && remove_size <= 1024
        && remove_hash
            .checked_add(4)
            .is_some_and(|end| end <= remove_size)
}
pub struct Reset {
    world: World,
    host: Host,
    inventories: Inventories,
    append: Append,
    buffers: usize,
    records: usize,
    add: u8,
    remove: u8,
    hash: usize,
    size: usize,
    data: usize,
    remove_size: usize,
    remove_hash: usize,
    template_type: u32,
    types: Vec<(&'static str, WritableType)>,
    derived: Vec<ComponentType>,
    actor: WritableType,
    state_at: usize,
    state_size: usize,
    version: enshwick::game_events::Field<u32>,
}
#[cfg(test)]
mod tests {
    #[test]
    fn accepts_separate_bounded_command_fields() {
        assert!(super::valid_layout(8, 12, 16, 8, 12));
    }
    #[test]
    fn rejects_overlaps_out_of_bounds_and_overflow() {
        assert!(!super::valid_layout(8, 10, 16, 8, 12));
        assert!(!super::valid_layout(14, 8, 16, 8, 12));
        assert!(!super::valid_layout(8, 12, 16, 10, 12));
        assert!(!super::valid_layout(usize::MAX, 0, 16, 8, 12));
    }
}
impl Reset {
    pub fn resolve(load: &mut Load<'_>) -> Result<Self, Off> {
        let world = load.world()?;
        let inventories = load.inventories()?;
        let at = |n| {
            load.constant(n)
                .and_then(|v| usize::try_from(v).map_err(|_| Off::new("command offset")))
        };
        let hash = at("ecs.command_set_hash")?;
        let size = at("ecs.command_set_size")?;
        let data = at("ecs.command_set_data")?;
        let remove_size = at("ecs.remove_components_size")?;
        let remove_hash = at("ecs.remove_components_hashes")?;
        if !valid_layout(hash, size, data, remove_hash, remove_size) {
            return Err(Off::new("unsafe command layout"));
        }
        let mut types = Vec::new();
        for n in [
            "keen::ecs::DefaultInventory",
            "keen::ecs::InventorySetup",
            "keen::ecs::InventorySetupInit",
            "keen::ecs::InventoryEmptyState",
            "keen::ecs::InteractionOfferRuntime",
        ] {
            types.push((n, WritableType::resolve(load, n)?));
        }
        let derived = [
            "keen::ecs::RandomLootInventory",
            "keen::ecs::RandomLootLock",
        ]
        .into_iter()
        .map(|n| ComponentType::resolve(load, n))
        .collect::<Result<Vec<_>, _>>()?;
        let actor = WritableType::resolve(load, "keen::ecs::BaseActor")?;
        let state_at = load.field_offset("keen::ecs::BaseActor", "currentState")? as usize;
        let state_size = load.type_size("keen::actor::StateMask")? as usize;
        if state_size > 256
            || state_at
                .checked_add(state_size)
                .is_none_or(|end| end > actor.size() as usize)
        {
            return Err(Off::new("actor state layout"));
        }
        let address = load.symbol("ecs.command_append")? as usize;
        let version = WritableType::resolve(load, "keen::ecs::RandomLootContainer")?
            .field::<u32>(load, "spawnedLootVersion")?;
        Ok(Self {
            world,
            host: load.host(),
            inventories,
            version,
            append: unsafe { std::mem::transmute::<usize, Append>(address) },
            buffers: at("ecs.command_buffers")?,
            records: at("ecs.command_records")?,
            add: u8::try_from(at("ecs.add_components")?).map_err(|_| Off::new("add opcode"))?,
            remove: u8::try_from(at("ecs.remove_components")?)
                .map_err(|_| Off::new("remove opcode"))?,
            hash,
            size,
            data,
            remove_size,
            remove_hash,
            template_type: types::TEMPLATE_RESOURCE.hash()?,
            types,
            derived,
            actor,
            state_at,
            state_size,
        })
    }
    fn append(&self, entity: Entity, kind: u8, size: usize) -> Result<*mut u8, String> {
        let world = (self.host.api().server_world)() as usize;
        if world == 0 || size > 65535 {
            return Err("no world or oversized command".into());
        }
        // SAFETY: called by the mod's scheduler on the game thread between steps.
        // These are the loader-resolved world command-buffer and append layouts.
        let buffer = unsafe { std::ptr::read_unaligned((world + self.buffers) as *const *mut u8) };
        if buffer.is_null() {
            return Err("no command buffer".into());
        }
        let command = unsafe {
            (self.append)(
                buffer,
                buffer.add(self.records),
                kind,
                size as u32,
                entity.0,
            )
        };
        if command.is_null() {
            Err("command buffer refused reset".into())
        } else {
            Ok(command)
        }
    }
    pub fn reset(&self, entity: Entity) -> Result<String, String> {
        if self.world.used_item(entity).is_some() {
            return Err("player storage excluded".into());
        }
        let template = self.world.template(entity).ok_or("missing entity")?;
        let bytes = self
            .host
            .game_resource(enshwick::abi::ResourceKey {
                guid: template,
                type_hash: self.template_type,
                part: 0,
            })
            .ok_or("missing template")?;
        let registry = Registry::game().map_err(|e| e.to_string())?;
        let root = registry
            .decode("keen::ecs::TemplateResource", &bytes)
            .map_err(|e| e.to_string())?;
        if !matches!(root.field("name"),Some(Value::Text(n)) if n.starts_with("LootChest_")) {
            return Err("not a world chest".into());
        }
        let Some(Value::List(components)) = root.field("components") else {
            return Err("template has no components".into());
        };
        let mut prepared = Vec::new();
        let mut mask = None;
        for item in components {
            let Value::Variant(v) = item else { continue };
            if let Some((name, ty)) = self.types.iter().find(|(n, _)| *n == v.type_name()) {
                let data = registry
                    .encode(name, v.value(), None)
                    .map_err(|e| e.to_string())?;
                if data.len() != ty.size() as usize || data.len() > 65535 - self.data {
                    return Err("component size mismatch".into());
                }
                prepared.push((*name, *ty, data));
            }
            if v.type_name() == "keen::ecs::BaseActor" {
                if let Some(state) = v.value().field("currentState") {
                    mask = Some(
                        registry
                            .encode("keen::actor::StateMask", state, None)
                            .map_err(|e| e.to_string())?,
                    );
                }
            }
        }
        if ![
            "keen::ecs::InventorySetup",
            "keen::ecs::InventorySetupInit",
            "keen::ecs::InteractionOfferRuntime",
        ]
        .iter()
        .all(|required| prepared.iter().any(|(name, _, _)| name == required))
        {
            return Err("unsupported chest inventory setup".into());
        }
        // Do not touch inventories that hold any items, linked inventories included.
        if self.inventories.slots(entity).iter().any(|s| s.count > 0) {
            return Err("chest still holds items".into());
        }
        let mut done = Vec::new();
        for ty in &self.derived {
            if self.world.component(entity, *ty).is_some() {
                let p = self.append(entity, self.remove, self.remove_size)?;
                unsafe {
                    std::ptr::write_unaligned(p.add(self.remove_hash).cast::<u32>(), ty.hash());
                }
                done.push(format!("remove {:x}", ty.hash()));
            }
        }
        for (name, ty, data) in prepared {
            if self.world.component(entity, ty.ty()).is_some() {
                self.world
                    .update(entity, ty, |b| b.copy_from_slice(&data))
                    .map_err(|e| format!("{name}: {e:?}"))?;
            } else {
                let p = self.append(entity, self.add, self.data + data.len())?;
                unsafe {
                    std::ptr::write_unaligned(p.add(self.hash).cast::<u32>(), ty.ty().hash());
                    std::ptr::write_unaligned(p.add(self.size).cast::<u16>(), data.len() as u16);
                    std::ptr::copy_nonoverlapping(data.as_ptr(), p.add(self.data), data.len());
                }
            }
            done.push(name.to_owned());
        }
        if let Some(mask) = mask {
            if mask.len() != self.state_size {
                return Err("state mask size".into());
            }
            self.world
                .update(entity, self.actor, |b| {
                    b[self.state_at..self.state_at + self.state_size].copy_from_slice(&mask)
                })
                .map_err(|e| format!("actor: {e:?}"))?;
            done.push("actor state".into());
        }
        if self.world.get(entity, self.version).is_some() {
            self.world
                .set(entity, self.version, 0)
                .map_err(|e| format!("loot version: {e:?}"))?;
        }
        Ok(format!("restored {entity:?}: {}", done.join(", ")))
    }
}
