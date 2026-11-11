//! Timed regeneration of world loot, never player storage or claimed crops.
use crate::{
    loot_store::{Key, Record, Timers},
    settings::Settings,
    store::Store,
};
use enshwick::{
    Entity, Load, Off,
    content::{Blob, types},
    ecs::ComponentType,
    effects::{Effects, Spawn},
    game_events::{AcceptOfferedInteractionEvent, EntityDiedEvent, Field, LootInteractionEvent},
    inventory::Inventories,
    world::{World, WritableType},
};
use std::{
    collections::{BTreeMap, VecDeque},
    io,
    sync::{Arc, Mutex},
};

#[derive(Clone)]
pub struct Api {
    world: World,
    chest_reset: Option<Arc<crate::chests::Reset>>,
    inventories: Inventories,
    effects: Effects,
    applied: Field<bool>,
    default_inventory: WritableType,
    table_offset: usize,
    node: ComponentType,
    node_drops: ComponentType,
    spawned_child: Field<Entity>,
    spawner: ComponentType,
    excluded: Vec<ComponentType>,
    host: enshwick::Host,
    template_type: u32,
    name_offset: usize,
    inventory_empty: Field<bool>,
    loot_empty: Field<bool>,
    random_type: ComponentType,
    area_excluded: Field<bool>,
    orientation: usize,
    events: Arc<Mutex<VecDeque<(Record, Option<Entity>)>>>,
}
impl Api {
    pub fn install(load: &mut Load<'_>) -> Result<Self, Off> {
        let world = load.world()?;
        let chest_reset = match crate::chests::Reset::resolve(load) {
            Ok(reset) => Some(Arc::new(reset)),
            Err(e) => {
                enshwick::warn!("Chest refill unavailable on this build: {e}");
                None
            }
        };
        let default_inventory = WritableType::resolve(load, "keen::ecs::DefaultInventory")?;
        let applied = default_inventory.field::<bool>(load, "applied")?;
        let table_offset =
            usize::try_from(load.field_offset("keen::ecs::DefaultInventory", "defaultInventory")?)
                .map_err(|_| Off::new("loot table offset"))?;
        if table_offset
            .checked_add(16)
            .is_none_or(|end| end > default_inventory.size() as usize)
        {
            return Err(Off::new("loot table outside component"));
        }
        let node = ComponentType::resolve(load, "keen::ecs::MiningNode")?;
        let node_drops = ComponentType::resolve(load, "keen::ecs::ResourceNodePickupDrops")?;
        let spawner_type = WritableType::resolve(load, "keen::ecs::RandomLootEntitySpawner")?;
        let spawned_child = spawner_type.field::<Entity>(load, "spawnedEntityId")?;
        let inventory_empty = WritableType::resolve(load, "keen::ecs::InventoryEmptyState")?
            .field::<bool>(load, "isEmpty")?;
        let loot_empty = WritableType::resolve(load, "keen::ecs::RandomLootLock")?
            .field::<bool>(load, "isEmpty")?;
        let random_type = WritableType::resolve(load, "keen::ecs::RandomLootContainer")?;
        let area_excluded = WritableType::resolve(load, "keen::ecs::AreaLootOverride")?
            .field::<bool>(load, "exclude")?;
        let excluded = [
            "keen::ecs::LifeTime",
            "keen::ecs::PlayerInput",
            "keen::ecs::ActiveNpcState",
            "keen::ecs::Animal",
        ]
        .into_iter()
        .map(|name| ComponentType::resolve(load, name))
        .collect::<Result<Vec<_>, _>>()?;
        let offset = load
            .constant("current_transform.transform")?
            .checked_add(load.constant("world_transform.orientation")?)
            .ok_or_else(|| Off::new("node orientation overflow"))?;
        let orientation =
            usize::try_from(offset).map_err(|_| Off::new("node orientation offset"))?;
        let size = load.type_size("keen::ecs::CurrentTransform")? as usize;
        if orientation.checked_add(16).is_none_or(|end| end > size) {
            return Err(Off::new("node orientation outside transform"));
        }
        let api = Self {
            world,
            chest_reset,
            inventories: load.inventories()?,
            effects: load.effects(),
            applied,
            default_inventory,
            table_offset,
            node,
            node_drops,
            spawned_child,
            spawner: spawner_type.ty(),
            excluded,
            host: load.host(),
            template_type: types::TEMPLATE_RESOURCE.hash()?,
            name_offset: usize::try_from(load.constant("template.name")?)
                .map_err(|_| Off::new("template name offset"))?,
            inventory_empty,
            loot_empty,
            random_type: random_type.ty(),
            area_excluded,
            orientation,
            events: Arc::default(),
        };
        let used = api.clone();
        load.on::<AcceptOfferedInteractionEvent>(move |e| used.arm(e.host_entity_id, false))?;
        let died = api.clone();
        load.on::<EntityDiedEvent>(move |e| died.arm(e.dead_entity_id, true))?;
        let looted = api.clone();
        load.on::<LootInteractionEvent>(move |e| looted.arm_chest(e.host_id))?;
        let pickup = load.game_event_type("keen::ecs::PickupEvent")?;
        let source = pickup.field::<Entity>(load, "pickedUpFromEntityId")?;
        let picked = api.clone();
        load.on_raw_game_event(pickup, move |event| {
            if let Some(entity) = event.get(source) {
                picked.arm_pickup(entity);
            }
        })?;
        Ok(api)
    }
    fn arm(&self, entity: Entity, died: bool) {
        if entity.0 == 0 {
            return;
        }
        // The game event still has the source entity: never infer harvesting
        // merely from an entity disappearing when its page unloads.
        if (self.world.component(entity, self.node).is_none()
            && self.world.component(entity, self.node_drops).is_none())
            || self.world.used_item(entity).is_some()
            || self.world.get(entity, self.area_excluded) == Some(true)
            || self
                .excluded
                .iter()
                .any(|&ty| self.world.component(entity, ty).is_some())
        {
            return;
        }
        let Some(position) = self.world.position(entity) else {
            return;
        };
        if !unclaimed(self.world, position) {
            return;
        }
        let Some(mut record) = self.record(entity, position, 2) else {
            return;
        };
        record.due = crate::ores::now(); // Harvest time; configured delay is added on drain.
        let mut events = self.events.lock().unwrap();
        if events.len() < 1024 {
            events.push_back((record, (!died).then_some(entity)));
        }
    }
    fn arm_chest(&self, entity: Entity) {
        if entity.0 == 0 {
            return;
        }
        let entity = self.parent(entity).unwrap_or(entity);
        if self.world.used_item(entity).is_some()
            || self.world.get(entity, self.area_excluded) == Some(true)
            || self
                .excluded
                .iter()
                .any(|&ty| self.world.component(entity, ty).is_some())
        {
            return;
        }
        let Some(at) = self.world.position(entity) else {
            return;
        };
        let Some(kind) = self.kind(entity) else {
            return;
        };
        if kind == 2
            && (self.world.get(entity, self.applied).is_none() || !unclaimed(self.world, at))
        {
            return;
        }
        let Some(mut record) = self.record(entity, at, kind) else {
            return;
        };
        record.due = crate::ores::now();
        let mut events = self.events.lock().unwrap();
        if events.len() < 1024 {
            events.push_back((record, Some(entity)));
        }
    }
    fn parent(&self, child: Entity) -> Option<Entity> {
        if child.0 == 0 {
            return None;
        }
        self.world
            .entities(self.spawner)
            .into_iter()
            .find(|p| self.world.get(p.entity, self.spawned_child) == Some(child))
            .map(|p| p.entity)
    }
    fn arm_pickup(&self, entity: Entity) {
        if entity.0 == 0 {
            return;
        }
        // World-authored inventory pickups (bandages, bundles, etc.) have a
        // DefaultInventory; thrown stacks carry AreaLootOverride/LifeTime.
        if self.parent(entity).is_some() || self.kind(entity).is_some() {
            self.arm_chest(entity);
            return;
        }
        self.arm(entity, false);
    }
    fn kind(&self, entity: Entity) -> Option<u8> {
        let guid = self.world.template(entity)?;
        template_kind(&self.template_name(guid)?)
    }
    fn template_name(&self, guid: [u8; 16]) -> Option<String> {
        let data = self.host.game_resource(enshwick::abi::ResourceKey {
            guid,
            type_hash: self.template_type,
            part: 0,
        })?;
        let (at, len) = Blob::new(&data).array(self.name_offset)?;
        let name = std::str::from_utf8(data.get(at..at + usize::try_from(len).ok()?)?)
            .ok()?
            .trim_end_matches('\0');
        Some(name.to_owned())
    }
    fn inventory_root(&self, entity: Entity) -> Entity {
        self.world
            .get(entity, self.spawned_child)
            .filter(|e| self.world.position(*e).is_some())
            .unwrap_or(entity)
    }
    fn chest_is_empty(&self, entity: Entity) -> bool {
        if let Some(empty) = self.world.get(entity, self.loot_empty) {
            empty
        } else if self.world.component(entity, self.random_type).is_some() {
            false
        } else {
            self.world.get(entity, self.inventory_empty) == Some(true)
        }
    }
    fn table(&self, entity: Entity) -> Option<[u8; 16]> {
        let p = self.world.component(entity, self.default_inventory.ty())?;
        // SAFETY: live game-thread component; reflected extent was checked
        // during installation. Copy it before returning to the game.
        Some(unsafe {
            std::ptr::read_unaligned(p.cast::<u8>().add(self.table_offset).cast::<[u8; 16]>())
        })
    }
    fn record(&self, entity: Entity, position: [f64; 3], kind: u8) -> Option<Record> {
        let template = self.world.template(entity)?;
        let transform = self.world.component(entity, self.world.transform_type())?;
        // SAFETY: live game-thread component, reflected offset checked against
        // CurrentTransform's size during install; read before returning to game.
        let orientation = unsafe {
            std::ptr::read_unaligned(
                transform
                    .cast::<u8>()
                    .add(self.orientation)
                    .cast::<[f32; 4]>(),
            )
        };
        if position.iter().any(|v| !v.is_finite()) || orientation.iter().any(|v| !v.is_finite()) {
            return None;
        }
        Some(Record {
            key: Key {
                kind,
                template,
                position: enshwick::ecs::fixed(position),
            },
            due: 0,
            orientation,
            fill_table: self.table(entity).unwrap_or([0; 16]),
        })
    }
    pub fn clear(&self) {
        self.events.lock().unwrap().clear();
    }
}

pub struct Runtime {
    api: Api,
    timers: Timers,
    settings: Settings,
    next_scan: u64,
    // Require a player to have remained nearby for several scans before spawn;
    // terrain reads alone do not establish that an ECS page finished loading.
    nearby_since: BTreeMap<Key, u64>,
    interactions: Vec<(Record, Entity)>,
    cursor: Option<(u64, Key)>,
    initializing: Vec<(Entity, Record, u64)>,
}
impl Runtime {
    pub fn open(api: Api, store: Store, settings: Settings) -> io::Result<Self> {
        api.clear();
        Ok(Self {
            api,
            timers: Timers::open_store(store)?,
            settings,
            next_scan: 0,
            nearby_since: BTreeMap::new(),
            interactions: Vec::new(),
            cursor: None,
            initializing: Vec::new(),
        })
    }
    pub fn tick(&mut self) -> io::Result<()> {
        let now = crate::ores::now();
        for (entity, record, started) in std::mem::take(&mut self.initializing) {
            if self.api.world.position(entity).is_none() {
                if now.saturating_sub(started) < 10 {
                    self.initializing.push((entity, record, started));
                }
                continue;
            }
            if self
                .api
                .world
                .component(entity, self.api.default_inventory.ty())
                .is_some()
            {
                if record.fill_table != [0; 16]
                    && self.api.table(entity) != Some(record.fill_table)
                    && !self
                        .api
                        .inventories
                        .slots(entity)
                        .iter()
                        .any(|s| s.count > 0)
                {
                    self.api
                        .world
                        .update(entity, self.api.default_inventory, |bytes| {
                            bytes[self.api.table_offset..self.api.table_offset + 16]
                                .copy_from_slice(&record.fill_table)
                        })
                        .map_err(io::Error::other)?;
                    self.api
                        .world
                        .set(entity, self.api.applied, false)
                        .map_err(io::Error::other)?;
                }
            }
        }
        let events: Vec<_> = self.api.events.lock().unwrap().drain(..).collect();
        for (mut record, interaction) in events {
            if (record.key.kind == 1 && self.settings.respawn_chests)
                || (record.key.kind == 2 && self.settings.respawn_lootables)
            {
                if let Some(entity) = interaction {
                    if self.interactions.len() < 1024 {
                        if !self
                            .interactions
                            .iter()
                            .any(|(r, e)| r.key == record.key && *e == entity)
                        {
                            self.interactions.push((record, entity));
                        }
                    }
                } else {
                    record.due = record
                        .due
                        .saturating_add(self.settings.lootable_respawn_seconds);
                    self.timers.schedule(&record)?;
                }
            }
        }
        if now < self.next_scan {
            return Ok(());
        }
        self.next_scan = now.saturating_add(5);
        let world = self.api.world;
        let players = world
            .players()
            .iter()
            .filter_map(|p| world.position(p.entity))
            .collect::<Vec<_>>();
        if self.settings.respawn_chests && !players.is_empty() {
            // Spent chests no longer have DefaultInventory. Native loot state
            // lets us capture them without requiring that removed component.
            for chest in world.entities(self.api.random_type) {
                if self.api.chest_is_empty(chest.entity)
                    && self.api.kind(chest.entity) == Some(1)
                    && world.used_item(chest.entity).is_none()
                {
                    if let Some(mut record) = self.api.record(chest.entity, chest.position, 1) {
                        if self.timers.get(&record.key)?.is_none() {
                            record.due = now.saturating_add(self.settings.chest_respawn_seconds);
                            self.timers.schedule(&record)?;
                            enshwick::info!("Scheduled spent world chest at {:?}", chest.position);
                        }
                    }
                }
            }
        }
        for (mut record, entity) in std::mem::take(&mut self.interactions) {
            let elapsed = now.saturating_sub(record.due);
            if elapsed < 2 {
                self.interactions.push((record, entity));
                continue;
            }
            let at = enshwick::ecs::metres(record.key.position);
            let nearby = players.iter().any(|&p| near(p, at));
            let missing = world.position(entity).is_none();
            let empty_chest = record.key.kind == 1
                && self.api.chest_is_empty(entity)
                && !self
                    .api
                    .inventories
                    .slots(self.api.inventory_root(entity))
                    .iter()
                    .any(|s| s.count > 0);
            if nearby && (missing || empty_chest) {
                record.due = if record.key.kind == 1 {
                    now.saturating_add(self.settings.chest_respawn_seconds)
                } else {
                    record
                        .due
                        .saturating_add(self.settings.lootable_respawn_seconds)
                };
                if self
                    .timers
                    .get(&record.key)?
                    .is_none_or(|old| old.due <= now)
                {
                    self.timers.schedule(&record)?;
                }
                enshwick::info!(
                    "Scheduled {} respawn at {at:?}",
                    if record.key.kind == 1 {
                        "world loot"
                    } else {
                        "resource node"
                    }
                );
            } else if nearby && (record.key.kind == 1 || elapsed < 10) {
                self.interactions.push((record, entity));
            }
        }
        // Rotate bounded deadline windows so distant overdue nodes cannot
        // starve loaded ones or require loading the whole queue into memory.
        let due = self.timers.due_after(now, 128, self.cursor)?;
        self.cursor = due.last().map(|r| (r.due, r.key));
        let placed = if due.is_empty() {
            Vec::new()
        } else {
            world.entities(world.transform_type())
        };
        let mut attempts = 0;
        self.nearby_since.retain(|key, _| {
            players
                .iter()
                .any(|&p| near(p, enshwick::ecs::metres(key.position)))
        });
        for record in due {
            // Retire timers written by the earlier experimental drop capture,
            // including timers whose source entity has already disappeared.
            if self
                .api
                .template_name(record.key.template)
                .is_some_and(|name| generated_drop(&name))
            {
                self.timers.remove(&record.key)?;
                continue;
            }
            if !(if record.key.kind == 1 {
                self.settings.respawn_chests
            } else {
                self.settings.respawn_lootables
            }) {
                continue;
            }
            let at = enshwick::ecs::metres(record.key.position);
            if !players.iter().any(|&p| near(p, at))
                || (record.key.kind == 2 && !unclaimed(world, at))
            {
                continue;
            }
            let since = self.nearby_since.entry(record.key).or_insert(now);
            if now.saturating_sub(*since) < 15 {
                continue;
            }
            // Any existing copy, depleted or naturally regrown, stops spawn.
            // Scene helpers can share the same transform. Only a player's
            // placed prop, rather than every helper, blocks replacement.
            let existing = placed
                .iter()
                .filter(|p| distance_squared(p.position, at) <= 0.25 * 0.25)
                .collect::<Vec<_>>();
            if let Some(chest) = existing
                .iter()
                .find(|p| world.template(p.entity) == Some(record.key.template))
            {
                // A present wild pickup/node is already regrown. Never infer
                // an uncollected PickupItem is empty from its inventory slots.
                if record.key.kind == 1
                    && world.used_item(chest.entity).is_none()
                    && self.api.chest_is_empty(chest.entity)
                    && !self
                        .api
                        .inventories
                        .slots(self.api.inventory_root(chest.entity))
                        .iter()
                        .any(|s| s.count > 0)
                {
                    // Keep the scene entity and rebuild its consumed inventory
                    // setup. Generic template spawns lack the scene's loot link.
                    let mut retry = record.clone();
                    retry.due = now.saturating_add(30);
                    self.timers.schedule(&retry)?;
                    if let Some(reset) = self.api.chest_reset.as_ref() {
                        match reset.reset(chest.entity) {
                            Ok(_) => {
                                self.timers.remove(&record.key)?;
                                enshwick::info!("Refilled world chest at {at:?}");
                            }
                            Err(e) => enshwick::warn!("World chest refill failed: {e}"),
                        }
                    }
                }
                continue;
            }
            // Missing scene chests need their native scene instance. Do not
            // manufacture an empty replacement; vanilla resets still apply.
            if record.key.kind == 1 {
                continue;
            }
            if existing.iter().any(|p| world.used_item(p.entity).is_some()) {
                continue;
            }
            if attempts >= 8 {
                break;
            }
            attempts += 1;
            // Persist retry before spawn. On crash/restart the proximity check
            // sees the saved replacement and does not spawn a second copy.
            let mut retry = record.clone();
            retry.due = now.saturating_add(30);
            self.timers.schedule(&retry)?;
            let mut how = Spawn::new().do_not_save();
            how.orientation = Some(record.orientation);
            match self.api.effects.spawn(record.key.template, at, how) {
                Ok(entity) => {
                    self.initializing.push((entity, record.clone(), now));
                    // Keep the retry until a later scan observes the actual
                    // entity: a queued unknown template may spawn nothing.
                    enshwick::debug!("Requested harvested resource node respawn at {at:?}");
                }
                Err(e) => enshwick::warn!("Resource node respawn refused: {e:?}"),
            }
        }
        Ok(())
    }
}
fn near(a: [f64; 3], b: [f64; 3]) -> bool {
    distance_squared(a, b) <= 32.0 * 32.0
}
fn template_kind(name: &str) -> Option<u8> {
    if generated_drop(name) {
        return None;
    }
    if name
        .split('_')
        .any(|part| part.eq_ignore_ascii_case("quest"))
    {
        return None;
    }
    if name.starts_with("LootChest_") {
        Some(1)
    } else if name.starts_with("LootPickup_") {
        Some(2)
    } else {
        None
    }
}
fn generated_drop(name: &str) -> bool {
    name.starts_with("LootPickup_Gravity_")
}
#[cfg(test)]
mod tests {
    #[test]
    fn distinguishes_real_chest_and_bandage_templates_from_quest_and_generic_drops() {
        assert_eq!(
            super::template_kind("LootChest_Random_silver_noMultiplayerScaling"),
            Some(1)
        );
        assert_eq!(super::template_kind("LootChest_player_start"), Some(1));
        assert_eq!(super::template_kind("LootPickup_Gravity_Wood_Loose"), None);
        assert_eq!(super::template_kind("LootPickup_Gravity_Fabric"), None);
        assert_eq!(
            super::template_kind("LootPickup_Consumable_Bandage01"),
            Some(2)
        );
        assert_eq!(super::template_kind("LootPickup_Quest_Key"), None);
        assert_eq!(super::template_kind("PickupItem_Generic"), None);
        assert_eq!(super::template_kind("Player_Chest"), None);
    }
}
fn distance_squared(a: [f64; 3], b: [f64; 3]) -> f64 {
    a.into_iter()
        .zip(b)
        .map(|(a, b)| (a - b) * (a - b))
        .sum::<f64>()
}
fn unclaimed(world: World, at: [f64; 3]) -> bool {
    world.persists(at.map(|v| v - 0.25), at.map(|v| v + 0.25)) == Ok(false)
}
