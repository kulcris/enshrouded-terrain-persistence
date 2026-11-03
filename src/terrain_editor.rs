//! Bind the terrain editor through the loader's own terrain read API.
//! Generic native voxel functions may also be used by other voxel worlds.
use enshwick::{
    Load, Next, Off,
    voxels::{VoxelPos, Voxels},
};
use std::{
    cell::Cell,
    sync::atomic::{AtomicUsize, Ordering},
};
static EDITOR: AtomicUsize = AtomicUsize::new(0);
static READ_FN: AtomicUsize = AtomicUsize::new(0);
thread_local! {static BIND:Cell<bool>=const {Cell::new(false)};}
#[repr(C)]
pub struct Words {
    data: *mut u16,
    count: u64,
}
type Read =
    unsafe extern "system" fn(*mut Words, *const [u32; 3], *mut u8, u32, *const [u32; 3]) -> u8;
enshwick::detour! {
    static READ: unsafe extern "system" fn(out:*mut Words,size:*const[u32;3],editor:*mut u8,level:u32,min:*const[u32;3])->u8 = read;
}
fn read(
    next: Next<'_, Read>,
    out: *mut Words,
    size: *const [u32; 3],
    editor: *mut u8,
    level: u32,
    min: *const [u32; 3],
) -> u8 {
    if BIND.with(Cell::get) {
        EDITOR.store(editor as usize, Ordering::Release);
    }
    // SAFETY: matches the loader's native voxel reader, all arguments unchanged.
    unsafe { next.call(out, size, editor, level, min) }
}
pub fn bind(voxels: Voxels) -> bool {
    EDITOR.store(0, Ordering::Release);
    struct Guard;
    impl Drop for Guard {
        fn drop(&mut self) {
            BIND.with(|v| v.set(false));
        }
    }
    BIND.with(|v| v.set(true));
    let _guard = Guard;
    let _ = voxels.sample(VoxelPos::new(0, 0, 0));
    let editor = EDITOR.load(Ordering::Acquire);
    if editor != 0 {
        enshwick::info!("Terrain voxel editor bound at {editor:#x}");
    }
    editor != 0
}
pub fn matches(editor: *mut u8) -> bool {
    !editor.is_null() && editor as usize == EDITOR.load(Ordering::Acquire)
}
pub fn address() -> usize {
    EDITOR.load(Ordering::Acquire)
}
pub fn brick(editor: *mut u8, min: [u32; 3]) -> Option<[u16; 512]> {
    if !matches(editor)
        || min[0].checked_add(8)? > 20480
        || min[1].checked_add(8)? > 2048
        || min[2].checked_add(8)? > 20480
    {
        return None;
    }
    let function = READ_FN.load(Ordering::Acquire);
    if function == 0 {
        return None;
    }
    let mut data = [0u16; 512];
    let mut out = Words {
        data: data.as_mut_ptr(),
        count: 512,
    };
    let size = [8u32; 3];
    // SAFETY: supported native reader, live terrain editor, exact 512-word
    // buffer. The reader takes and releases its own column locks.
    let read: Read = unsafe { std::mem::transmute(function) };
    let resident = unsafe { read(&mut out, &size, editor, 6, &min) };
    (resident != 0).then_some(data)
}
/// The native undo kernel reads a padded working cube, not just its output
/// brick. Every intersecting column must be resident before it starts.
pub fn replay_halo(k: [u32; 3]) -> ([u32; 3], [u32; 3]) {
    let limit = [20480u32, 2048, 20480];
    let min = k.map(|v| v.saturating_sub(8));
    let size = [0, 1, 2].map(|axis| limit[axis].saturating_sub(min[axis]).min(24));
    (min, size)
}
pub fn replay_ready(editor: *mut u8, k: [u32; 3]) -> bool {
    if !matches(editor) {
        return false;
    }
    let (min, size) = replay_halo(k);
    if size.contains(&0) {
        return false;
    }
    let function = READ_FN.load(Ordering::Acquire);
    if function == 0 {
        return false;
    }
    // SAFETY: this guard runs only for our scheduled replay, on the quiet
    // game thread. The exact halo buffer and live terrain editor are valid.
    let read: Read = unsafe { std::mem::transmute(function) };
    let mut ready = true;
    for level in 0..=6 {
        let scale = 1u32 << (6 - level);
        let lo = min.map(|v| v / scale);
        let dimensions = [0, 1, 2].map(|axis| (min[axis] + size[axis]).div_ceil(scale) - lo[axis]);
        let count = dimensions.iter().map(|v| *v as usize).product();
        let mut data = vec![0u16; count];
        let mut out = Words {
            data: data.as_mut_ptr(),
            count: count as u64,
        };
        // Request every level in one pass, even when one is still loading.
        ready &= unsafe { read(&mut out, &dimensions, editor, level, &lo) != 0 };
    }
    ready
}

#[cfg(test)]
mod halo_tests {
    use super::replay_halo;
    #[test]
    fn replay_at_streaming_boundary_loads_the_western_column() {
        assert_eq!(
            replay_halo([7424, 1680, 2864]),
            ([7416, 1672, 2856], [24; 3])
        );
    }
    #[test]
    fn replay_halo_stays_within_world_at_both_corners() {
        assert_eq!(replay_halo([0; 3]), ([0; 3], [24; 3]));
        assert_eq!(
            replay_halo([20472, 2040, 20472]),
            ([20464, 2032, 20464], [16; 3])
        );
    }
}
pub fn install(load: &mut Load<'_>) -> Result<(), Off> {
    let target = load.symbol("voxel.read_box")?;
    READ_FN.store(target as usize, Ordering::Release);
    // SAFETY: the documented pinned read_box signature from the loader's source.
    unsafe { load.hook(&READ, target) }
}
