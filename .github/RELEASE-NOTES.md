**Compatibility update: bundled Enshwick Alpha 13.**

This release updates the mod for Alpha 13 and includes that exact loader version, so Alpha 11 is no longer required. It also uses a compact database with batched terrain writes and indexed ore/clay timers.

### Installation and upgrading

Stop your server and back up the save and mod data first. Extract the entire ZIP into your dedicated-server folder, replacing the included files. If upgrading from v0.4.x, remove the old loose `mods/terrain_persistence.dll`; the new file is `mods/terrain-persistence/terrain_persistence.dll`.

Keep the old `mod-data` and `mod-configs` folders for automatic import. Settings now live beside each save, for example `savegame/world0-mod-settings/kulcr.terrain-persistence/settings.toml`. Original legacy records remain available for rollback. Check Alpha 13 compatibility for any other installed mods before replacing a shared loader.

**Experimental — expect bugs.** This mod is "vibe coded" with AI assistance and has not been tested with extensive terrain changes or heavily modified worlds.

[Installation, settings and backups](https://github.com/kulcris/enshrouded-terrain-persistence#install-or-update)
