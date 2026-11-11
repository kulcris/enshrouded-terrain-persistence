Adds optional, independent respawn timers for world loot chests and lootables such as placed bandages and supported wild resource nodes. Ore and clay timers remain separate.

The new features are disabled by default. In your world's `settings.toml`, enable `respawn_chests` and/or `respawn_lootables`, then set `chest_respawn_seconds` and `lootable_respawn_seconds`. Both default to 1800 seconds. Restart after changing settings.

Timers share the compact world database and survive restarts. Overdue replacements wait for a nearby player. Player storage, dropped bags, corpses, enemies and claimed resource nodes are excluded. Vanilla world resets still apply; custom timers do not delay them.

Enshwick Alpha 13 remains bundled. Stop the server and back up the save and its matching mod-data/settings folders before extracting the ZIP into the server folder.

**Experimental — expect bugs.** This mod is "vibe coded" with AI assistance and has not been tested with extensive terrain changes or heavily modified worlds.

[Installation, settings and backups](https://github.com/kulcris/enshrouded-terrain-persistence#install-or-update)
