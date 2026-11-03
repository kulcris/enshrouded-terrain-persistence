//! Explicit, test-build-only commands for the disposable local server.
use enshwick::{Load, Off, terrain::Shape};
use std::{
    ops::ControlFlow,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};
pub fn install(load: &mut Load<'_>) -> Result<(), Off> {
    let mip = crate::mip_probe::install(load)?;
    let terrain = load.terrain();
    let world = load.world()?;
    let voxels = load.voxels();
    let data = load.data_dir()?;
    let altars = load.altars()?;
    let ready = Arc::new(AtomicBool::new(false));
    let flag = ready.clone();
    load.on::<enshwick::events::WorldLoaded>(move |_| {
        flag.store(true, Ordering::Release);
    })?;
    let mut completed = String::new();
    let mut warmup = 0;
    let mut last_transaction = None;
    let mut race_expected: Option<enshwick::voxels::VoxelBox> = None;
    let mut asymmetric_command = String::new();
    let mut asymmetric_stage = 0;
    load.scheduler()?.every(Duration::from_secs(2), move || {
        if !ready.load(Ordering::Acquire) {
            return ControlFlow::Continue(());
        }
        if warmup < 5 {
            warmup += 1;
            return ControlFlow::Continue(());
        }
        if let Some(expected) = race_expected.take() {
            let result = match voxels.read(expected.min(), expected.size()) {
                Ok(actual) if actual == expected => {
                    "race verified: newer native edit preserved".to_owned()
                }
                other => format!("race FAILED: {other:?}"),
            };
            enshwick::info!("RUNTIME TEST {result}");
            let _ = std::fs::write("terrain-test-result.txt", result);
        }
        let command = std::fs::read_to_string("terrain-test-command.txt").unwrap_or_default();
        if command == completed || command.trim().is_empty() {
            return ControlFlow::Continue(());
        }
        let action = command.lines().next().unwrap_or_default().trim();
        if action=="ore-seed" || action=="ore-mine" {
            let names=std::fs::read_to_string(data.join("ore-materials.txt")).unwrap_or_default();
            let material=|needle:&str|names.lines().find_map(|line| {
                let (id,name)=line.split_once(": ")?;
                (name==needle).then(||id.parse::<u8>().ok()).flatten()
            });
            if let (Some(copper),Some(clay))=(material("Material_T2_Ore_Copper"),material("Material_T2_Clay")) {
                let _=std::fs::write("probe-region.txt","2784 1376 2568");
                if voxels.read(enshwick::voxels::VoxelPos::new(2784,1376,2568),[64;3]).is_err() {return ControlFlow::Continue(());}
                if action=="ore-seed" || action=="ore-mine" {
                    use enshwick::voxels::{Voxel,VoxelBox,VoxelMaterial,VoxelPos};
                    let value=|material|if action=="ore-seed" {Voxel::new(VoxelMaterial::Terrain(material),255)} else {Voxel::new(VoxelMaterial::Air,0)};
                    let a=VoxelBox::new(VoxelPos::new(2816,1408,2600),[8;3],vec![value(copper);512]).unwrap();
                    let b=VoxelBox::new(VoxelPos::new(2832,1408,2600),[8;3],vec![value(clay);512]).unwrap();
                    if let Ok(id)=world.transaction().write_voxels(&a).write_voxels(&b).commit() {
                        let _=std::fs::write("terrain-test-result.txt",format!("{action} queued {id:?}"));completed=command;
                    }
                    return ControlFlow::Continue(());
                }
            }
            return ControlFlow::Continue(());
        }
        if action == "claim-add" {
            if let Some(altar) = altars.list().first() {
                let p = [altar.position[0] + 4.0, altar.position[1] + 2.0, altar.position[2] + 4.0];
                let min = p.map(|v| (v * 2.0 - 32.0).max(0.0) as u32 & !7);
                let _ = std::fs::write("probe-region.txt", format!("{} {} {}", min[0], min[1], min[2]));
                let bounds = (p.map(|v| v - 2.0), p.map(|v| v + 2.0));
                if world.persists(bounds.0, bounds.1) != Ok(true) {
                    let _ = std::fs::write("terrain-test-result.txt", "claim-add FAILED: target outside claim");
                    completed = command;
                } else {
                    match terrain.add(p, 1.5, 2.0, 1, Shape::Round) {
                        Ok(volume) => {
                            let _ = std::fs::write("terrain-test-result.txt", format!("claim-add volume={volume} persists=true position={p:?}"));
                            completed = command;
                        }
                        Err(_) => {}
                    }
                }
            }
            return ControlFlow::Continue(());
        }
        if action=="probe-player" {
            if let Some(p)=world.players().first().and_then(|p|world.position(p.entity)) {
                let limits=[20416u32,1984,20416];
                let min=[0,1,2].map(|i|((p[i]*2.0-32.0).max(0.0) as u32).min(limits[i])&!7);
                let text=format!("{} {} {}",min[0],min[1],min[2]);
                match std::fs::write("probe-region.txt",&text) {
                    Ok(())=> {
                        let result=format!("player probe min=[{text}] position={p:?}");
                        let _=std::fs::write("terrain-test-result.txt",&result);
                        enshwick::info!("RUNTIME TEST {result}");
                    },
                    Err(e)=>enshwick::error!("Probe region write failed: {e}"),
                }
                completed=command;
            }
            return ControlFlow::Continue(());
        }
        if action=="mip-baseline" || action=="mip-verify" {
            match mip.run(action=="mip-baseline") {
                Ok(text)=> {
                    enshwick::info!("RUNTIME TEST {text}");
                    let _=std::fs::write("terrain-test-result.txt",text);
                    completed=command;
                },
                Err(e) if e=="NotLoaded"=>{},
                Err(e)=> {
                    enshwick::error!("RUNTIME TEST mip failure: {e}");
                    let _=std::fs::write("terrain-test-result.txt",format!("mip FAILED: {e}"));
                    completed=command;
                }
            }
            return ControlFlow::Continue(());
        }
        if action=="status" {
            let Some(key)=world.key() else {return ControlFlow::Continue(());};
            if let Ok(journal)=crate::journal::Journal::open(data.join("worlds").join(key).join(crate::journal::DIRECTORY)) {
                let keys=journal.keys();
                let mut claimed=0;
                for k in &keys {
                    let a=k.map(|v|v as f64*0.5);
                    let b=k.map(|v|(v+8) as f64*0.5);
                    if world.persists(a,b)==Ok(true) {claimed+=1;}
                }
                let text=format!("status bricks={} wholly_claimed={claimed} other={} altars={}",keys.len(),keys.len()-claimed,altars.list().len());
                enshwick::info!("RUNTIME TEST {text}");
                let _=std::fs::write("terrain-test-result.txt",text);
                completed=command;
            }
            return ControlFlow::Continue(());
        }
        if action == "verify" || action == "compare" {
            let Some(key) = world.key() else {
                return ControlFlow::Continue(());
            };
            let journal = match crate::journal::Journal::open(
                data.join("worlds").join(key).join(crate::journal::DIRECTORY),
            ) {
                Ok(j) => j,
                Err(e) => {
                    enshwick::error!("TEST journal failed: {e}");
                    return ControlFlow::Continue(());
                }
            };
            let keys = journal.keys();
            let mut matches = 0;
            let mut different = 0;
            let mut unloaded = 0;
            for k in &keys {
                match voxels.read(enshwick::voxels::VoxelPos::new(k[0], k[1], k[2]), [8; 3]) {
                    Ok(b) => {
                        use enshwick::voxels::VoxelMaterial;
                        let words: [u16; 512] = b
                            .as_slice()
                            .iter()
                            .map(|v| {
                                let m = match v.material() {
                                    VoxelMaterial::Air => 0,
                                    VoxelMaterial::Terrain(m)
                                    | VoxelMaterial::Building(m)
                                    | VoxelMaterial::Unknown(m) => m,
                                    _ => 255,
                                };
                                u16::from_le_bytes([m, v.fill()])
                            })
                            .collect::<Vec<_>>()
                            .try_into()
                            .unwrap();
                        if journal.merge(*k, &words) == Some(words) {
                            matches += 1;
                        } else {
                            different += 1;
                        }
                    }
                    Err(_) => unloaded += 1,
                }
            }
            let text = format!(
                "compare bricks={} matches={matches} different={different} unloaded={unloaded}",
                keys.len()
            );
            let _ = std::fs::write("terrain-test-result.txt", &text);
            if !keys.is_empty() && unloaded == 0 && (action == "compare" || different == 0) {
                enshwick::info!("RUNTIME TEST {text}");
                completed = command;
            }
            return ControlFlow::Continue(());
        }
        if action == "shape-baseline" || action == "shape-verify" {
            use enshwick::voxels::VoxelMaterial;
            const SHAPE_MIN: [u32; 3] = [2768, 1368, 2568];
            const SHAPE_SIZE: [u32; 3] = [64; 3];
            let shape_path = std::path::Path::new("shape-baseline.bin");
            let text = match voxels.read(
                enshwick::voxels::VoxelPos::new(SHAPE_MIN[0], SHAPE_MIN[1], SHAPE_MIN[2]),
                SHAPE_SIZE,
            ) {
                Err(enshwick::WorldError::NotLoaded | enshwick::WorldError::Missing) => {
                    return ControlFlow::Continue(());
                }
                Err(e) => format!("{} FAILED: voxel read: {e}", action),
                Ok(b) => {
                    let bytes: Vec<u8> = b
                        .as_slice()
                        .iter()
                        .flat_map(|v| {
                            let material = match v.material() {
                                VoxelMaterial::Air => 0,
                                VoxelMaterial::Terrain(m)
                                | VoxelMaterial::Building(m)
                                | VoxelMaterial::Unknown(m) => m,
                                _ => 255,
                            };
                            [material, v.fill()]
                        })
                        .collect();
                    if bytes.len() != 524288 {
                        format!("{} FAILED: voxel data length={} expected=524288", action, bytes.len())
                    } else if action == "shape-baseline" {
                        match std::fs::write(shape_path, &bytes) {
                            Ok(()) => format!("shape baseline voxels=262144 bytes={}", bytes.len()),
                            Err(e) => format!("shape-baseline FAILED: write {}: {e}", shape_path.display()),
                        }
                    } else {
                        match std::fs::read(shape_path) {
                            Err(e) => format!("shape-verify FAILED: read {}: {e}", shape_path.display()),
                            Ok(reference) if reference.len() != 262144 * 2 => format!(
                                "shape-verify FAILED: baseline length={} expected=524288",
                                reference.len()
                            ),
                            Ok(reference) => {
                                let mut different = 0usize;
                                let mut first = None;
                                for i in 0..262144 {
                                    let expected = u16::from_le_bytes([reference[i * 2], reference[i * 2 + 1]]);
                                    let actual = u16::from_le_bytes([bytes[i * 2], bytes[i * 2 + 1]]);
                                    if expected != actual {
                                        different += 1;
                                        if first.is_none() {
                                            let [x, y, z] = [
                                                (i % 64) as u32,
                                                ((i / 64) % 64) as u32,
                                                (i / 4096) as u32,
                                            ];
                                            first = Some((
                                                [SHAPE_MIN[0] + x, SHAPE_MIN[1] + y, SHAPE_MIN[2] + z],
                                                expected,
                                                actual,
                                            ));
                                        }
                                    }
                                }
                                let first_text = first.map_or_else(
                                    || "".to_owned(),
                                    |(p, expected, actual)| format!(" first=[{},{},{}] expected={expected:#06x} actual={actual:#06x}", p[0], p[1], p[2]),
                                );
                                format!("shape compare voxels=262144 different={different}{first_text}")
                            }
                        }
                    }
                }
            };
            enshwick::info!("RUNTIME TEST {text}");
            let _ = std::fs::write("terrain-test-result.txt", &text);
            completed = command;
            return ControlFlow::Continue(());
        }
        if action == "asymmetric" {
            if asymmetric_command != command {
                asymmetric_command = command.clone();
                asymmetric_stage = 0;
            }
            let Some(material) = terrain
                .materials()
                .into_iter()
                .find(|m| m.can_be_flattened)
                .map(|m| m.terrain_material)
            else {
                return ControlFlow::Continue(());
            };
            let result = if asymmetric_stage == 0 {
                terrain.add([1400.37, 700.42, 1300.83], 3.7, 2.3, material, Shape::Square)
                    .map(|v| {
                        asymmetric_stage = 1;
                        format!("asymmetric add volume={v}")
                    })
            } else {
                terrain.dig([1401.1, 701.0, 1300.2], 1.2, Shape::Round, &[])
                    .map(|v| {
                        asymmetric_stage = 2;
                        format!("asymmetric dig volume={v}")
                    })
            };
            match result {
                Ok(text) if asymmetric_stage == 1 => {
                    enshwick::info!("RUNTIME TEST {text}");
                    return ControlFlow::Continue(());
                }
                Ok(text) => {
                    enshwick::info!("RUNTIME TEST {text}");
                    let _ = std::fs::write("terrain-test-result.txt", &text);
                    completed = command;
                }
                Err(enshwick::WorldError::NotLoaded | enshwick::WorldError::Missing) => {}
                Err(e) => {
                    let text = format!("asymmetric FAILED: {e}");
                    enshwick::error!("RUNTIME TEST {text}");
                    let _ = std::fs::write("terrain-test-result.txt", &text);
                    completed = command;
                }
            }
            return ControlFlow::Continue(());
        }
        let p = [1024.0, 768.0, 1024.0];
        let Some(material) = terrain
            .materials()
            .into_iter()
            .find(|m| m.can_be_flattened)
            .map(|m| m.terrain_material)
        else {
            return ControlFlow::Continue(());
        };
        let result = match action {
            "add" => terrain.add(p, 3.0, 3.0, material, Shape::Square).map(|v| {
                format!(
                    "add volume={v}; persists={:?}",
                    world.persists([1020.0, 764.0, 1020.0], [1028.0, 776.0, 1028.0])
                )
            }),
            "dig" => terrain
                .dig(p, 1.0, Shape::Round, &[])
                .map(|v| format!("dig volume={v}")),
            "flatten" => terrain
                .flatten([1024.0, 770.0, 1024.0], 2.0, material, Shape::Square)
                .map(|v| format!("flatten changed={v}")),
            "save" => world.save().map(|_| "save requested".to_owned()),
            "paste" => {
                use enshwick::voxels::{Voxel, VoxelBox, VoxelMaterial, VoxelPos};
                let b = VoxelBox::new(
                    VoxelPos::new(2120, 1536, 2048),
                    [4; 3],
                    vec![Voxel::new(VoxelMaterial::Terrain(material), 200); 64],
                )
                .unwrap();
                world.transaction().write_voxels(&b).commit().map(|id| {
                    last_transaction = Some(id);
                    format!("paste queued id={}", id.get())
                })
            }
            "undo" => match last_transaction {
                Some(id) => world.undo(id).map(|_| "undo queued".to_owned()),
                None => Err(enshwick::WorldError::Missing),
            },
            "race" => {
                use enshwick::voxels::{Voxel, VoxelBox, VoxelPos};
                let k = [2048, 1544, 2048];
                let old = VoxelBox::new(
                    VoxelPos::new(k[0], k[1], k[2]),
                    [8; 3],
                    vec![Voxel::default(); 512],
                )
                .unwrap();
                // Load the region before queuing either write.
                match voxels.read(old.min(), old.size()) {
                    Err(e) => Err(e),
                    Ok(_) => {
                        crate::hooks::expect(k, [0; 512]);
                        match world.transaction().write_voxels(&old).commit() {
                            Err(e) => {
                                crate::hooks::cancel(k);
                                Err(e)
                            }
                            Ok(_) => terrain
                                .add([1024.0, 772.0, 1024.0], 3.0, 3.0, material, Shape::Square)
                                .and_then(|v| {
                                    race_expected = Some(voxels.read(old.min(), old.size())?);
                                    Ok(format!("race queued; new native volume={v}"))
                                }),
                        }
                    }
                }
            }
            _ => {
                completed = command;
                return ControlFlow::Continue(());
            }
        };
        match result {
            Ok(text) => {
                enshwick::info!("RUNTIME TEST {text}");
                let _ = std::fs::write("terrain-test-result.txt", &text);
                completed = command;
            }
            Err(enshwick::WorldError::NotLoaded | enshwick::WorldError::Missing) => {}
            Err(e) => {
                enshwick::error!("RUNTIME TEST failed: {e}");
                completed = command;
            }
        }
        ControlFlow::Continue(())
    });
    Ok(())
}
