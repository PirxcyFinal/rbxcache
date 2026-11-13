# rbxcache

A viewer and editor for the asset cache the Roblox client keeps on disk
(`%LOCALAPPDATA%\Roblox\rbx-storage` and `rbx-storage.db`). It lists every cached
asset, previews most of them, exports them, and can replace or delete them in
place. It works on the files directly: there is no proxy, no certificate and no
network traffic to intercept.

Windows only.

## Features

- Browse the whole cache in a sortable, searchable table, filtered by content
  type or category.
- Preview assets:
  - images (PNG, JPEG, WebP, GIF) and KTX2 textures (BC1/3/4/5/7, Zstandard)
  - meshes, versions 1 to 7 including Draco-compressed ones, in a 3D view
  - models (`rbxm`, `rbxmx`) as a collapsible instance tree with properties
  - keyframe animations played on an R6 or R15 rig, with the keyframes and
    poses still available as a tree
  - sounds (OGG, MP3, WAV) with seek and volume
  - fonts, JSON and other text
- Export an asset as it is, or converted: KTX2 to PNG, mesh to OBJ.
- Replace an asset's content with a file of your own, or delete it so Roblox
  downloads it again.
- Apply replacement configs in the JSON format Fleasion uses.
- Follow the cache live while Roblox runs: new and rewritten entries appear as
  they are written, an applied config is re-applied to them, and the tool
  reports whether Roblox read a replacement or wrote over it.

## Getting it

Download `rbxcache.exe` from the releases page, or build it:

```
cargo build --release
```

The binary is `target\release\rbxcache.exe`. Building needs a Rust toolchain and
the MSVC build tools.

## Using it

Run `rbxcache.exe` with no arguments to open the window. Select a row to see its
details and preview on the right; right-click a row for the entry menu.

| Keys | Action |
| --- | --- |
| Up / Down, Page Up / Page Down | Move through the list |
| Shift + arrows, Ctrl / Shift + click | Select several |
| Double-click | Open a copy in the default program |
| Space | Play or pause a sound or animation |
| Ctrl+F | Search |
| Ctrl+E | Export |
| Ctrl+O | Apply a config |
| Delete | Delete the selection |
| F5 | Reload the cache |

An asset Roblox has already loaded stays in its memory, so a replacement shows
up the next time the asset is loaded: after rejoining, or after restarting the
client. The window shows a warning while the client is running.

### Watching the cache

The window follows the cache while it is open (View > Watch cache live). Entries
Roblox has just written are marked with a dot that fades, and a replaced entry
shows whether Roblox has read it since. After a config is applied it stays
active: anything Roblox writes that matches a rule is replaced again, until you
choose File > Stop re-applying config. An entry Roblox overwrites three times is
left alone.

To copy one cached asset over another, right-click the first and choose "Use as
replacement source", then right-click the target and choose "Replace with
source".

### Command line

The same binary takes commands. Entry IDs can be shortened to any unique prefix.

```
rbxcache stats                         summary by category and type
rbxcache list --kind png --limit 20    list entries
rbxcache info <id>                     everything known about one entry
rbxcache export <id> [-o file]         write the payload to a file
rbxcache export <id> --convert         KTX2 to PNG, mesh to OBJ
rbxcache export-all <dir> --kind ogg   export every entry of one type
rbxcache render <id> [--time 1.5]      picture of a mesh or an animation frame
rbxcache replace <id> <file>           swap the payload for a file
rbxcache replace <id> --from <id2>     copy another entry's payload over it
rbxcache watch [--config <file>]       follow the cache, keeping a config applied
rbxcache delete <id>...                remove entries
rbxcache tag <id> <assetId>            record which asset an entry is
rbxcache apply-config <file>           apply a replacement config
rbxcache probe                         test-decode every texture, mesh and model
```

`--dir <folder>` points it at a different cache (a folder holding
`rbx-storage.db` and `rbx-storage`). `apply-config --dry-run` reports what would
change without writing.

## Replacement configs

A config is a JSON file of rules:

```json
{
  "replacement_rules": [
    { "name": "Quiet footsteps", "replace_ids": [123456789], "mode": "remove" },
    { "name": "My crosshair", "replace_ids": [987654321], "mode": "local", "local_path": "crosshair.png" },
    { "name": "Swap sword", "replace_ids": [111, 222], "with_id": 333 },
    { "name": "From a URL", "replace_ids": [444], "mode": "cdn", "cdn_url": "https://example.com/a.png" }
  ]
}
```

Rules may be nested in `{"type": "group", "children": [...]}` and switched off
with `"enabled": false`. A relative `local_path` is resolved against the
config's folder.

Each entry of `replace_ids` can be:

- a Roblox asset ID,
- a cache entry's 32-digit hex ID, or
- a type name (`Image`, `Audio`, `Mesh`, `Video`, `Font`) for every cached asset
  of that kind.

### Asset IDs need tagging

The cache does not record which asset ID an entry belongs to, and its entry
names cannot be derived from an ID. Fleasion learns the mapping by watching
requests through its proxy; this tool has no such view. Instead it keeps an ID
map at `%APPDATA%\rbxcache\idmap.json`: select an entry, type its asset ID and
press Tag (or use `rbxcache tag`). The map stores a fingerprint of the content
rather than the cache key, so it still matches after the cache is cleared and
can be shared between machines.

A rule whose asset ID is not in the map is reported and skipped. Fleasion's
texture-pack slot syntax (`"id:slot"`) and type names that cannot be recognised
from content, such as `Animation`, are not supported.

## Cache layout

Entries live in the `files` table of `rbx-storage.db`. Anything up to 7,000
bytes is stored inline in the `content` column; larger entries are files at
`rbx-storage\<first two hex digits>\<id>`. Asset bodies and cached redirects
are wrapped in an `RBXH` header:

```
"RBXH" | u32 version | u32 urlLen | url | u8 flag | u32 httpStatus
| u32 headerLen | u32 headerSum | u32 bodyLen | u32 bodySum | u32 reserved
| response headers | body
```

`headerSum` and `bodySum` are XXH32 with seed `0x00C4E564` over the response
headers and the body, and zero when those are empty. `rbxcache probe` checks
this against every entry.

## Limitations

- Whether a replacement takes effect in game has only been checked as far as
  the cache goes: the entry is rewritten with a valid checksum. Use the watch
  view to see whether Roblox reads it or writes over it.
- `remove` rules write an empty payload rather than a valid empty asset.
- Curve animations (`CurveAnimation`) and animations for custom rigs are shown
  as a tree only. The R15 rig uses the proportions of the default blocky
  character.
- Meshes are drawn untextured. Video segments and solid-model data have no
  viewer.
