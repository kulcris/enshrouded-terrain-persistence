//! Runtime-only native LOD readback probe. Call `run` from the game thread.
use enshwick::{
    Load, Off,
    voxels::{VoxelPos, Voxels},
};
use std::io::ErrorKind;

const FINE_MIN: [u32; 3] = [2768, 1368, 2568];
const FINE_SIZE: u32 = 64;
const LEVELS: u32 = 7;
const MAX_WORDS: usize = 64 * 64 * 64;
const WORLD_SIZE: [u32; 3] = [20480, 2048, 20480];

#[repr(C)]
pub struct Words {
    pub data: *mut u16,
    pub count: u64,
}

type ReadBox = unsafe extern "system" fn(
    out: *mut Words,
    size: *const [u32; 3],
    editor: *mut u8,
    detail: u32,
    min: *const [u32; 3],
) -> u8;

/// Cloneable readback handle. The native editor is learned on first snapshot.
#[derive(Clone, Copy)]
pub struct Probe {
    readfn: usize,
    voxels: Voxels,
}

pub type LevelSnapshot = (u32, [u32; 3], [u32; 3], Vec<u16>);

impl Probe {
    /// Read all seven LODs around the requested finest-voxel region.
    /// Call on the game thread, outside a game step.
    pub fn snapshot(&self) -> Result<Vec<LevelSnapshot>, String> {
        let fine_min = region_min()?;
        match self
            .voxels
            .sample(VoxelPos::new(fine_min[0], fine_min[1], fine_min[2]))
        {
            Ok(_) => {}
            Err(enshwick::WorldError::NotLoaded) => return Err("NotLoaded".to_owned()),
            Err(e) => return Err(format!("voxel sample: {e:?}")),
        }
        let editor = crate::terrain_editor::address();
        if editor == 0 {
            return Err("read_box binding did not capture an editor".to_owned());
        }
        // SAFETY: `readfn` is the loader-resolved function address kept alive by the hook.
        let read: ReadBox = unsafe { std::mem::transmute(self.readfn) };
        let mut snapshots = Vec::with_capacity(LEVELS as usize);
        for level in 0..LEVELS {
            let scale = 1u32 << (6 - level);
            let mut min = [0; 3];
            let mut size = [0; 3];
            for axis in 0..3 {
                min[axis] = fine_min[axis] / scale;
                let end = fine_min[axis] + FINE_SIZE;
                let ceil_end = end.div_ceil(scale);
                size[axis] = ceil_end - min[axis];
            }
            let count = size
                .iter()
                .try_fold(1usize, |n, v| n.checked_mul(*v as usize))
                .ok_or_else(|| format!("mip {level}: voxel count overflow"))?;
            if count == 0 || count > MAX_WORDS {
                return Err(format!("mip {level}: invalid voxel count {count}"));
            }
            let mut data = vec![0u16; count];
            let mut words = Words {
                data: data.as_mut_ptr(),
                count: count as u64,
            };
            // SAFETY: all pointed-to values remain valid for the synchronous native call;
            // the output buffer has exactly the checked voxel count.
            let status = unsafe { read(&mut words, &size, editor as *mut u8, level, &min) };
            if status == 0 {
                return Err("NotLoaded".to_owned());
            }
            if words.count != count as u64 {
                return Err(format!(
                    "mip {level}: native count={} expected={count}",
                    words.count
                ));
            }
            snapshots.push((level, min, size, data));
        }
        Ok(snapshots)
    }

    /// Save each LOD baseline, or compare each current LOD against its baseline.
    pub fn run(&self, baseline: bool) -> Result<String, String> {
        let snapshots = self.snapshot()?;
        let mut reports = Vec::with_capacity(snapshots.len());
        for (level, min, size, words) in snapshots {
            let path = format!("shape-mip-{level}.bin");
            let mut bytes = Vec::with_capacity(24 + words.len() * 2);
            for v in min.into_iter().chain(size) {
                bytes.extend_from_slice(&v.to_le_bytes());
            }
            for word in &words {
                bytes.extend_from_slice(&word.to_le_bytes());
            }
            if baseline {
                std::fs::write(&path, &bytes)
                    .map_err(|e| format!("mip {level}: write {path}: {e}"))?;
                reports.push(format!("mip {level} saved words={}", words.len()));
            } else {
                let reference =
                    std::fs::read(&path).map_err(|e| format!("mip {level}: read {path}: {e}"))?;
                if reference.len() != bytes.len() {
                    return Err(format!(
                        "mip {level}: baseline length={} expected={}",
                        reference.len(),
                        bytes.len()
                    ));
                }
                if reference[..24] != bytes[..24] {
                    return Err(format!("mip {level}: baseline min/size header differs"));
                }
                let different = words
                    .iter()
                    .enumerate()
                    .filter(|(i, word)| {
                        let at = 24 + i * 2;
                        u16::from_le_bytes([reference[at], reference[at + 1]]) != **word
                    })
                    .count();
                reports.push(format!("mip {level} different={different}/{}", words.len()));
            }
        }
        Ok(reports.join("\n"))
    }
}

pub fn install(load: &mut Load<'_>) -> Result<Probe, Off> {
    let readfn = load.symbol("voxel.read_box")?;
    Ok(Probe {
        readfn: readfn as usize,
        voxels: load.voxels(),
    })
}

fn region_min() -> Result<[u32; 3], String> {
    let path = "probe-region.txt";
    let contents = match std::fs::read_to_string(path) {
        Ok(s) => s,
        Err(e) if e.kind() == ErrorKind::NotFound => return Ok(FINE_MIN),
        Err(e) => return Err(format!("probe region: read {path}: {e}")),
    };
    let fields: Vec<_> = contents.split_whitespace().collect();
    if fields.len() != 3 {
        return Err(format!(
            "probe region: expected exactly three unsigned coordinates in {path}"
        ));
    }
    let mut min = [0; 3];
    for (axis, field) in fields.iter().enumerate() {
        min[axis] = field.parse::<u32>().map_err(|_| {
            format!("probe region: invalid unsigned coordinate {field:?} in {path}")
        })?;
    }
    for axis in 0..3 {
        let max_min = WORLD_SIZE[axis] - FINE_SIZE;
        if min[axis] > max_min {
            return Err(format!(
                "probe region: coordinate axis {axis}={} exceeds world maximum {max_min}",
                min[axis]
            ));
        }
    }
    Ok(min)
}
