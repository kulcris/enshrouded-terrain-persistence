//! One-time import of alpha 11 records into the world-scoped alpha 13 data dir.
use crate::journal;
use std::{
    fs, io,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

const MOD_ID: &str = "kulcr.terrain-persistence";
const IMPORTING: &str = ".legacy-importing";
const IMPORTED: &str = ".legacy-imported";
static TEMP_ID: AtomicU64 = AtomicU64::new(0);

/// Imports the alpha 11 data associated with this save, if it exists.
///
/// In-progress imports are resumable. A destination containing records without
/// a matching in-progress marker is left untouched and reported as an error.
pub fn prepare(data: &Path, world_key: &str) -> io::Result<()> {
    prepare_from(data, world_key, &std::env::current_dir()?)
}

fn prepare_from(data: &Path, world_key: &str, cwd: &Path) -> io::Result<()> {
    let savefolder = data
        .parent()
        .and_then(Path::parent)
        .ok_or_else(|| invalid("world data path has no save folder"))?;
    let world_key = legacy_world_key(world_key)?;
    let source = cwd
        .join("mod-data")
        .join(MOD_ID)
        .join("worlds")
        .join(format!("{world_key}-{:08x}", fnv1a_path(savefolder)?));
    let source_identity = format!("{}\n{}", normalized_path(&source), world_key);
    let importing = data.join(IMPORTING);
    let imported = data.join(IMPORTED);

    if imported.exists() {
        return Ok(());
    }
    let resuming = if importing.exists() {
        check_marker(&importing, &source_identity)?;
        true
    } else {
        false
    };

    let old_terrain = source.join(journal::DIRECTORY);
    let old_ores = source.join("ore-respawn");
    let has_terrain = old_terrain.is_dir();
    let has_ores = old_ores.is_dir();
    if !has_terrain && !has_ores {
        if resuming {
            return Err(invalid(
                "legacy import source disappeared before the import completed",
            ));
        }
        return Ok(());
    }

    // Validate both source stores before creating or copying any destination data.
    if has_terrain {
        drop(journal::legacy_records(&old_terrain)?);
    }
    if has_ores {
        drop(crate::ores::legacy_records(&old_ores)?);
    }

    let new_terrain = data.join(journal::DIRECTORY);
    let new_ores = data.join("ore-respawn");
    if !resuming {
        if contains_records(&new_terrain, "brick")?
            || contains_records(&new_terrain, "redb")?
            || contains_records(&new_ores, "json")?
            || contains_records(&new_ores, "redb")?
        {
            return Err(invalid(
                "new data already has records without a matching legacy-import marker",
            ));
        }
        fs::create_dir_all(data)?;
        write_marker(&importing, &source_identity)?;
    }

    if has_terrain {
        // Validate any partial destination before resuming a prior import.
        drop(journal::legacy_records(&new_terrain)?);
        copy_records(&old_terrain, &new_terrain, "brick")?;
        drop(journal::legacy_records(&new_terrain)?);
    }
    if has_ores {
        drop(crate::ores::legacy_records(&new_ores)?);
        copy_records(&old_ores, &new_ores, "json")?;
        drop(crate::ores::legacy_records(&new_ores)?);
    }

    fs::rename(&importing, &imported)?;
    Ok(())
}

fn contains_records(dir: &Path, extension: &str) -> io::Result<bool> {
    if !dir.exists() {
        return Ok(false);
    }
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        if entry.file_type()?.is_file()
            && entry.path().extension().and_then(|s| s.to_str()) == Some(extension)
        {
            return Ok(true);
        }
    }
    Ok(false)
}

fn copy_records(source: &Path, destination: &Path, extension: &str) -> io::Result<()> {
    fs::create_dir_all(destination)?;
    let mut files = Vec::new();
    for entry in fs::read_dir(source)? {
        let entry = entry?;
        if entry.file_type()?.is_file()
            && entry.path().extension().and_then(|s| s.to_str()) == Some(extension)
        {
            files.push(entry);
        }
    }
    files.sort_by_key(|entry| entry.file_name());
    for entry in files {
        let target = destination.join(entry.file_name());
        let bytes = fs::read(entry.path())?;
        if target.exists() {
            if fs::read(&target)? == bytes {
                continue;
            }
            return Err(invalid(
                "legacy record conflicts with an existing destination record",
            ));
        }
        let temp = temp_path(&target);
        let result = (|| {
            use io::Write;
            let mut file = fs::File::create(&temp)?;
            file.write_all(&bytes)?;
            file.sync_all()?;
            fs::rename(&temp, &target)
        })();
        if result.is_err() {
            let _ = fs::remove_file(temp);
        }
        result?;
    }
    Ok(())
}

fn write_marker(path: &Path, contents: &str) -> io::Result<()> {
    fs::create_dir_all(
        path.parent()
            .ok_or_else(|| invalid("marker has no parent"))?,
    )?;
    let temp = temp_path(path);
    let result = (|| {
        use io::Write;
        let mut file = fs::File::create(&temp)?;
        file.write_all(contents.as_bytes())?;
        file.sync_all()?;
        fs::rename(&temp, path)
    })();
    if result.is_err() {
        let _ = fs::remove_file(temp);
    }
    result
}

fn check_marker(path: &Path, expected: &str) -> io::Result<()> {
    let actual = fs::read_to_string(path)?;
    if actual == expected {
        Ok(())
    } else {
        Err(invalid(
            "legacy import marker belongs to a different source or world",
        ))
    }
}

fn legacy_world_key(world_key: &str) -> io::Result<&str> {
    let key = world_key
        .trim_end_matches(|ch| ch == '/' || ch == '\\')
        .rsplit(|ch| ch == '/' || ch == '\\')
        .next()
        .filter(|key| !key.is_empty())
        .ok_or_else(|| invalid("world key is empty"))?;
    Ok(key)
}

fn temp_path(target: &Path) -> PathBuf {
    let id = TEMP_ID.fetch_add(1, Ordering::Relaxed);
    let name = target.file_name().unwrap_or_default().to_string_lossy();
    target.with_file_name(format!(".{name}.legacy-{}-{id}.tmp", std::process::id()))
}

fn fnv1a_path(path: &Path) -> io::Result<u32> {
    let full = fs::canonicalize(path)?;
    let text = normalized_path(&full);
    Ok(text.as_bytes().iter().fold(0x811c9dc5u32, |hash, byte| {
        (hash ^ u32::from(*byte)).wrapping_mul(0x01000193)
    }))
}

fn normalized_path(path: &Path) -> String {
    let mut value = path.to_string_lossy().replace('\\', "/").to_lowercase();
    if let Some(without_prefix) = value.strip_prefix("//?/") {
        value = without_prefix.to_owned();
    }
    value
}

fn invalid(message: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{hooks::Image, journal::BrickKey};
    use std::time::{SystemTime, UNIX_EPOCH};

    static TEST_ID: AtomicU64 = AtomicU64::new(0);

    fn test_dir() -> PathBuf {
        let id = TEST_ID.fetch_add(1, Ordering::Relaxed);
        std::env::temp_dir().join(format!(
            "legacy-import-{}-{}-{id}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ))
    }

    #[test]
    fn migration_resumes_and_preserves_legacy_files() {
        let root = test_dir();
        let cwd = root.join("cwd");
        let save = root.join("saves").join("FixtureWorld");
        let data = save.join("world0-mod-data").join(MOD_ID);
        fs::create_dir_all(&cwd).unwrap();
        fs::create_dir_all(&save).unwrap();
        let source = cwd
            .join("mod-data")
            .join(MOD_ID)
            .join("worlds")
            .join(format!("world0-{:08x}", fnv1a_path(&save).unwrap()));
        let old_terrain = source.join(journal::DIRECTORY);
        let old_ores = source.join("ore-respawn");
        let keys: [BrickKey; 2] = [[8, 16, 24], [40, 48, 56]];
        let mut image: Image = [0xffff; 512];
        image[4] = 0xff23;
        fs::create_dir_all(&old_terrain).unwrap();
        fs::create_dir_all(&old_ores).unwrap();
        let terrain_path = old_terrain.join(format!(
            "{}-{}-{}.brick",
            keys[0][0], keys[0][1], keys[0][2]
        ));
        fs::write(&terrain_path, encode_legacy(keys[0], &[0x0011; 512])).unwrap();
        let ore_path = old_ores.join(format!("{}-{}-{}.json", keys[1][0], keys[1][1], keys[1][2]));
        fs::write(
            &ore_path,
            serde_json::to_vec(&serde_json::json!({
                "key": keys[1],
                "due": 1234,
                "original": image.to_vec(),
            }))
            .unwrap(),
        )
        .unwrap();

        // Seed the matching progress marker and one already-copied record as if
        // the process stopped between atomic record copies.
        let source_identity = format!("{}\nworld0", normalized_path(&source));
        fs::create_dir_all(&data).unwrap();
        fs::write(data.join(IMPORTING), source_identity).unwrap();
        let partial = data.join(journal::DIRECTORY);
        fs::create_dir_all(&partial).unwrap();
        fs::copy(
            old_terrain.join(format!(
                "{}-{}-{}.brick",
                keys[0][0], keys[0][1], keys[0][2]
            )),
            partial.join(format!(
                "{}-{}-{}.brick",
                keys[0][0], keys[0][1], keys[0][2]
            )),
        )
        .unwrap();

        let full_world_key = format!("{}/world0", normalized_path(&save));
        prepare_from(&data, &full_world_key, &cwd).unwrap();
        assert!(data.join(IMPORTED).is_file());
        assert!(!data.join(IMPORTING).exists());
        assert!(
            old_terrain
                .join(format!(
                    "{}-{}-{}.brick",
                    keys[0][0], keys[0][1], keys[0][2]
                ))
                .is_file()
        );
        assert!(
            old_ores
                .join(format!("{}-{}-{}.json", keys[1][0], keys[1][1], keys[1][2]))
                .is_file()
        );
        assert!(
            data.join("ore-respawn")
                .join(format!("{}-{}-{}.json", keys[1][0], keys[1][1], keys[1][2]))
                .is_file()
        );
        assert_eq!(
            fs::read(data.join(journal::DIRECTORY).join(format!(
                "{}-{}-{}.brick",
                keys[0][0], keys[0][1], keys[0][2]
            )))
            .unwrap(),
            fs::read(&terrain_path).unwrap()
        );
        assert_eq!(
            fs::read(
                data.join("ore-respawn")
                    .join(format!("{}-{}-{}.json", keys[1][0], keys[1][1], keys[1][2]))
            )
            .unwrap(),
            fs::read(&ore_path).unwrap()
        );
        assert!(!old_terrain.join("records.redb").exists());
        assert!(!old_ores.join("records.redb").exists());

        fs::remove_dir_all(root).unwrap();
    }

    fn encode_legacy(key: BrickKey, words: &[u16; 512]) -> Vec<u8> {
        let mut bytes = b"EVJNL001".to_vec();
        for coordinate in key {
            bytes.extend_from_slice(&coordinate.to_le_bytes());
        }
        for word in words {
            bytes.extend_from_slice(&word.to_le_bytes());
        }
        let checksum = bytes.iter().fold(0x811c9dc5u32, |hash, byte| {
            (hash ^ u32::from(*byte)).wrapping_mul(0x01000193)
        });
        bytes.extend_from_slice(&checksum.to_le_bytes());
        bytes
    }
}
