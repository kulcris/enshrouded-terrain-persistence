# Enshrouded Terrain Persistence

Keep your terrain changes after a server restart—even outside Flame Altar claims. Includes configurable respawn timers for ore and clay, with optional timers for world loot containers and wild resource nodes.

**Windows dedicated servers. Players do not need to install the mod.**

**Experimental — expect bugs.** This mod is "vibe coded" with AI assistance and has not been tested with extensive terrain changes or heavily modified worlds. Back up your world before using it.

[Download the latest release](https://github.com/kulcris/enshrouded-terrain-persistence/releases/latest)

## Install or update

1. Stop the server. Back up the save folder and existing mod data/settings.
2. Extract the release ZIP into the folder containing `enshrouded_server.exe`, merging folders and replacing the included files.
3. If upgrading from v0.4.x, remove the old `mods/terrain_persistence.dll` file. The new mod is in its own folder as shown below. Keep your old `mod-data` and `mod-configs` folders so their journals, timers and settings can be imported.
4. Start the server.

**Enshwick Alpha 13 is included.** Use the included loader with this version of the mod. If you use other mods, check that they also support Alpha 13 before replacing their loader.

```text
<server folder>/powrprof.dll
<server folder>/mods/enshwick/enshwick.dll
<server folder>/mods/terrain-persistence/terrain_persistence.dll
```

The mod records new terrain edits once installed. It cannot recover changes that were already lost.

## Ore and clay respawns

Mined ore and clay return after **30 minutes** by default. Nearby mining shares a timer; mining again in that area restarts the countdown. Other terrain edits stay saved, and player building blocks are preserved.

After the first start, edit the settings beside your world save. For the default world and save folder:

```text
savegame/world0-mod-settings/kulcr.terrain-persistence/settings.toml
```

```toml
respawn_ores = true
ore_respawn_seconds = 1800
extra_ore_materials = []
respawn_chests = false
chest_respawn_seconds = 1800
respawn_lootables = false
lootable_respawn_seconds = 1800
```

| Setting | What it does |
| --- | --- |
| `respawn_ores` | Set to `false` to keep mined ore and clay removed. |
| `ore_respawn_seconds` | Delay in seconds: `600` = 10 minutes, `1800` = 30 minutes, `3600` = 1 hour. |
| `extra_ore_materials` | Optional additional resource material IDs. Leave empty for the default selection. |
| `respawn_chests` | Enable timers for supported world loot containers. Disabled by default. |
| `chest_respawn_seconds` | Delay after a container is observed empty. |
| `respawn_lootables` | Enable timers for placed world pickups, including bandages, and supported wild resource nodes outside claims. Disabled by default. |
| `lootable_respawn_seconds` | Delay after collecting a pickup or harvesting a supported node. |

**Restart after changing settings.** The default selection includes copper, tin, iron, silver, gold, celesthium and clay. Additional material IDs are listed in `ore-materials.txt` in this world's mod-data folder.

Timers survive restarts, including offline time. Changing the delay affects new timers. Disabling respawns keeps existing timers saved; overdue respawns run when re-enabled.

## World loot and wild resource timers

Set `respawn_chests` and/or `respawn_lootables` to `true` to enable these experimental features. Each has its own delay; `600` is ten minutes and `3600` is one hour. Restart after changing settings.

Only containers and nodes used or harvested while the feature is running are tracked. The mod asks the game to generate loot from its own templates; it does not copy a player's deposited items or restore a saved inventory. It checks for an existing object before spawning a replacement. Player storage, dropped bags, corpses, enemies, and resource nodes inside Flame Altar claims are excluded.

Overdue replacements wait until a player is nearby and the area has had time to load. Allow about 15–30 extra seconds near the spot. Normal game resets and natural regrowth still apply and can happen before a custom timer. These timers do not prevent vanilla respawns. Not every quest object or world pickup is a supported resource node.

## Saved data and backups

Terrain edits inside and outside claims are preserved. Enemy respawning and unsupported world objects keep their normal game behavior.

Terrain, ore timers and world-loot/node timers share a compact database. For the default world:

```text
savegame/world0-mod-data/kulcr.terrain-persistence/terrain-delta/records.redb
```

Existing journals are imported automatically. Original legacy files are retained as rollback copies; they are no longer updated. The database has a fixed size overhead, so very small journals may use more space, while larger sparse journals benefit from compact records and fewer active files.

Stop the server before backing up or moving the save and its matching `world0-mod-data` and `world0-mod-settings` folders. Replace `world0` with your world's name, and use your configured save folder if it differs. Keep the old `mod-data` and `mod-configs` backups until you have checked the upgraded world.

Large edited areas can take time to restore at startup. Use a normal shutdown to save pending edits. Game or loader updates can require a mod update. Diagnostics are in the timestamped `logs/kulcr.terrain-persistence_*.log` and `logs/enshwick_*.log` files.

## Removing the mod

Stop the server and remove `mods/terrain-persistence`, then start it again. Keep your data backups if you might reinstall. Without the mod, terrain outside claims may reset according to the game's normal behavior. Other mods may still require the Enshwick loader.
