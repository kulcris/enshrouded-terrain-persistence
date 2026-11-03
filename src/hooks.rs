//! Common voxel commit, located by a unique native code signature.
use enshwick::{Load, Next, Off};
use std::{
    cell::Cell,
    collections::{BTreeMap, BTreeSet},
    sync::{
        Mutex, OnceLock,
        atomic::{AtomicBool, Ordering},
    },
};
pub type Brick = [u32; 3];
pub type Image = [u16; 512];
static ACTIVE: AtomicBool = AtomicBool::new(false);
#[derive(Default)]
struct CaptureState {
    snapshots: BTreeMap<Brick, Image>,
    expected: BTreeMap<Brick, (Image, u64)>,
    versions: BTreeMap<Brick, u64>,
    deferred: BTreeSet<Brick>,
    ore_materials: BTreeSet<u8>,
    ores: BTreeMap<Brick, Image>,
}
static STATE: OnceLock<Mutex<CaptureState>> = OnceLock::new();
fn state() -> std::sync::MutexGuard<'static, CaptureState> {
    STATE.get_or_init(Default::default).lock().unwrap()
}
thread_local! {static REPLAY:Cell<bool>=const {Cell::new(false)};}
pub fn expect(k: Brick, image: Image) -> bool {
    state().expect(k, image)
}
pub fn take_deferred(k: Brick) -> bool {
    state().deferred.remove(&k)
}
impl CaptureState {
    fn expect(&mut self, k: Brick, image: Image) -> bool {
        // Never replay a journal image older than a native edit still waiting
        // to be persisted, including a large edit beyond this tick's budget.
        if self.snapshots.contains_key(&k) {
            return false;
        }
        let version = self.versions.get(&k).copied().unwrap_or(0);
        self.deferred.remove(&k);
        self.expected.insert(k, (image, version));
        true
    }
    fn capture(&mut self, k: Brick, image: Image) {
        let stored = self.snapshots.entry(k).or_insert([0xffff; 512]);
        for i in 0..512 {
            if image[i] != 0xffff {
                stored[i] = image[i];
            }
        }
        let version = self.versions.entry(k).or_default();
        *version = version.wrapping_add(1);
    }
}
pub fn cancel(k: Brick) {
    state().expected.remove(&k);
}
pub fn active(value: bool) {
    ACTIVE.store(value, Ordering::Release);
}
pub fn clear() {
    *state() = CaptureState::default();
}
pub fn ore_materials(materials: impl IntoIterator<Item = u8>) {
    state().ore_materials = materials.into_iter().collect();
}
pub fn take_ores(limit: usize) -> Vec<(Brick, Image)> {
    let mut s = state();
    let n = limit.min(s.ores.len());
    (0..n).filter_map(|_| s.ores.pop_first()).collect()
}
pub fn retry_ores(k: Brick, image: Image) {
    let mut s = state();
    let pending = s.ores.entry(k).or_insert([0xffff; 512]);
    for (word, old) in pending.iter_mut().zip(image) {
        if old != 0xffff {
            *word = old;
        }
    }
}
pub fn invalidate(k: Brick) {
    let mut s = state();
    let version = s.versions.entry(k).or_default();
    *version = version.wrapping_add(1);
}
pub fn pending(k: Brick) -> bool {
    let s = state();
    s.snapshots.contains_key(&k) || s.ores.contains_key(&k)
}
pub fn take(limit: usize) -> Vec<(Brick, Image)> {
    let mut q = state();
    let limit = limit.min(q.snapshots.len());
    (0..limit).filter_map(|_| q.snapshots.pop_first()).collect()
}
pub fn retry(k: Brick, image: Image) {
    let mut s = state();
    let stored = s.snapshots.entry(k).or_insert([0xffff; 512]);
    for i in 0..512 {
        if stored[i] == 0xffff {
            stored[i] = image[i];
        }
    }
}
#[repr(C)]
pub struct Words {
    data: *const u16,
    count: u64,
}
type Commit =
    unsafe extern "system" fn(*mut u8, *const Words, *const [u32; 3], *const [u32; 3]) -> usize;
#[repr(C)]
pub struct UndoRecord {
    min: [u32; 3],
    size: [u32; 3],
    voxels: *const u16,
    count: u64,
}
type Undo = unsafe extern "system" fn(*mut u8, *const UndoRecord, *mut u8) -> u8;
enshwick::detour! {
    static UNDO: unsafe extern "system" fn(editor:*mut u8,record:*const UndoRecord,alloc:*mut u8)->u8 = undo;
}
fn undo(next: Next<'_, Undo>, editor: *mut u8, record: *const UndoRecord, alloc: *mut u8) -> u8 {
    if !crate::terrain_editor::matches(editor) {
        // SAFETY: another voxel world's undo is forwarded without inspecting it.
        return unsafe { next.call(editor, record, alloc) };
    }
    let mut stale = false;
    let own = if record.is_null() {
        false
    } else {
        // SAFETY: pinned undo function reads this caller-owned head unchanged.
        let r = unsafe { &*record };
        let mut captured = state();
        let matched = r.size == [8; 3]
            && r.count >= 512
            && !r.voxels.is_null()
            && captured.expected.get(&r.min).is_some_and(|(image, _)| {
                // SAFETY: game undo record owns count readable voxel words.
                (unsafe { std::slice::from_raw_parts(r.voxels, 512) }) == image
            });
        if matched {
            let (_, version) = captured.expected.remove(&r.min).unwrap();
            stale = captured.versions.get(&r.min).copied().unwrap_or(0) != version;
        }
        matched
    };
    // A player or another mod changed this brick after replay was queued.
    // Refuse the stale write and explicitly defer it so the scheduler retries
    // with the newly captured image. Never overwrite the newer edit.
    if stale {
        state().deferred.insert(unsafe { (*record).min });
        return 1;
    }
    if own {
        // SAFETY: a matched expected replay has a live, non-null record.
        let k = unsafe { (*record).min };
        if !crate::terrain_editor::replay_ready(editor, k) {
            // An unloaded neighbour must never be substituted with air by
            // the native undo kernel. Load it and let the scheduler retry.
            state().deferred.insert(k);
            return 1;
        }
    }
    let previous = REPLAY.with(|v| {
        let old = v.get();
        v.set(old || own);
        old
    });
    // SAFETY: all native undo arguments are passed through unchanged.
    let result = unsafe { next.call(editor, record, alloc) };
    REPLAY.with(|v| v.set(previous));
    result
}
enshwick::detour! {
    static COMMIT: unsafe extern "system" fn(store:*mut u8,words:*const Words,size:*const [u32;3],min:*const [u32;3])->usize = commit;
}
fn commit(
    next: Next<'_, Commit>,
    store: *mut u8,
    words: *const Words,
    size: *const [u32; 3],
    min: *const [u32; 3],
) -> usize {
    let mut before = Vec::new();
    if ACTIVE.load(Ordering::Acquire)
        && crate::terrain_editor::matches(store)
        && !REPLAY.with(Cell::get)
        && !words.is_null()
        && !size.is_null()
        && !min.is_null()
    {
        // SAFETY: the caller owns these read-only descriptors and its voxel
        // buffer until this call returns. The original never frees that buffer.
        let (w, size, min) = unsafe { (&*words, *size, *min) };
        if !w.data.is_null()
            && min.iter().all(|v| v % 8 == 0)
            && size.iter().all(|v| *v > 0 && v % 8 == 0)
            && size
                .into_iter()
                .try_fold(1u64, |n, v| n.checked_mul(v as u64))
                .is_some_and(|n| n <= w.count)
        {
            for z in (0..size[2]).step_by(8) {
                for y in (0..size[1]).step_by(8) {
                    for x in (0..size[0]).step_by(8) {
                        let k = [min[0] + x, min[1] + y, min[2] + z];
                        if let Some(image) = crate::terrain_editor::brick(store, k) {
                            before.push((k, image));
                        } else {
                            enshwick::error!("Could not read pre-edit terrain brick {k:?}");
                        }
                    }
                }
            }
        } else {
            enshwick::error!(
                "Unsupported voxel commit min={min:?} size={size:?}; terrain capture incomplete"
            );
        }
    }
    // SAFETY: pinned native arguments and return register are forwarded unchanged.
    let result = unsafe { next.call(store, words, size, min) };
    for (k, old) in before {
        if let Some(new) = crate::terrain_editor::brick(store, k) {
            if old != new {
                let mut delta = [0xffff; 512];
                let mut originals = [0xffff; 512];
                let mut s = state();
                for i in 0..512 {
                    if old[i] != new[i] && old[i] & 0xff < 128 && new[i] & 0xff < 128 {
                        delta[i] = new[i];
                        if s.ore_materials.contains(&((old[i] & 255) as u8)) {
                            originals[i] = old[i];
                        }
                    }
                }
                if originals.iter().any(|&v| v != 0xffff) {
                    let saved = s.ores.entry(k).or_insert([0xffff; 512]);
                    for (word, value) in saved.iter_mut().zip(originals) {
                        if *word == 0xffff {
                            *word = value;
                        }
                    }
                }
                s.capture(k, delta);
            }
        } else {
            enshwick::error!("Could not read post-edit terrain brick {k:?}");
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn unflushed_native_edit_prevents_stale_replay_even_after_budget_exhausted() {
        let mut s = CaptureState::default();
        let k = [32, 16, 8];
        s.capture(k, [17; 512]);
        assert!(!s.expect(k, [3; 512]));
        assert!(s.expected.is_empty());
        assert_eq!(s.snapshots[&k], [17; 512]);
        s.snapshots.remove(&k);
        assert!(s.expect(k, [17; 512]));
        assert_eq!(s.expected[&k].1, 1);
        s.capture(k, [22; 512]);
        assert_ne!(s.expected[&k].1, s.versions[&k]);
    }
}
pub fn install(load: &mut Load<'_>) -> Result<(), Off> {
    let target = crate::native::commit(load)?;
    let expected = [
        0x4c, 0x8b, 0xdc, 0x4d, 0x89, 0x4b, 0x20, 0x4d, 0x89, 0x43, 0x18,
    ];
    // SAFETY: the unique signature resolves mapped executable code.
    if unsafe { std::slice::from_raw_parts(target as *const u8, expected.len()) } != expected {
        return Err(Off::new(
            "Native voxel commit signature differs; refusing hook",
        ));
    }
    let undo = load.symbol("voxel.undo_apply")?;
    let undo_expected = [0x48, 0x8b, 0xc4, 0x48, 0x81, 0xec, 0xa8, 0x00, 0x00, 0x00];
    // SAFETY: loader resolves undo using its validated table or pattern fallback;
    // additionally check the ABI-sensitive native prologue here.
    if unsafe { std::slice::from_raw_parts(undo as *const u8, undo_expected.len()) }
        != undo_expected
    {
        return Err(Off::new(
            "Native voxel undo signature differs; refusing hook",
        ));
    }
    // SAFETY: disassembly of the matched signature establishes these arguments;
    // all eleven callers are edit kernels, not scene loading or reset paths.
    unsafe {
        load.hook(&UNDO, undo)?;
        load.hook(&COMMIT, target)
    }
}
