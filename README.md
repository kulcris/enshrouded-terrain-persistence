# Terrain Persistence

Experimental server-only Enshrouded mod that journals terrain edits so they can be restored after a server restart. Requires Enshwick alpha 11, installed separately. Restart tests passed inside a real claim and on actual player edits outside claims. The player confirmed that the oversized-hole defect is fixed and intended cuts retain their correct size.

## Game update compatibility

Version 0.3.0 finds the terrain commit by a unique code signature instead of a fixed address or game-version list. Enshwick resolves the remaining engine APIs through its own tables and pattern fallback. There is no exact-build restriction in the mod manifest.

The signature was checked against server builds `26083076` (PE age `399`) and `26319703` (PE age `479`); runtime testing uses the latter. Future updates that retain the required native code and layouts can work without rebuilding this mod. An engine or loader API change can still require an update: missing or ambiguous signatures disable the mod. Check the Enshwick logs after game updates before relying on persistence. Keep matching savegame and journal backups.

## Install

Put the files in the dedicated server installation using this layout:

    <server>/
      powrprof.dll
      mods/
        enshwick/
          enshwick.dll
        terrain_persistence.dll

Install the [Enshwick alpha 11 loader](https://github.com/enshwick/enshwick/releases) separately, following its installation instructions. This mod's release ZIP contains only `mods/terrain_persistence.dll`, documentation, a config example and a checksum manifest; no loader files. Back up the server save before first use.

## Ore and clay respawn configuration

The loader creates `mod-configs/kulcr.terrain-persistence.toml` on first start:

```toml
respawn_ores = true
ore_respawn_seconds = 1800
extra_ore_materials = []
```

Restart the server after changing these settings. The default delay is 30 minutes. Mining an ore or clay voxel starts a timer for its 4-metre brick; further mining in that brick restarts its timer. Only the original ore/clay voxel values are restored, preserving surrounding terrain edits and player building blocks. Materials are identified from the running game's yielded-item names containing an `Ore` or `Clay` token, including iron, copper, tin, silver, gold and celesthium. Additional mineral resources can be included with `extra_ore_materials`; the running material names and IDs are listed in `mod-data/kulcr.terrain-persistence/ore-materials.txt`.

Timers and original ore shapes are saved in each world's `ore-respawn` directory. Offline time counts toward the deadline. Set `respawn_ores = false` to keep ore edits persistent; saved timers remain available if re-enabled, and overdue timers then run. Changing the delay affects new mining, not already saved deadlines. This tracks edits made after installing this version; it cannot reconstruct ores mined before tracking began. Back up the whole world mod-data directory along with the savegame.

Loot boxes retain the game's existing behavior; this mod adds no loot-box handling. The terrain capture hooks require native code; the installed Enshwick Luau API does not expose them, so this remains a DLL mod.

## Terrain journal

Journal files are stored alongside the server data at:

    mod-data/kulcr.terrain-persistence/worlds/<world-key>/terrain-delta/*.brick

The journal stores sparse changes to air and terrain in 8×8×8 voxel bricks, including edits inside and outside Flame Altar claims. Unchanged voxels are omitted, so natural terrain is not captured as a full snapshot. The older `terrain` journal namespace is intentionally ignored because it may contain unsafe full snapshots. The mod does not persist general entities, chests, trees, or other non-voxel world objects. Buildings are preserved during replay.

On startup, saved bricks are restored incrementally, loading padded neighboring terrain columns at all seven LOD levels before replay to avoid collateral terrain erasure, up to eight bricks per 100 ms tick. Loaded world chunks also trigger restoration. Worlds with many saved bricks can take longer to finish restoring. Captured edits are drained at up to 32 bricks per 100 ms tick and written to the journal; a normal shutdown also flushes pending captures. A crash can lose captures still waiting in memory.

In a frozen-fixture validation, actual player edits changed 311 voxels across 10 bricks. After a cold restart, all 262,144 finest-level cells and all seven LOD levels had zero differences. Nine unit tests also passed. A separate copy of a real claimed world also passed: a native terrain addition inside its Flame Altar claim was captured in eight wholly claimed bricks, then the surrounding 262,144 finest-level cells and all seven LOD levels matched exactly after restart. The player also confirmed that the unintended oversized hole is gone and the intended cuts retain their correct size.

Back up the journal together with the matching savegame. The journal is stored under a per-world key; if a world is migrated or its key changes, check that the journal still belongs to that save before copying or reusing it.

Removing this mod stops its persistence. Keep the journal if you may reinstall it; without the mod, terrain outside claims is left to vanilla save behavior and may reset.

## Build

The Enshwick dependency is pinned to Git commit `e0c8a0bc0e9571860ca9e76ac99a8db4a66e5172`; `Cargo.lock` pins its dependencies. On Windows with Rust 1.99.1 and its GNU toolchain:

```powershell
rustup toolchain install 1.99.1-x86_64-pc-windows-gnu --profile minimal
$env:RUSTFLAGS = '-C linker=rust-lld -C linker-flavor=ld.lld -C link-self-contained=yes'
cargo +1.99.1-x86_64-pc-windows-gnu test --lib --locked
cargo +1.99.1-x86_64-pc-windows-gnu build --release --locked
./Package-Mod.ps1 -SkipBuild
```

The release build excludes disposable-server test commands. The optional `runtime-test` feature enables commands used by `Invoke-TestCommand.ps1` on an isolated test server. Game binaries and saves are not included. `tools-build.ps1` is a convenience wrapper for the isolated development toolchain, not needed by CI.

GitHub Actions runs tests and creates a mod-only ZIP and SHA-256 checksum for pushes, pull requests and manual runs. Push a tag matching the Cargo version (for example `v0.4.0`) to publish the GitHub-built files as release assets. The workflow does not download or package the loader or game.
