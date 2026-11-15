# rbxcache

A viewer and editor for the asset cache the Roblox client keeps on disk
(`rbx-storage.db` and the `rbx-storage` folder). It lists every cached
asset, previews most of them, exports them, and can replace or delete them in
place. It works on the files directly: there is no proxy, no certificate and no
network traffic to intercept.

This is the safest way to do what [Fleasion](https://github.com/fleasion/Fleasion)
does. Fleasion swaps assets by routing Roblox's traffic through a local proxy,
which means installing a certificate, relaunching the client and reading your
Roblox login. rbxcache does none of that: it never touches the client or its
traffic, never asks for or reads a login, and only edits files Roblox has
already saved to disk, which Roblox replaces by itself when the cache is
cleared. It reads the same config files Fleasion does.

"Safest" is not "risk free". Changing how a game looks on your own machine may
still be against Roblox's rules or a game's, and that is yours to judge.

Windows, macOS and Linux. The cache is looked for in the client's own folder:

| Platform | Folder |
| --- | --- |
| Windows | `%LOCALAPPDATA%\Roblox` |
| macOS | `~/Library/Roblox` |
| Linux (Sober) | `~/.var/app/org.vinegarhq.Sober/data/sober/appData` |

Pass `--dir <folder>` to use another one. Development and testing so far have
been on Windows; the macOS and Linux builds compile and pass the unit tests in
CI but have not been run against a real cache.

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
- Follow the cache live while Roblox runs: newly fetched and rewritten entries appear as
  they are written, an applied config is re-applied to them, and the tool
  reports whether Roblox read a replacement or wrote over it. Manual replacements
  are stored outside Roblox's cache and restored whenever Roblox overwrites them.

## Getting it

Download the file for your platform from the releases page
(`rbxcache-windows-x86_64.exe`, `rbxcache-macos-arm64`, `rbxcache-macos-x86_64`
or `rbxcache-linux-x86_64`). On macOS and Linux, make it executable first with
`chmod +x`. Or build it:

```
cargo build --release
```

The binary is `target/release/rbxcache` (`rbxcache.exe` on Windows). Building
needs a Rust toolchain, plus the MSVC build tools on Windows or the ALSA and
GTK 3 development packages on Linux (`libasound2-dev libgtk-3-dev`).

## Using it

Run `rbxcache` with no arguments to open the window. Select a row to see its
details and preview on the right; right-click a row for the entry menu.

| Keys | Action |
| --- | --- |
| Up / Down, Page Up / Page Down | Move through the list |
| Shift + arrows, Ctrl / Shift + click | Select several |
| Double-click | Open a copy in the default program |
| Space | Play or pause a sound or animation |
| Ctrl+F | Search |
| Ctrl+E | Export |
| Ctrl+O | Add configs |
| Delete | Delete the selection |
| F5 | Reload the cache |

An asset Roblox has already loaded stays in its memory, so a replacement shows
up the next time the asset is loaded: after rejoining, or after restarting the
client. The window shows a warning while the client is running.

### Watching the cache

The window follows the cache while it is open; the Live menu turns that on and
off. The Live tab starts empty and fills with what Roblox writes from then on,
newest first, and the Live menu can switch to it, clear it, or put it back in
arrival order. On either tab, entries Roblox has just written are marked with a
dot that fades, and a replaced entry shows whether Roblox has read it since.

Protected entries are monitored and restored indefinitely, including after
`rbxcache` restarts. Durable replacement bodies and metadata are stored in
`%LOCALAPPDATA%\rbxcache` on Windows, `~/Library/Application Support/rbxcache`
on macOS, and `$XDG_DATA_HOME/rbxcache` (or `~/.local/share/rbxcache`) on Linux.

### Loading configs

Configs > Add configs opens one or more config files. They are checked together
against the cache, and a dialog shows what they would change before anything is
written. Once applied they stay loaded and are listed in the Configs menu:

- A ticked config is re-applied to whatever Roblox writes from then on.
  Untick one to pause it; Remove takes it out of the list. Neither undoes
  what was already replaced. Delete those entries, or clear the cache, to get
  the originals back.
- Where two configs change the same entry, the one lower in the list wins.
- Apply enabled configs now reads the files again and applies them to the
  whole cache, which picks up edits made to them since.
- The list is remembered. When the app next starts, the configs are loaded
  again and re-applied to new arrivals; nothing already in the cache is
  touched until you apply them.

To copy one cached asset over another, right-click the first and choose "Use as
replacement source", then right-click the target and choose "Replace with
source".

### Staying in the tray

On Windows, closing the window keeps rbxcache running in the notification area,
so it goes on following the cache and re-applying replacements. Click the tray
icon to bring the window back; right-click it for Show window and Quit. File >
Hide to tray does the same as closing, File > Quit exits the app, and View >
Close to tray turns the behaviour off. That choice,
and the Animations and Watch cache live settings, are remembered in
`settings.json` in the data folder. macOS and Linux have no tray support yet;
there, closing the window quits.

### Clearing the cache

File > Clear cache empties the whole cache so Roblox downloads everything
again: it deletes the index and the blob folders, after closing Roblox if it is
running (the dialog says so before you confirm). Other files kept in the cache
folder are left alone. `rbxcache clear` does the same and refuses while Roblox
is running unless `--close-roblox` is given.

### Updates

On start the window asks GitHub once whether a newer release exists and, if so,
shows an "Update to ..." button that opens the release page. Help > Check for
updates asks again, and `rbxcache update` does the same from a terminal. Nothing
is downloaded or installed automatically. The check reads the public releases
of this repository, so it finds nothing while the repository is private.

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
rbxcache watch [--config <file>]...    follow the cache, keeping configs applied
rbxcache delete <id>...                remove entries
rbxcache tag <id> <assetId>            record which asset an entry is
rbxcache apply-config <file>...        apply one or more replacement configs
rbxcache identify <file>...            look up the asset IDs in configs
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

### How asset IDs are matched

The cache does not record which asset ID an entry belongs to, and its entry
names cannot be derived from an ID. When a config that names asset IDs is
applied, each ID is looked up at Roblox's asset delivery endpoint, the same one
the client uses, with no login. What comes back is byte-for-byte what the client
cached, so its fingerprint (MD5 and size) identifies the entry. For images the
client asks for a KTX2 rendition instead, so that is fetched too.

The fingerprints are kept in an ID map (`%APPDATA%\rbxcache\idmap.json` on
Windows, `idmap.json` in the data folder above elsewhere), whether or not the
asset is in the cache yet. With the config still active, an asset Roblox
downloads later is recognised and replaced when it arrives. The map does not
depend on the cache key, so it survives a cache clear and can be shared.

Not every asset is public. Roblox answers 401 or 403 for those that need a
login or a permission, and rules naming them are reported and skipped; this
tool never sends credentials. You can still tie such an asset to an entry by
hand: select the entry, type the asset ID and press Tag (or use `rbxcache tag`).
`rbxcache identify <config>` shows how each ID in a config fared, and
`apply-config --offline` skips the lookup.

Each ID is looked up once. The map remembers what it learned, including which
IDs were refused, so applying the same or another config later only asks about
IDs it has not seen. `rbxcache identify <config> --refresh` asks again, for an
asset that was updated or made public since.

A replacement asset named by `with_id` is taken from the cache when it is
there and downloaded otherwise. An image going into an entry Roblox stores as
KTX2 is converted to an uncompressed KTX2 texture first.

Fleasion's
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

## Credits

The wordmark on the loading screen is set in Manrope ExtraBold, used under the
SIL Open Font License (`assets/Manrope-OFL.txt`).
