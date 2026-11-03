# Enshrouded Terrain Persistence

Keep your terrain changes after a server restart—even outside Flame Altar claims. Includes a configurable respawn timer for ore and clay.

**For Windows dedicated servers. Players do not need to install the mod.**

[Download the latest release](https://github.com/kulcris/enshrouded-terrain-persistence/releases/latest)

## Install

1. Stop your server and back up its save folder and `mod-data` folder, if present.
2. Install [Enshwick alpha 11](https://github.com/enshwick/enshwick/releases) separately, following the loader's instructions.
3. Download `terrain-persistence-<version>-windows.zip` from the latest release.
4. Extract it and copy the `mods` folder into your server folder, beside `enshrouded_server.exe`. Merge it with the existing `mods` folder.
5. Start the server.

The mod file should be at:

```text
<server folder>/mods/terrain_persistence.dll
```

The loader is not included in the download. The mod starts recording new terrain edits once installed; it cannot recover changes that were already lost.

## Ore and clay respawns

By default, mined ore and clay return after **30 minutes**. Nearby mining shares a timer; mining again in that area restarts the countdown. Other terrain edits remain saved, and player building blocks are left intact.

Edit this file after the first server start:

```text
mod-configs/kulcr.terrain-persistence.toml
```

```toml
respawn_ores = true
ore_respawn_seconds = 1800
extra_ore_materials = []
```

| Setting | What it does |
| --- | --- |
| `respawn_ores` | Set to `false` to keep mined ore and clay removed. |
| `ore_respawn_seconds` | Respawn delay in seconds: `600` = 10 minutes, `1800` = 30 minutes, `3600` = 1 hour. |
| `extra_ore_materials` | Optional additional resource material IDs. Leave empty for the default ore and clay selection. |

**Restart the server after changing settings.** The default selection includes copper, tin, iron, silver, gold, celesthium and clay. For additional resources, material IDs are listed in `mod-data/kulcr.terrain-persistence/ore-materials.txt`.

Timers survive restarts, and time spent offline counts toward respawning. Changing the delay affects new timers. Disabling respawns keeps existing timers saved; re-enabling it allows overdue respawns to run.

## What is saved?

Terrain edits inside and outside claims are preserved. Loot boxes, trees, enemies and other world objects keep their normal game behavior.

Large edited areas may take a short time to restore after startup. Use a normal server shutdown so pending edits can be saved.

## Updating and backups

Stop the server before replacing `mods/terrain_persistence.dll` with a newer version. Keep your existing configuration and mod data.

Back up both the server's save folder and the entire `mod-data/kulcr.terrain-persistence` folder together. The mod data belongs to that world; keep it with the matching save when moving servers.

This mod is experimental. Game updates may require an update to the mod or loader. If it stops working, check `logs/enshwick/mods/kulcr.terrain-persistence.log` and the other Enshwick logs.

## Removing the mod

Stop the server, remove `mods/terrain_persistence.dll`, then start it again. Keep your mod-data backup if you might reinstall. Without the mod, terrain outside claims may reset according to the game's normal behavior.
