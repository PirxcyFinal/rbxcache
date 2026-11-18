# rbxcache

A viewer and editor for the asset cache the Roblox client keeps on disk
(`rbx-storage.db` and the `rbx-storage` folder). It lists every cached
asset, previews most of them, exports them, and can replace or delete them in
place. It works on the files directly: there is no proxy, no certificate and no
network traffic to intercept.

This is the safest way to do what [Fleasion](https://github.com/fleasion/Fleasion)
does. Fleasion swaps assets by routing Roblox's traffic through a local proxy:
it adds its own certificate to the client's trusted list, relaunches the client
through the proxy, and reads your Roblox session cookie, on its own, from
Roblox's own files or your browser's, to fetch assets. rbxcache does none of
that. It never modifies the client or sits in its traffic, never asks for your
password, and never reads a cookie by itself. It only edits files Roblox has
already saved to disk, and it reads the same config format, with the
exceptions listed under [How asset IDs are matched](#how-asset-ids-are-matched).

Adding an account is optional and works the same way as Fleasion's manual
cookie import: you copy your own account's cookie out of your own browser and
paste it in, deliberately, rather than rbxcache finding it on its own. See
[Signing in](#signing-in), and read its warnings before adding one.

"Safest" is not "risk free". Changing how a game looks on your own machine may
still be against Roblox's rules or a game's, and that is yours to judge.

Switching a config off puts back what it replaced, from a copy rbxcache keeps
of each original. To undo everything at once, remove your configs and clear the
cache: Roblox downloads the originals again. Entries you replaced by hand are
protected, which means they are replaced again whenever Roblox rewrites them,
after a clear too; there is no command yet to stop protecting one, short of
deleting the `protections` folder in the data folder.

Windows, macOS and Linux. The cache is looked for in the client's own folder:

| Platform | Folder |
| --- | --- |
| Windows | `%LOCALAPPDATA%\Roblox` |
| macOS | `~/Library/Roblox` |
| Linux (Sober) | `~/.var/app/org.vinegarhq.Sober/data/sober/appData` |

Pass `--dir <folder>` to use another one. Development and testing so far have
been on Windows. Each release is built and unit-tested in CI on Apple silicon,
an Intel Mac and Linux, and on the two Macs the app is also opened from its
disk image; but the macOS and Linux builds have not been run against a real
cache.

It is lightweight by construction. Nothing runs inside Roblox and nothing sits
between it and its servers, so there is nothing to cost frames or add lag: a
replacement is a file swapped on disk, which Roblox loads the way it loads any
cached asset.

## Features

- Browse the whole cache in a sortable, searchable table, with a tab for each
  kind of asset the cache holds (images, textures, meshes, models, sounds,
  fonts, video, text). The table can be filtered further by content type or
  category.
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
  downloads it again. 3D models are the exception; see below.
- Apply replacement configs in the JSON format Fleasion uses, or pick a known
  pack from the list in the app. Each config and each rule in it has a switch;
  switching one off puts back what it replaced. Where two configs change the
  same thing, you choose which one has it.
- Follow the cache live while Roblox runs: newly fetched and rewritten entries appear as
  they are written, loaded configs are re-applied to them, and the tool
  reports whether Roblox read a replacement or wrote over it. Manual replacements
  are stored outside Roblox's cache and restored whenever Roblox overwrites them.

### Skies (Windows)

The Skies tab replaces the sky in every game with an animated one, by
rewriting the sky shaders in Roblox's shader pack. It covers stock Roblox,
Voidstrap, Bloxstrap and Fishstrap, and games that use an Atmosphere as well
as those that draw the classic sky, such as Rivals.

| Best looking | Work in progress |
| --- | --- |
| Neo Tokyo (pirxcy's recommended) | Rain, Snow, Storm |
| Aurora | Sakura |
| Synthwave | Shooting Stars |
| Blood Moon | Lantern Festival, Sunset Ocean |

A GIF of your own can be used as the sky too. Applying a sky closes Roblox
if it is open; start it again and join a game with a visible sky. Turn off
puts Roblox's own shaders back. Skies are experimental: they add shader work
to every frame, work best on NVIDIA GPUs and can lower the frame rate. The
first time a sky is applied its shaders are compiled, which takes up to about
a minute and a half for Neo Tokyo; after that it is kept and applies at once.

### No 3D model swapping or downloading

rbxcache does not swap 3D models and does not download them. It will not
write over a mesh, a model, a place, a package, an accessory, a body part or
the collision data Roblox keeps for them, and it will not write any of those
into another entry, whether they come from the cache, a file, a link, a pack
or a config. FBX, glTF, OBJ, STL, PLY and Draco files are refused the same
way. Asset IDs that turn out to be 3D are not downloaded: a lookup says so
instead. Config rules leave such entries alone and say how many. Animations
are stored like models and are still allowed when that is all a file holds.

Meshes and models can still be browsed, previewed and exported from your own
cache.

## Getting it

Download the file for your platform from the releases page. Each name carries
the version, shown here as `<version>`:

| Platform | File |
| --- | --- |
| Windows | `rbxcache-<version>-windows-x86_64.exe` |
| macOS, Apple silicon (M1 and later) | `rbxcache-<version>-macos-arm64.dmg` |
| macOS, Intel | `rbxcache-<version>-macos-x86_64.dmg` |
| Linux | `rbxcache-<version>-linux-x86_64` |

On macOS, open the disk image and drag rbxcache to Applications. The app is
signed ad hoc, not with an Apple developer certificate, so macOS refuses to
open it the first time. Either run
`xattr -dr com.apple.quarantine /Applications/rbxcache.app`, or try to open it,
then go to System Settings > Privacy & Security and press Open Anyway (on
macOS 14 and earlier, right-clicking the app and choosing Open also works).
For the command line, the binary is `rbxcache.app/Contents/MacOS/rbxcache`.

On Linux, make the file executable first with `chmod +x`.

Or build it:

```
cargo build --release
```

The binary is `target/release/rbxcache` (`rbxcache.exe` on Windows). Building
needs a Rust toolchain, plus the MSVC build tools on Windows or the ALSA and
GTK 3 development packages on Linux (`libasound2-dev libgtk-3-dev`).

## Using it

Run `rbxcache` with no arguments to open the window. The sidebar has two
halves. Browse lists what is in the cache, with a row of tabs for everything,
for what Roblox has just written (Live), and for each kind of asset. Manage
holds Configs, Packs, a Guide and Info. In a list, select a row to see its
details and preview on the right; right-click a row for the entry menu.

Only one window runs at a time for each user of the computer. Starting
rbxcache again brings up the one that is already running, from the tray if
that is where it is.

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
| F1 | The Info page |

An asset Roblox has already loaded stays in its memory, so a replacement shows
up the next time the asset is loaded: after rejoining, or after restarting the
client. The window shows a warning while the client is running.

### Watching the cache

The window follows the cache while it is open; the Live menu turns that on and
off. The Live tab starts empty and fills with what Roblox writes from then on,
newest first, and the Live menu can switch to it, clear it, or sort it newest
first again. In every list, entries Roblox has just written are marked with a
dot that fades, and a replaced entry shows whether Roblox has read it since.

Protected entries are monitored and restored indefinitely, including after
`rbxcache` restarts. Durable replacement bodies and metadata are stored in
`%LOCALAPPDATA%\rbxcache` on Windows, `~/Library/Application Support/rbxcache`
on macOS, and `$XDG_DATA_HOME/rbxcache` (or `~/.local/share/rbxcache`) on Linux.

### Using a config, step by step

This is what most people use rbxcache for, and the Guide tab in the app says
the same thing.

1. **Get a config.** A config is a file of replacement rules: swap this asset
   for that file, remove that one. The Packs tab lists ready-made ones, and any
   config made for Fleasion works too.
2. **Add it.** Press Apply on a pack, or Add configs on the Configs tab (or
   Ctrl+O) for a file of your own; several files can be added at once.
   rbxcache looks up the asset IDs the config names and shows what it would
   change. Nothing is written until you press Apply in that dialog; cancelling
   leaves things as they were.
3. **Play.** Leave rbxcache open, or close it and let it sit in the tray.
   Roblox only caches an asset when a game first needs it, so most of a pack
   has nothing to replace until you have joined the game. Each asset is
   replaced as it arrives, for as long as Live > Watch cache live is on.
4. **Rejoin.** Roblox keeps what it has already loaded in memory, so a
   replacement shows the next time the asset is loaded. The first time with a
   pack: join, leave, join again.

### Configs

The Configs tab lists the loaded configs. They are remembered between runs.

- Each config has a switch. On, its rules are applied to the cache and to
  whatever Roblox writes later. Off, what it replaced is put back. Remove does
  the same and takes it out of the list.
- Under Rules, each rule of a config has its own switch, which works the same
  way for that rule alone.
- When two enabled configs change the same thing, a Conflicts box lists what
  they share. Pick a config for all of it, or go through it one by one. Until
  you choose, the config lower in the list has it. Adding a config that is
  already loaded moves it to the bottom.
- Each config shows how many of its asset IDs were identified and how many
  Roblox would not hand out.
- A config whose file cannot be read is kept in the list, marked unreadable,
  until the file is back or you remove it.
- Reload from files reads the config files again, which picks up edits made
  to them, and shows the dialog again.

Putting things back works from a copy: before a rule replaces an entry,
rbxcache saves what the entry held in an `originals` folder in its data
folder. If Roblox has downloaded the asset again in the meantime, there is
nothing to put back and the copy is dropped. Replacements made by the
`apply-config` command are not tracked this way.

To copy one cached asset over another, right-click the first and choose "Use as
replacement source", then right-click the target and choose "Replace with
source". Replacing a texture by hand, like replacing it with a rule, writes
every pack of that texture that is cached at that moment; a rule also catches
the packs Roblox downloads later, a replacement by hand does not.

### Packs

The Packs tab lists known packs. The list is [`packs.json`](packs.json) in
this repository, fetched when the app starts; the copy built into the app is
shown when it cannot be reached. Apply downloads the pack's config into the
data folder and loads it like any other config, with the same dialog first.
A pack's config may download files over https but may not name files on your
computer. `rbxcache packs` prints the list and `rbxcache packs --apply <id>`
applies one.

The packs listed change sounds as well as images, and most of them change the
font. The sound rules do nothing as shipped, for the reason given under
[How asset IDs are matched](#how-asset-ids-are-matched).

To add a pack, put its config under `packs/` and add an entry to `packs.json`:

```json
{ "id": "my-pack", "name": "My Pack", "author": "me", "game": "RIVALS",
  "description": "What it changes.", "config": "packs/my-pack.json" }
```

`config` may also be an `https` address. `packs.json` can carry fingerprints
for asset IDs as well; see [How asset IDs are matched](#how-asset-ids-are-matched).

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

### Signing in

Adding an account is optional, and it is how sounds, which Roblox only hands
to signed-in players, can be fetched and matched to your cache automatically.
Everything else works without it. What needs it is locked until an account is
added: the IDs tab, `rbxcache fetch`, and the rules in a config that name
assets Roblox keeps from guests, mostly sounds. Adding one asks about those
again, and the Configs page shows how many are waiting.

**This can be a main account. An alternate account is recommended, and never
paste anyone else's.** A throwaway account works exactly as well as your main
one: it only has to exist and be able to sign in. Whoever holds the cookie you
paste in can act as that account, including spending its Robux, for as long as
the cookie is valid, so an alt with nothing of value on it is the better choice.

Nothing is sent until that warning has been on screen for 5 seconds. In the
window the Add button stays disabled and counts down; from a terminal,
`rbxcache login` waits the same 5 seconds and Ctrl+C stops it.

rbxcache has no Roblox login page of its own, and reads no cookie by itself.
Open Manage > Account (or run `rbxcache login`) and paste in the account's
`.ROBLOSECURITY` cookie, which you copy yourself from that account's own,
already signed-in browser session (in most browsers: developer tools (F12) >
Application or Storage > Cookies > `https://www.roblox.com` > `.ROBLOSECURITY`
> copy its value). rbxcache checks it with Roblox and shows who it belongs to
before adding it.

- Several accounts can be added; one is in use at a time. Its name and avatar
  show at the bottom right of the window.
- The cookie is kept encrypted for your Windows user (DPAPI) in
  `accounts.json` in the data folder, so the file is no use on another user or
  computer.
- Remove deletes an account from this computer at once, with one click
  (`rbxcache logout` from a terminal). This does not end the account's Roblox
  session: a cookie cannot be revoked the way a login token can. To be sure a
  pasted cookie can never be used again, sign that account out on roblox.com.

Once an account is in use, every asset request is sent with its cookie, so
Roblox answers as that account, which gets anything the account itself can
reach. When Roblox still refuses, the request is sent once more naming the game
you last joined, with the `Roblox-Place-Id` header the real client sends for
the game it is in. That is as far as it goes: rbxcache does not hunt for a game
that will unlock an asset the account was denied.

Most game sounds stay refused this way, because Roblox keeps them to the game
that uses them and hands them only to a client actually in that game. There is
no way to pull those bytes from Roblox after the fact, so they are matched by
ear instead: the bytes are already in your cache from playing the game, and the
IDs tab lists the cached sounds of the right length for you to play and pick
(see [Looking an ID up](#looking-an-id-up)). Pick once, and
[Copy fingerprints](#looking-an-id-up) lets you publish the match so no one
else has to.

Signing in is only built for Windows so far; on other platforms the by-ear
matching is the way to get sounds working.

### Looking an ID up

This needs a signed-in account (see [Signing in](#signing-in)). Without one the
IDs tab is locked, and `rbxcache fetch` says so.

The IDs tab under Manage takes asset IDs: numbers, links or `rbxassetid://`
addresses, one or several. For each it shows what Roblox says in public about
the asset (name, type, creator) and then one of three things:

- Roblox hands the file out. It is shown, or can be played, and saved with
  Save as. If the same content is in your cache the ID is tied to that entry;
  if not, it will be recognised when Roblox downloads it.
- Roblox refuses, and the asset is a sound. The cached sounds within a second
  of its length, and not yet tied to an ID, are listed. Play them and press
  This is it on the right one. Configs that name the ID work from then on,
  and are applied straight away.
- Roblox refuses, and it is something else. It can still be tagged by hand
  from Browse.

Copy fingerprints puts what is known about the listed IDs on the clipboard, in
the form `packs.json` takes under `"fingerprints"`, so that what one person
picked by ear works for everyone who uses the pack list.

`rbxcache fetch <id>...` does the lookup from a terminal (`-o <folder>` saves
the files), and `rbxcache gui --fetch <id>...` opens the window on the result.

### Updates

On start the window asks GitHub once whether a newer release exists. If so, an
"Update to ..." button appears in the sidebar, and the Info page lists what
changed in every version after yours, with a button for the release page. Help >
Check for updates asks again, and `rbxcache update` does the same from a
terminal. Nothing is downloaded or installed automatically.

The changes come from [`changelog.json`](changelog.json), which has an entry
for every version. The first time a new version is started, a window lists
what changed since the one used before. The Info page and `rbxcache changelog`
show the same list.

### Command line

The same binary takes commands. Entry IDs can be shortened to any unique prefix.

```
rbxcache gui --tab packs               open the window on a tab
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
rbxcache packs [--apply <id>]          list the known packs, or apply one
rbxcache share <file>...               print the fingerprints known for a config's IDs
rbxcache fetch <id>... [-o dir]        look asset IDs up, saving what Roblox hands out
rbxcache login [cookie]                add an account by its .ROBLOSECURITY cookie
rbxcache logout [user]                 remove an account from this computer
rbxcache accounts                      list the added accounts
rbxcache changelog                     what changed in each version
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
cached, so its fingerprint (MD5 and size) identifies the entry.

Images are different. The client does not cache an image as one file: it asks
for KTX2 "packs", the texture cut into groups of mip levels, smallest first,
and fetches the larger ones as the texture gets closer. Each pack is its own
cache entry, and all packs of one texture carry the same content hash in their
metadata. The lookup fetches the first pack, reads that hash, and from then on
every cached pack with the hash counts as that asset. A replacement is written
into each pack at the pack's own size, block-compressed (BC1 or BC3) and
Zstandard-wrapped like the original, with the original's metadata kept. A pack
with no alpha channel becomes BC3 when the replacement is transparent.

The fingerprints are kept in an ID map (`%APPDATA%\rbxcache\idmap.json` on
Windows, `idmap.json` in the data folder above elsewhere), whether or not the
asset is in the cache yet. With the config still active, an asset Roblox
downloads later is recognised and replaced when it arrives. The map does not
depend on the cache key, so it survives a cache clear and can be shared.

Not every asset is public. Roblox answers 401 or 403 for those that need a
login or a permission, and rules naming them are reported and skipped, unless
you have signed in (see [Signing in](#signing-in)). As a guest, in DETERMINATION and Mirc's 5k Pack, that is
every sound and some of the images: 48 of their 126 asset IDs when this was
written. So a
pack's sound replacements do nothing until those IDs are known some other way:

- Pick it by ear on the IDs tab (see [Looking an ID up](#looking-an-id-up)).
  Roblox does say in public how long a sound is, to the second, so the app
  lists the cached sounds of about that length; you play them and pick the one
  it is. Identify them, under a config on the Configs tab, queues up every ID
  of that config that Roblox would not hand out.
- Tie the asset to an entry by hand: select the entry under Browse, type the
  asset ID and press Tag (or use `rbxcache tag`).
- Share what was tagged. A fingerprint describes content, not a machine, so it
  works for everyone. `rbxcache share <config>` prints the fingerprints known
  for a config's IDs; put under `"fingerprints"` in `packs.json`, they are
  merged into every user's ID map when the app fetches the list. None are
  published there yet.

`rbxcache identify <config>` shows how each ID in a config fared, and
`apply-config --offline` skips the lookup.

Each ID is looked up once. The map remembers what it learned, including which
IDs were refused, so applying the same or another config later only asks about
IDs it has not seen. `rbxcache identify <config> --refresh` asks again, for an
asset that was updated or made public since.

A replacement asset named by `with_id` is taken from the cache when it is
there and downloaded otherwise; for an image the original is downloaded when
Roblox gives it out, since a cached pack is only part of the picture. Files a
config downloads are fetched once while the app runs, and a dry run downloads
them too, to tell what would change.

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

`headerSum` and `bodySum` are XXH32 with a fixed seed over the response
headers and the body, and zero when those are empty. `rbxcache probe` checks
this against every entry.

## Limitations

- Whether a replacement takes effect in game has only been checked as far as
  the cache goes: the entry is rewritten with a valid checksum, and rewritten
  texture packs decode with the reference Zstandard and BC decoders. Use the
  Live tab to see whether Roblox reads a replacement or writes over it.
- Sounds, and anything else Roblox will not give out without a login, are not
  matched by ID unless tagged by hand or covered by shared fingerprints.
- The rewritten texture packs have not been checked in a running client.
- A transparent replacement for a texture with no alpha channel is written as
  BC3, which is twice the size of the BC1 pack it replaces.
- Texture packs in a format other than BC1 or BC3 are rewritten as BC3.
- A `remove` rule writes a transparent picture into an image, and an empty
  payload into anything else, rather than a valid empty asset of that kind.
- Curve animations (`CurveAnimation`) and animations for custom rigs are shown
  as a tree only. The R15 rig uses the proportions of the default blocky
  character.
- Meshes are drawn untextured. Video segments and solid-model data have no
  viewer.

## Credits

The wordmark on the loading screen is set in Manrope ExtraBold, used under the
SIL Open Font License (`assets/Manrope-OFL.txt`).

The Japanese words on the Neo Tokyo sky's signs are drawn from M PLUS Rounded 1c
ExtraBold (Copyright 2016 The Rounded M+ Project Authors), and the names of the
made-up brands on its adverts from Pacifico (Copyright 2018 The Pacifico Project
Authors), Anton (Copyright 2020 The Anton Project Authors), Orbitron (Copyright 2018
The Orbitron Project Authors), Bungee (Copyright 2023 The Bungee Project Authors),
Lobster (Copyright 2010 The Lobster Project Authors), Bangers (Copyright 2010 The
Bangers Project Authors) and Righteous (Copyright 2011 Brian J. Bonislawsky, Astigmatic),
all used under the SIL Open Font License. Only
distance fields of the characters it needs are kept, in
`src/skies/hlsl/neotokyo_glyphs.hlsl`, made by `skies/neotokyo_glyphs.py`; the fonts
themselves are not included. The brands (Kira Cola, ZAPP, HOSHI, KURO Coffee, Sora
Beer, BIG BUN, ONPU) are invented.

## License

rbxcache is released under the MIT License; see [LICENSE](LICENSE). The fonts
named under Credits keep their own SIL Open Font License.
