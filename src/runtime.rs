use crate::{
    hooks::{self, Brick},
    journal::Journal,
};
use enshwick::{
    Load, Off,
    events::{ShutdownStarted, TerrainPageLoaded, WorldLoaded},
    voxels::{Voxel, VoxelBox, VoxelMaterial, VoxelPos},
    world::{TransactionId, TransactionStatus},
};
use std::{
    collections::{BTreeSet, VecDeque},
    ops::ControlFlow,
    sync::{Arc, Mutex},
    time::Duration,
};

#[derive(Default)]
struct State {
    settings: crate::settings::Settings,
    journal: Option<Journal>,
    ores: Option<crate::ores::Ores>,
    restore: VecDeque<Brick>,
    queued: BTreeSet<Brick>,
    inflight: Vec<(Brick, TransactionId)>,
    failed: bool,
    captures: u64,
    replays: u64,
}
fn raw(v: Voxel) -> u16 {
    let m = match v.material() {
        VoxelMaterial::Air => 0,
        VoxelMaterial::Terrain(m) | VoxelMaterial::Building(m) | VoxelMaterial::Unknown(m) => m,
        _ => 255,
    };
    u16::from_le_bytes([m, v.fill()])
}
fn voxel(w: u16) -> Voxel {
    let [m, f] = w.to_le_bytes();
    Voxel::new(
        match m {
            0 => VoxelMaterial::Air,
            1..=127 => VoxelMaterial::Terrain(m),
            _ => VoxelMaterial::Building(m),
        },
        f,
    )
}
pub fn install(load: &mut Load<'_>) -> Result<(), Off> {
    let settings = Arc::new(Mutex::new(crate::settings::Settings::default()));
    let configured = settings.clone();
    load.on_settings(move |value: &crate::settings::Settings| {
        *configured.lock().unwrap() = value.clone();
    })?;
    let catalog = crate::ore_materials::Catalog::resolve(load)?;
    let terrain = load.terrain();
    let world = load.world()?;
    let voxels = load.voxels();
    let state = Arc::new(Mutex::new(State::default()));
    let opened = state.clone();
    load.on::<WorldLoaded>(move |_| {
        hooks::clear();
        hooks::active(false);
        let mut s=opened.lock().unwrap();
        *s=State::default();
        let mut settings=settings.lock().unwrap().clone();
        let Some(key)=world.key() else { s.failed=true; return; };
        let Some(data)=world.data_dir() else {s.failed=true;enshwick::error!("World data folder unavailable");return;};
        if let Err(e)=crate::legacy::prepare(&data,&key) {s.failed=true;enshwick::error!("Legacy data import refused: {e}");return;}
        if let Err(e)=crate::settings::upgrade_legacy(&data,&mut settings) {s.failed=true;enshwick::error!("Legacy settings import refused: {e}");return;}
        s.settings=settings.clone();
        match Journal::open(data.join(crate::journal::DIRECTORY)) {
            Ok(journal)=> {
                let keys=match journal.keys(){Ok(keys)=>keys,Err(e)=>{s.failed=true;enshwick::error!("Journal index refused: {e}");return;}};
                s.queued.extend(keys.iter().copied());
                s.restore.extend(keys);
                enshwick::info!("Opened terrain journal: {} bricks",s.restore.len());
                s.journal=Some(journal);
                if settings.respawn_ores {
                    match crate::ores::Ores::open_shared(data.join("ore-respawn"),s.journal.as_ref().unwrap().store()) {
                        Ok(queue)=>s.ores=Some(queue),
                        Err(e)=>{s.failed=true;enshwick::error!("Ore timers refused: {e}");return;}
                    }
                    let materials=catalog.materials(terrain);
                    let text=materials.iter().map(|(id,name)|format!("{id}: {name}\n")).collect::<String>();
                    let _=std::fs::write(data.join("ore-materials.txt"),text);
                    let ids=materials.iter().filter(|(_,name)|crate::ore_materials::is_ore_item(name))
                        .map(|(id,_)|*id).chain(settings.extra_ore_materials.iter().copied()).collect::<Vec<_>>();
                    enshwick::info!("Ore respawn: {} seconds, materials {ids:?}",settings.ore_respawn_seconds);
                    hooks::ore_materials(ids);
                }
                if crate::terrain_editor::bind(voxels) {
                    hooks::active(true);
                } else {
                    s.failed=true;
                    enshwick::error!("Terrain editor unavailable; capture/replay disabled");
                }
            },
            Err(e)=> {s.failed=true; enshwick::error!("Terrain journal refused: {e}. Existing data preserved; capture and replay disabled.");}
        }
    })?;
    let closed = state.clone();
    load.on::<ShutdownStarted>(move |_| {
        hooks::active(false);
        let mut s = closed.lock().unwrap();
        let ore_delay = s.settings.ore_respawn_seconds;
        if let Some(queue) = s.ores.as_mut() {
            let updates = hooks::take_ores(usize::MAX);
            if let Err(e) =
                queue.schedule_many(&updates, crate::ores::now().saturating_add(ore_delay))
            {
                enshwick::error!("Shutdown ore timer flush failed: {e}");
            }
        }
        if let Some(j) = s.journal.as_mut() {
            if let Err(e) = j.record_deltas(&hooks::take(usize::MAX)) {
                enshwick::error!("Shutdown terrain flush failed: {e}");
            }
        }
    })?;
    let chunks = state.clone();
    load.on::<TerrainPageLoaded>(move |chunk| {
        let mut s = chunks.lock().unwrap();
        let Some(j) = s.journal.as_ref() else {
            return;
        };
        let size = chunk.size.saturating_mul(2);
        if size == 0 {
            return;
        }
        let keys = match j.keys() {
            Ok(keys) => keys,
            Err(e) => {
                s.failed = true;
                hooks::active(false);
                enshwick::error!("Journal index failed: {e}");
                return;
            }
        };
        for k in keys {
            if k[0] / size == chunk.x && k[2] / size == chunk.z && s.queued.insert(k) {
                s.restore.push_back(k);
            }
        }
    })?;
    load.scheduler()?
        .every(Duration::from_millis(100), move || {
            let mut s = state.lock().unwrap();
            if s.failed || s.journal.is_none() {
                return ControlFlow::Continue(());
            }
            let ore_delay = s.settings.ore_respawn_seconds;
            // Snapshot native edits before any replay, including further edits to saved bricks.
            if let Some(queue) = s.ores.as_mut() {
                let updates = hooks::take_ores(32);
                if let Err(e) =
                    queue.schedule_many(&updates, crate::ores::now().saturating_add(ore_delay))
                {
                    for (k, words) in updates {
                        hooks::retry_ores(k, words);
                    }
                    enshwick::error!("Ore timer write failed: {e}");
                }
            }
            let updates = hooks::take(32);
            if let Err(e) = s.journal.as_mut().unwrap().record_deltas(&updates) {
                for (k, words) in updates {
                    hooks::retry(k, words);
                }
                enshwick::error!("Terrain capture write failed, will retry: {e}");
            } else {
                s.captures += updates.len() as u64;
            }
            let active = std::mem::take(&mut s.inflight);
            for (k, id) in active {
                match world.transaction_status(id) {
                    Some(TransactionStatus::Pending) => s.inflight.push((k, id)),
                    Some(TransactionStatus::Applied) => {
                        hooks::cancel(k);
                        if hooks::take_deferred(k) {
                            s.restore.push_back(k);
                            continue;
                        }
                        s.queued.remove(&k);
                        s.replays += 1;
                    }
                    other => {
                        hooks::cancel(k);
                        hooks::take_deferred(k);
                        enshwick::warn!("Replay {k:?} retry after {other:?}");
                        s.restore.push_back(k);
                    }
                }
            }
            let n = s.restore.len().min(8);
            let due = match s
                .ores
                .as_ref()
                .map(|q| q.due(crate::ores::now(), 8))
                .transpose()
            {
                Ok(values) => values.unwrap_or_default(),
                Err(e) => {
                    s.failed = true;
                    hooks::active(false);
                    enshwick::error!("Ore deadline index failed: {e}");
                    return ControlFlow::Continue(());
                }
            };
            for (k, original) in due {
                if hooks::pending(k) {
                    continue;
                }
                if let Err(e) = s.journal.as_mut().unwrap().record_delta(k, &original) {
                    enshwick::error!("Ore restoration journal failed: {e}");
                    continue;
                }
                hooks::invalidate(k);
                if s.queued.insert(k) {
                    s.restore.push_back(k);
                }
                if let Err(e) = s.ores.as_mut().unwrap().remove(k) {
                    enshwick::error!("Ore timer removal failed: {e}");
                }
            }
            for _ in 0..n {
                let k = s.restore.pop_front().unwrap();
                let (halo_min, halo_size) = crate::terrain_editor::replay_halo(k);
                match voxels.read(
                    VoxelPos::new(halo_min[0], halo_min[1], halo_min[2]),
                    halo_size,
                ) {
                    Ok(_) => {}
                    Err(enshwick::WorldError::NotLoaded) => {
                        s.restore.push_back(k);
                        continue;
                    }
                    Err(e) => {
                        s.failed = true;
                        enshwick::error!("Replay halo {k:?} failed: {e}");
                        break;
                    }
                }
                if !crate::terrain_editor::replay_ready(
                    crate::terrain_editor::address() as *mut u8,
                    k,
                ) {
                    s.restore.push_back(k);
                    continue;
                }
                let min = VoxelPos::new(k[0], k[1], k[2]);
                match voxels.read(min, [8; 3]) {
                    Ok(b) => {
                        let current: [u16; 512] = b
                            .as_slice()
                            .iter()
                            .copied()
                            .map(raw)
                            .collect::<Vec<_>>()
                            .try_into()
                            .unwrap();
                        let merged = match s.journal.as_ref().unwrap().merge(k, &current) {
                            Ok(value) => value,
                            Err(e) => {
                                s.failed = true;
                                hooks::active(false);
                                enshwick::error!("Terrain record failed: {e}");
                                break;
                            }
                        };
                        if let Some(merged) = merged {
                            if merged == current {
                                s.queued.remove(&k);
                                continue;
                            }
                            let b =
                                VoxelBox::new(min, [8; 3], merged.into_iter().map(voxel).collect())
                                    .unwrap();
                            if !hooks::expect(k, merged) {
                                s.restore.push_back(k);
                                continue;
                            }
                            match world.transaction().write_voxels(&b).commit() {
                                Ok(id) => s.inflight.push((k, id)),
                                Err(e) => {
                                    hooks::cancel(k);
                                    enshwick::warn!("Replay queue refused: {e}");
                                    s.restore.push_back(k);
                                }
                            }
                        } else {
                            s.queued.remove(&k);
                        }
                    }
                    Err(enshwick::WorldError::NotLoaded) => s.restore.push_back(k),
                    Err(e) => {
                        s.failed = true;
                        enshwick::error!("Replay {k:?} failed: {e}");
                        break;
                    }
                }
            }
            ControlFlow::Continue(())
        });
    Ok(())
}
