//! The window: sidebar, entry table, detail panel with previews, the Configs,
//! Packs and Info pages, menus and dialogs.

use crate::account::{self, Accounts};
use crate::anim::Animation;
use crate::audio::{self, Audio};
use crate::changelog::{self, Version};
use crate::cli::{describe, export_name, matches};
use crate::config::{self, IdMap};
use crate::format::{self, human_size};
use crate::instance::{self, Claim};
use crate::lookup::{self, Found, Sound};
use crate::originals::Originals;
use crate::packs::{self, Catalog};
use crate::protection::Protections;
use crate::resolve;
use crate::settings::{SavedConfig, Settings};
use crate::store::{self, category_name, format_time_ms, Entry, Id, Store};
use crate::theme::{self, alpha};
use crate::tray::{Tray, TrayEvent};
use crate::update::{self, Release};
use crate::viewers;
use crate::watch::{self, Change, Tracker, Verdict, Watcher};
use anyhow::{anyhow, Context, Result};
use eframe::egui::{self, Color32, CornerRadius, Margin, Rect, Shape, Stroke, Vec2};
use egui_extras::{Column, TableBuilder};
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::net::TcpListener;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{channel, Receiver, TryRecvError};
use std::sync::Arc;

const ROW_HEIGHT: f32 = 24.0;
/// Side of the window and tray icon, in pixels.
const ICON_SIZE: u32 = 64;
/// Seconds a newly written entry keeps its marker in the table.
const FRESH_SECONDS: f32 = 10.0;
/// Seconds the detail panel takes to settle after the focused entry changes.
const PANEL_TIME: f32 = 0.32;
/// Width of the sidebar and height of one of its rows.
const NAV_WIDTH: f32 = 184.0;
const NAV_ROW: f32 = 48.0;
/// Widest a page (Configs, Packs, Info) lays its content out.
const PAGE_WIDTH: f32 = 820.0;

/// Content types that share a tab in the sidebar. A group is listed only
/// while the cache holds some of it; anything in no group goes under Other.
const GROUPS: [(&str, &[&str]); 8] = [
    ("Images", &["png", "jpeg", "gif", "webp"]),
    ("Textures", &["ktx2", "ktx"]),
    ("Meshes", &["mesh"]),
    ("Models", &["rbxm", "rbxmx"]),
    ("Sounds", &["ogg", "mp3", "wav"]),
    ("Fonts", &["ttf", "otf"]),
    ("Video", &["webm", "mp4"]),
    ("Text", &["json", "text", "asset-map"]),
];

/// Opens the window. `tab` names the tab to start on; see [`Tab::named`].
/// `fetch` is text holding asset IDs to look up on the IDs tab straight away.
pub fn run(dir: PathBuf, idmap_path: PathBuf, tab: Option<&str>, fetch: String) -> Result<()> {
    let tab = match tab {
        Some(name) => Tab::named(name).with_context(|| {
            format!("there is no tab '{name}'; try all, live, configs, packs, ids, account, guide, info or a kind such as textures")
        })?,
        None => Tab::All,
    };
    // A second copy would fight the first over the cache; it asks the first to come forward and leaves.
    let listener = match instance::claim() {
        Claim::First(listener) => Some(listener),
        Claim::Second => return Ok(()),
        Claim::Unknown => None,
    };
    let icon = theme::app_icon(ICON_SIZE);
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_inner_size([1400.0, 860.0]).with_min_inner_size([960.0, 560.0]).with_icon(egui::IconData {
            rgba: icon.clone(),
            width: ICON_SIZE,
            height: ICON_SIZE,
        }),
        ..Default::default()
    };
    eframe::run_native(
        "Roblox Cache Explorer",
        options,
        Box::new(move |cc| {
            let mut app = App::new(cc, dir, idmap_path, icon, tab, listener);
            if !fetch.trim().is_empty() {
                app.lookup_input = fetch;
                app.show_tab(Tab::Lookup, &cc.egui_ctx);
                // Without an account the IDs tab is locked, so there is nothing to look up yet.
                app.lookup_wanted = !app.accounts.is_empty();
            }
            Ok(Box::new(app))
        }),
    )
    .map_err(|e| anyhow!("{e}"))
}

/// What the window shows: one of the entry lists, or a page.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum Tab {
    /// Everything in the cache.
    All,
    /// Only what Roblox has written since the window opened or the list was cleared.
    Live,
    /// The entries of one of [`GROUPS`].
    Group(usize),
    /// The entries that belong to no group.
    Other,
    Configs,
    Packs,
    /// Looking asset IDs up one at a time.
    Lookup,
    /// Signing in with Roblox.
    Account,
    Guide,
    Info,
}

impl Tab {
    /// The tab a name given on the command line stands for.
    fn named(name: &str) -> Option<Self> {
        let name = name.trim().to_ascii_lowercase();
        let group = GROUPS.iter().position(|(group, _)| group.eq_ignore_ascii_case(&name)).map(Tab::Group);
        group.or(match name.as_str() {
            "all" => Some(Tab::All),
            "live" => Some(Tab::Live),
            "other" => Some(Tab::Other),
            "configs" => Some(Tab::Configs),
            "packs" => Some(Tab::Packs),
            "ids" => Some(Tab::Lookup),
            "account" => Some(Tab::Account),
            "guide" => Some(Tab::Guide),
            "info" => Some(Tab::Info),
            _ => None,
        })
    }

    /// True for the tabs that show a list of cache entries.
    fn lists(self) -> bool {
        !matches!(self, Tab::Configs | Tab::Packs | Tab::Lookup | Tab::Account | Tab::Guide | Tab::Info)
    }

    /// Whether an entry of this content type belongs on the tab.
    fn holds(self, kind: &str) -> bool {
        match self {
            Tab::Group(i) => GROUPS[i].1.contains(&kind),
            Tab::Other => kind != "redirect" && !GROUPS.iter().any(|(_, kinds)| kinds.contains(&kind)),
            _ => true,
        }
    }
}

#[derive(Clone, Copy, PartialEq)]
enum SortKey {
    /// Order of arrival on the Live tab, newest first unless ascending.
    Arrival,
    Kind,
    Category,
    Size,
    Hits,
    Used,
    Id,
}

const COLUMNS: [(&str, SortKey); 6] = [
    ("Type", SortKey::Kind),
    ("Category", SortKey::Category),
    ("Size", SortKey::Size),
    ("Hits", SortKey::Hits),
    ("Last used", SortKey::Used),
    ("ID", SortKey::Id),
];

enum LoadMsg {
    Progress(usize, usize),
    Done(Result<Vec<Entry>, String>),
}

#[derive(Clone)]
enum Action {
    Reload,
    OpenFolder,
    Export,
    ExportMany,
    Replace,
    OpenExternally,
    AskDelete,
    AskClear,
    Clear,
    Delete,
    AddConfigs,
    ReapplyConfigs,
    ConfirmConfigs,
    CancelConfigs,
    ToggleConfig(usize),
    /// Switches one rule of a config on or off: the config, then the rule.
    ToggleRule(usize, usize),
    /// Gives these targets to one of the configs that set them.
    Choose(Vec<String>, usize),
    RemoveConfig(usize),
    RemoveAllConfigs,
    Tag,
    CopyId,
    SortBy(SortKey),
    ShowTab(Tab),
    ClearLive,
    SortByArrival,
    SetSource,
    ReplaceFromSource,
    PlayAudio,
    /// Looks up the asset IDs typed on the IDs tab.
    RunLookup,
    /// Opens the warning that has to be read before a cookie is sent.
    AskAddAccount,
    /// Checks a pasted cookie and, if it is one, adds the account.
    AddAccount,
    /// Removes an account from this computer, by user ID. Does not end its Roblox session.
    SignOut(u64),
    /// Makes an account the one used for requests.
    UseAccount(u64),
    /// Looks up the IDs of this config that Roblox would not hand out.
    LookupFor(usize),
    /// Plays a cached sound, or the file fetched for the lookup at this place in the list.
    PlaySound(Id),
    PlayFetched(usize),
    SaveFetched(usize),
    /// Ties an asset ID to the cached entry the user picked for it.
    Pick(u64, Id),
    /// Opens the All list on this entry.
    ShowEntry(Id),
    CopyFingerprints,
    ForcePreview,
    ExportConverted,
    RefreshPacks,
    ApplyPack(usize),
    CheckUpdate,
    OpenUpdate,
    Quit,
    HideToTray,
}

struct Preview {
    id: Id,
    facts: Vec<(&'static str, String)>,
    texture: Option<egui::TextureHandle>,
    mesh: Option<MeshView>,
    anim: Option<AnimView>,
    model: Option<viewers::Model>,
    /// The family a cached font was registered under, and the pass it was added in.
    font: Option<(egui::FontFamily, u64)>,
    audio: bool,
    /// Too large to preview without being asked.
    deferred: bool,
    text: Option<String>,
    hex: String,
    error: Option<String>,
}

/// Camera and cached picture for a 3D view the user can turn and zoom.
struct Orbit {
    yaw: f32,
    pitch: f32,
    zoom: f32,
    texture: Option<egui::TextureHandle>,
    /// What the texture was last drawn for: camera, pixel size and a caller-chosen stamp.
    drawn: Option<(f32, f32, f32, usize, usize, f32)>,
}

struct MeshView {
    mesh: viewers::Mesh,
    bounds: ([f32; 3], f32),
    orbit: Orbit,
    spin: bool,
}

struct AnimView {
    orbit: Orbit,
    /// Framing that keeps every pose of the animation in view.
    bounds: ([f32; 3], f32),
    time: f32,
    playing: bool,
    speed: f32,
    looping: bool,
}

/// Configs that have been checked against the cache together and await confirmation.
struct PendingConfigs {
    configs: Vec<LoadedConfig>,
    summary: String,
    /// How looking up the configs' asset IDs went; empty when they name none.
    lookup: String,
    notes: Vec<String>,
}

/// A config that is loaded and, while enabled and the cache is watched, re-applied to what Roblox writes.
#[derive(Clone)]
struct LoadedConfig {
    name: String,
    path: PathBuf,
    rules: Vec<config::Rule>,
    enabled: bool,
    /// The rules that are switched off, by [`rule_keys`].
    off: BTreeSet<String>,
    /// Why the file could not be read when it was last tried; its rules are empty then.
    error: Option<String>,
}

/// What each rule of a config is remembered by: its name, numbered from the
/// second on when several rules share one.
fn rule_keys(rules: &[config::Rule]) -> Vec<String> {
    let mut seen: HashMap<&str, usize> = HashMap::new();
    rules
        .iter()
        .map(|rule| {
            let n = seen.entry(rule.name.as_str()).or_insert(0);
            *n += 1;
            if *n == 1 {
                rule.name.clone()
            } else {
                format!("{} #{n}", rule.name)
            }
        })
        .collect()
}

/// A target that more than one enabled config sets.
struct Conflict {
    target: String,
    /// The configs that set it, by index, each with the rule that does.
    sides: Vec<(usize, String)>,
}

impl Conflict {
    /// The config that has the target: the one the user chose if it is still
    /// among the sides, otherwise the lowest in the list.
    fn winner(&self, configs: &[LoadedConfig], winners: &BTreeMap<String, PathBuf>) -> usize {
        let chosen = winners.get(&self.target).and_then(|path| self.sides.iter().find(|(config, _)| configs[*config].path == *path));
        chosen.or(self.sides.last()).map_or(0, |(config, _)| *config)
    }
}

/// The rules of a config that are switched on, each with its index.
fn active_rules(config: &LoadedConfig) -> impl Iterator<Item = (usize, &config::Rule)> {
    let keys = rule_keys(&config.rules);
    config.rules.iter().enumerate().filter(move |(i, _)| config.enabled && !config.off.contains(&keys[*i]))
}

/// Every target set by rules of more than one enabled config.
fn conflicts(configs: &[LoadedConfig]) -> Vec<Conflict> {
    let mut by_target: BTreeMap<String, Vec<(usize, String)>> = BTreeMap::new();
    for (index, config) in configs.iter().enumerate() {
        for (_, rule) in active_rules(config) {
            for target in &rule.targets {
                let sides = by_target.entry(target.to_string()).or_default();
                if sides.last().is_none_or(|(config, _)| *config != index) {
                    sides.push((index, rule.name.clone()));
                }
            }
        }
    }
    by_target.into_iter().filter(|(_, sides)| sides.len() > 1).map(|(target, sides)| Conflict { target, sides }).collect()
}

/// The rules in force: those of the enabled configs that are switched on,
/// less the targets that were given to another config.
fn rules_in_force(configs: &[LoadedConfig], winners: &BTreeMap<String, PathBuf>) -> Vec<config::Rule> {
    let mut lost: HashSet<(usize, String)> = HashSet::new();
    for conflict in conflicts(configs) {
        let winner = conflict.winner(configs, winners);
        lost.extend(conflict.sides.iter().filter(|(config, _)| *config != winner).map(|(config, _)| (*config, conflict.target.clone())));
    }
    let kept: Vec<(&str, Vec<config::Rule>)> = configs
        .iter()
        .enumerate()
        .filter(|(_, config)| config.enabled)
        .map(|(index, config)| {
            let rules = active_rules(config).map(|(_, rule)| {
                let mut rule = rule.clone();
                rule.targets.retain(|target| !lost.contains(&(index, target.to_string())));
                rule
            });
            (config.name.as_str(), rules.filter(|rule| !rule.targets.is_empty()).collect())
        })
        .collect();
    config::combine(kept.iter().map(|(name, rules)| (*name, rules.as_slice())))
}

/// The rules of `configs` as one list; see [`config::combine`].
fn combined<'a>(configs: impl IntoIterator<Item = &'a LoadedConfig>) -> Vec<config::Rule> {
    config::combine(configs.into_iter().map(|c| (c.name.as_str(), c.rules.as_slice())))
}

/// A pasted cookie being checked with Roblox on a worker thread.
struct AddingAccount {
    result: Receiver<Result<(account::Identity, String), String>>,
}

/// Gets requests ready to go out as the signed-in account, if there is one: a
/// token that is still good, and the game last joined to name if Roblox refuses.
fn prepare_requests(dir: &std::path::Path) {
    account::activate();
    resolve::set_place(store::latest_place_id(dir));
}

/// Progress and result of looking asset IDs up on a worker thread.
enum LookupMsg {
    Progress(usize, usize),
    /// What was found, the cached sounds, and the ID map with what was learned.
    Done(Result<(Vec<Found>, Vec<Sound>, IdMap), String>),
}

/// Stands in for an entry ID when the sound playing is a fetched file.
fn fetched_id(asset: u64) -> Id {
    let mut id = [0xff; 16];
    id[8..].copy_from_slice(&asset.to_be_bytes());
    id
}

/// Where a pack's config was saved, and what that file held before.
type SavedPack = (PathBuf, Option<Vec<u8>>);

/// What bringing the cache in line with the configs produced.
struct Synced {
    summary: String,
    /// The entries as they are after being replaced, and after being put back.
    rewritten: Vec<Entry>,
    restored: Vec<Entry>,
}

/// Progress and result of checking a config on a worker thread.
enum ConfigMsg {
    /// Asset IDs looked up so far, out of how many.
    Progress(usize, usize),
    /// The checked config and the ID map with what the lookup learned.
    Done(Result<(PendingConfigs, IdMap), String>),
}

/// The full-window loading screen; see [`App::show_splash`].
struct Splash {
    /// When it appeared.
    shown: f64,
    /// When it started to dissolve, once the work is done.
    leaving: Option<f64>,
    /// Short title of what is happening.
    caption: String,
    /// A sentence under the title that says what that work is doing.
    detail: String,
    fraction: Option<f32>,
}

struct Toast {
    text: String,
    error: bool,
    born: f64,
}

/// The highlight behind the focused row. Its leading edge is eased toward a
/// newly focused row faster than its trailing edge, so it stretches while it
/// travels and closes up on arrival.
#[derive(Default)]
struct Blob {
    id: Option<Id>,
    lead: f32,
    tail: f32,
    visible: bool,
    moving: bool,
}

struct App {
    dir: PathBuf,
    idmap_path: PathBuf,
    idmap: IdMap,
    /// Why the ID map could not be loaded. While set, the map is never written,
    /// so a file that failed to parse is not replaced by an empty one.
    idmap_error: Option<String>,
    tag_input: String,
    entries: Vec<Entry>,
    index: HashMap<Id, usize>,
    /// Indices into `entries` that pass the filters, in display order.
    view: Vec<usize>,
    view_bytes: u64,
    kinds: Vec<(&'static str, usize)>,
    categories: Vec<(i64, usize)>,
    /// How many entries each of [`GROUPS`] has, and how many belong to none.
    group_counts: [usize; GROUPS.len()],
    other_count: usize,

    loading: Option<Receiver<LoadMsg>>,
    progress: (usize, usize),
    load_error: Option<String>,
    /// Kept current by a background thread; see [`watch_roblox`].
    roblox: Arc<AtomicBool>,
    roblox_running: bool,

    search: String,
    kind_filter: Option<&'static str>,
    category_filter: Option<i64>,
    show_redirects: bool,
    sort: SortKey,
    ascending: bool,
    tab: Tab,
    /// The tab each half of the sidebar last showed, to go back to.
    last_browse: Tab,
    last_manage: Tab,
    /// When the tab was last switched, for its entrance.
    tab_time: f64,
    /// When each sidebar row and chip first appeared, and where the sidebar's highlight is now.
    nav_seen: HashMap<&'static str, f64>,
    nav_y: Option<f32>,
    /// The Live list keeps its own sort order; this is the order of whichever side is not showing.
    other_sort: (SortKey, bool),
    /// Entries Roblox wrote while watching, oldest first.
    arrivals: Vec<Id>,

    selected: BTreeSet<Id>,
    focus: Option<Id>,
    anchor: Option<usize>,
    preview: Option<Preview>,
    confirm_delete: bool,
    confirm_clear: bool,
    /// The outcome of a cache clear that is under way.
    clearing: Option<Receiver<Result<Vec<String>, String>>>,
    delete_prompt: String,
    pending_configs: Option<PendingConfigs>,
    /// A config being checked in the background.
    config_job: Option<Receiver<ConfigMsg>>,
    /// Asset IDs looked up so far for the config being checked, and how many there are.
    config_progress: (usize, usize),
    /// A confirmed config being applied in the background.
    apply_job: Option<Receiver<Result<Synced, String>>>,
    /// Entries Roblox has written that the configs have not been put on yet, and the worker doing so.
    reapply_targets: HashSet<Id>,
    reapply_job: Option<Receiver<Result<Vec<Entry>, String>>>,
    /// Set when the configs, their rules or the choices between them changed, so the cache has to follow.
    sync_wanted: bool,
    /// True while the work going on was started by a switch on the Configs page, which should not take over the window.
    quiet: bool,
    splash: Option<Splash>,
    /// When the current stretch of work began, for deciding whether it merits a splash.
    busy_since: Option<f64>,
    confirm_configs: bool,
    /// The loaded configs; the enabled ones are re-applied to whatever Roblox writes while watching.
    configs: Vec<LoadedConfig>,
    /// Changes arriving from the watcher thread, and the flag that stops it.
    watch: Option<(Receiver<Change>, Arc<AtomicBool>)>,
    tracker: Tracker,
    protections: Option<Protections>,
    /// When each recently written entry arrived, for the marker beside it.
    fresh: HashMap<Id, f64>,
    /// The entry "Replace with source" copies from.
    source: Option<Id>,
    /// The known packs: the list built in, until the one on GitHub arrives.
    catalog: Catalog,
    /// The list being fetched, whether one has ever arrived, and why the last attempt failed.
    catalog_job: Option<Receiver<Result<Catalog, String>>>,
    catalog_fetched: bool,
    catalog_note: Option<String>,
    /// A pack whose config is being downloaded, and its name. The answer is
    /// where the config was saved and what that file held before.
    pack_job: Option<(Receiver<Result<SavedPack, String>>, String)>,
    /// The same, for a pack that is waiting to be confirmed: cancelling puts the file back.
    pack_undo: Option<SavedPack>,
    /// The IDs tab: what is typed, what was found (newest first), the lookup
    /// under way with how far it is, and the cached sounds an ID can be picked from.
    lookup_input: String,
    /// Set to look up what is typed as soon as the cache has been read.
    lookup_wanted: bool,
    /// Set after signing in, to look at the loaded configs again now that sounds can be fetched.
    recheck_configs: bool,
    lookup_found: Vec<Found>,
    lookup_job: Option<Receiver<LookupMsg>>,
    lookup_progress: (usize, usize),
    lookup_sounds: Vec<Sound>,
    /// Pictures of the files that were fetched, by asset ID.
    lookup_pictures: HashMap<u64, egui::TextureHandle>,
    /// The Roblox accounts that have been added by pasting a cookie.
    accounts: Accounts,
    /// What is typed into the Account page's cookie box.
    account_input: String,
    account_job: Option<AddingAccount>,
    account_error: Option<String>,
    /// When the pre-submit warning opened, in egui's clock. The cookie is not sent until [`account::SUBMIT_HOLD`] has passed.
    account_warn_from: Option<f64>,
    /// Avatar pictures by user ID, those already asked for, and where the answers arrive.
    avatars: HashMap<u64, egui::TextureHandle>,
    avatar_asked: HashSet<u64>,
    avatar_tx: std::sync::mpsc::Sender<(u64, Option<Vec<u8>>)>,
    avatar_rx: Receiver<(u64, Option<Vec<u8>>)>,
    /// What changed in this version and the ones before it.
    changelog: Vec<Version>,
    /// Whether the changes since the version last used are waiting to be shown.
    show_news: bool,
    /// Later copies of the app asking this one to come forward.
    summons: Option<Receiver<()>>,
    /// A newer release, once a check has found one.
    update: Option<Release>,
    /// The answer of a check in progress, and whether the user asked for it.
    update_check: Option<(Receiver<Result<Release, String>>, bool)>,
    status: String,

    settings: Settings,
    /// The notification-area icon, where the platform has one.
    tray: Option<Tray>,
    /// Set once the user has asked to quit, so closing is not turned into hiding.
    quitting: bool,
    blob: Blob,
    /// When the detail panel last switched entry.
    panel_time: f64,
    toast: Option<Toast>,
    scroll_to: Option<usize>,
    focus_search: bool,
    audio: Option<Audio>,
    force_preview: bool,
}

impl App {
    fn new(cc: &eframe::CreationContext<'_>, dir: PathBuf, idmap_path: PathBuf, icon: Vec<u8>, tab: Tab, listener: Option<TcpListener>) -> Self {
        theme::apply(&cc.egui_ctx);
        let (idmap, idmap_error) = match IdMap::load(&idmap_path) {
            Ok(map) => (map, None),
            Err(e) => (IdMap::default(), Some(format!("{e:#}"))),
        };
        // Copies made by "Open" in earlier sessions.
        let _ = std::fs::remove_dir_all(open_dir());
        let settings = Settings::load();
        let (avatar_tx, avatar_rx) = channel();
        let mut app = Self {
            dir,
            idmap_path,
            idmap,
            status: idmap_error.clone().unwrap_or_default(),
            idmap_error,
            tag_input: String::new(),
            entries: Vec::new(),
            index: HashMap::new(),
            view: Vec::new(),
            view_bytes: 0,
            kinds: Vec::new(),
            categories: Vec::new(),
            group_counts: [0; GROUPS.len()],
            other_count: 0,
            loading: None,
            progress: (0, 0),
            load_error: None,
            roblox: watch_roblox(),
            roblox_running: false,
            search: String::new(),
            kind_filter: None,
            category_filter: None,
            show_redirects: false,
            sort: SortKey::Size,
            ascending: false,
            tab: Tab::All,
            last_browse: Tab::All,
            last_manage: Tab::Configs,
            tab_time: 0.0,
            nav_seen: HashMap::new(),
            nav_y: None,
            other_sort: (SortKey::Arrival, false),
            arrivals: Vec::new(),
            selected: BTreeSet::new(),
            focus: None,
            anchor: None,
            preview: None,
            confirm_delete: false,
            confirm_clear: false,
            clearing: None,
            delete_prompt: String::new(),
            pending_configs: None,
            config_job: None,
            config_progress: (0, 0),
            apply_job: None,
            sync_wanted: false,
            reapply_targets: HashSet::new(),
            reapply_job: None,
            quiet: false,
            splash: None,
            busy_since: None,
            confirm_configs: false,
            configs: Vec::new(),
            watch: None,
            tracker: Tracker::default(),
            protections: Protections::load_default().ok(),
            fresh: HashMap::new(),
            source: None,
            catalog: packs::bundled(),
            catalog_job: None,
            catalog_fetched: false,
            catalog_note: None,
            pack_job: None,
            pack_undo: None,
            lookup_input: String::new(),
            lookup_wanted: false,
            recheck_configs: false,
            lookup_found: Vec::new(),
            lookup_job: None,
            lookup_progress: (0, 0),
            lookup_sounds: Vec::new(),
            lookup_pictures: HashMap::new(),
            accounts: Accounts::load(),
            account_input: String::new(),
            account_job: None,
            account_error: None,
            account_warn_from: None,
            avatars: HashMap::new(),
            avatar_asked: HashSet::new(),
            avatar_tx,
            avatar_rx,
            changelog: changelog::bundled(),
            show_news: settings.seen_version != update::CURRENT,
            summons: None,
            update: None,
            update_check: None,
            tray: Tray::new(cc, icon, ICON_SIZE),
            settings,
            quitting: false,
            blob: Blob::default(),
            panel_time: 0.0,
            toast: None,
            scroll_to: None,
            focus_search: false,
            audio: None,
            force_preview: false,
        };
        if let Some(listener) = listener {
            let (tx, rx) = channel();
            let ctx = cc.egui_ctx.clone();
            instance::listen(listener, tx, move || ctx.request_repaint());
            app.summons = Some(rx);
        }
        app.restore_configs();
        app.start_load(&cc.egui_ctx);
        app.check_update(&cc.egui_ctx, false);
        app.fetch_catalog(&cc.egui_ctx);
        app.fetch_avatars(&cc.egui_ctx);
        app.show_tab(tab, &cc.egui_ctx);
        app
    }

    /// Fetches the pack list from GitHub without holding up the window.
    fn fetch_catalog(&mut self, ctx: &egui::Context) {
        if self.catalog_job.is_some() {
            return;
        }
        let (tx, rx) = channel();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let _ = tx.send(packs::fetch().map_err(|e| e.root_cause().to_string()));
            ctx.request_repaint();
        });
        self.catalog_job = Some(rx);
    }

    /// Adds the fingerprints the pack list carries to the ID map.
    fn learn_fingerprints(&mut self, ctx: &egui::Context) {
        // A map that failed to load is never written, so the broken file stays as it was.
        if self.idmap_error.is_some() || self.idmap.merge(&self.catalog.fingerprints) == 0 {
            return;
        }
        if let Err(e) = self.idmap.save(&self.idmap_path) {
            self.notify(ctx, format!("The ID map could not be saved: {e:#}"), true);
        }
    }

    fn poll_catalog(&mut self, ctx: &egui::Context) {
        let Some(result) = self.catalog_job.as_ref().and_then(finished) else { return };
        self.catalog_job = None;
        match result {
            Ok(catalog) => {
                (self.catalog, self.catalog_fetched, self.catalog_note) = (catalog, true, None);
                self.learn_fingerprints(ctx);
            }
            Err(e) => self.catalog_note = Some(format!("The pack list could not be fetched, so this is the one this version came with. ({e})")),
        }
        // The cards come in again now that the list is here.
        if self.tab == Tab::Packs {
            self.tab_time = ctx.input(|i| i.time);
        }
    }

    /// Downloads a pack's config on a worker thread; it is then checked like any config.
    fn install_pack(&mut self, ctx: &egui::Context, index: usize) {
        let Some(pack) = self.catalog.packs.get(index).cloned() else { return };
        let (tx, rx) = channel();
        let (ctx, name) = (ctx.clone(), pack.name.clone());
        std::thread::spawn(move || {
            let before = pack.path().ok().and_then(|path| std::fs::read(path).ok());
            let _ = tx.send(pack.install().map(|path| (path, before)).map_err(|e| format!("{e:#}")));
            ctx.request_repaint();
        });
        self.pack_job = Some((rx, name));
    }

    fn poll_pack(&mut self, ctx: &egui::Context) {
        let Some(result) = self.pack_job.as_ref().and_then(|(rx, _)| finished(rx)) else { return };
        self.pack_job = None;
        let checking = result.map_err(|e| anyhow!(e)).and_then(|(path, before)| {
            self.pack_undo = Some((path.clone(), before));
            self.check_configs(ctx, vec![path])
        });
        if let Err(e) = checking {
            self.undo_pack();
            self.notify(ctx, format!("{e:#}"), true);
        }
    }

    /// Puts a pack's config file back as it was before the download that was not confirmed.
    fn undo_pack(&mut self) {
        match self.pack_undo.take() {
            Some((path, Some(before))) => drop(std::fs::write(path, before)),
            Some((path, None)) => drop(std::fs::remove_file(path)),
            None => {}
        }
    }

    /// Switches tab, starting its entrance.
    fn show_tab(&mut self, tab: Tab, ctx: &egui::Context) {
        if self.tab == tab {
            return;
        }
        if (self.tab == Tab::Live) != (tab == Tab::Live) {
            let current = (self.sort, self.ascending);
            (self.sort, self.ascending) = std::mem::replace(&mut self.other_sort, current);
        }
        self.tab = tab;
        self.tab_time = ctx.input(|i| i.time);
        if tab.lists() {
            if self.last_browse != tab {
                // A type filter or a selection made on one list means nothing on another.
                self.kind_filter = None;
                self.selected.clear();
                (self.focus, self.preview) = (None, None);
            }
            self.rebuild_view();
        }
        if tab == Tab::Packs && !self.catalog_fetched {
            self.fetch_catalog(ctx);
        }
        if tab.lists() {
            self.last_browse = tab;
        } else {
            self.last_manage = tab;
        }
        // A sound started on one tab has no stop button on another.
        if let Some(audio) = &mut self.audio {
            audio.stop();
        }
    }

    /// How far the current tab's entrance has come, 0 to 1, for the `index`-th
    /// thing on it; later ones start a little after earlier ones.
    fn entrance(&self, ctx: &egui::Context, index: usize) -> f32 {
        if !self.settings.animations {
            return 1.0;
        }
        let age = (ctx.input(|i| i.time) - self.tab_time) as f32 - index.min(12) as f32 * 0.045;
        if age < 0.34 {
            ctx.request_repaint();
        }
        theme::ease_out(age / 0.34)
    }

    fn start_load(&mut self, ctx: &egui::Context) {
        let (tx, rx) = channel();
        let (dir, ctx) = (self.dir.clone(), ctx.clone());
        std::thread::spawn(move || {
            let result = Store::open(&dir, false).and_then(|store| {
                store.list(|done, total| {
                    let _ = tx.send(LoadMsg::Progress(done, total));
                    ctx.request_repaint();
                })
            });
            let _ = tx.send(LoadMsg::Done(result.map_err(|e| format!("{e:#}"))));
            ctx.request_repaint();
        });
        self.stop_watch();
        self.loading = Some(rx);
        self.progress = (0, 0);
        self.load_error = None;
    }

    fn poll_load(&mut self, ctx: &egui::Context) {
        let Some(rx) = &self.loading else { return };
        let mut finished = None;
        for msg in rx.try_iter() {
            match msg {
                LoadMsg::Progress(done, total) => self.progress = (done, total),
                LoadMsg::Done(result) => finished = Some(result),
            }
        }
        match finished {
            Some(Ok(entries)) => {
                self.loading = None;
                self.entries = entries;
                if let (Some(protections), Ok(store)) = (&mut self.protections, Store::open(&self.dir, true)) {
                    let ids: Vec<Id> = protections.entries().collect();
                    if let Ok(restored) = protections.restore(&store, ids) {
                        for entry in &restored {
                            self.tracker.record(entry);
                        }
                        watch::merge(&mut self.entries, &Change { written: restored, ..Default::default() });
                    }
                }
                self.selected.clear();
                self.focus = None;
                self.preview = None;
                self.reindex();
                if self.settings.watch {
                    self.start_watch(ctx);
                }
                if self.status.is_empty() {
                    self.status = format!("Loaded {}", format::count(self.entries.len(), "entry", "entries"));
                }
            }
            Some(Err(e)) => {
                self.loading = None;
                self.load_error = Some(e);
            }
            None => {}
        }
    }

    /// Asks GitHub for the latest release without holding up the window.
    /// `asked` is true when the user requested it and wants to hear the result either way.
    fn check_update(&mut self, ctx: &egui::Context, asked: bool) {
        let (tx, rx) = channel();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let _ = tx.send(update::latest().map_err(|e| format!("{e:#}")));
            ctx.request_repaint();
        });
        self.update_check = Some((rx, asked));
    }

    fn poll_update(&mut self, ctx: &egui::Context) {
        let Some((rx, asked)) = &self.update_check else { return };
        let (Ok(result), asked) = (rx.try_recv(), *asked) else { return };
        self.update_check = None;
        match result {
            Ok(release) if release.is_newer() => {
                let news: usize = release.news.iter().map(|v| v.changes.len()).sum();
                let more = if news > 0 { format!(", with {}; see Info", format::count(news, "change", "changes")) } else { String::new() };
                self.notify(ctx, format!("rbxcache {} is available{more}", release.version), false);
                self.update = Some(release);
            }
            Ok(_) if asked => self.notify(ctx, format!("rbxcache {} is the latest release", update::CURRENT), false),
            Err(e) if asked => self.notify(ctx, format!("Could not check for updates: {e}"), true),
            // A check made at startup stays quiet unless there is something to install.
            _ => {}
        }
    }

    /// Empties the cache on a worker thread; closing Roblox and deleting tens of
    /// thousands of files takes a moment.
    fn start_clear(&mut self, ctx: &egui::Context) {
        // The watcher holds the index open, which would keep it from being deleted.
        self.stop_watch();
        let (tx, rx) = channel();
        let (dir, ctx) = (self.dir.clone(), ctx.clone());
        std::thread::spawn(move || {
            let _ = tx.send(store::clear_cache(&dir, true).map_err(|e| format!("{e:#}")));
            ctx.request_repaint();
        });
        self.clearing = Some(rx);
    }

    fn poll_clear(&mut self, ctx: &egui::Context) {
        let Some(result) = self.clearing.as_ref().and_then(finished) else { return };
        self.clearing = None;
        match result {
            Ok(steps) => {
                self.entries.clear();
                self.arrivals.clear();
                self.selected.clear();
                self.fresh.clear();
                self.tracker = Tracker::default();
                (self.focus, self.preview, self.source) = (None, None, None);
                self.reindex();
                self.notify(ctx, format!("Cache cleared: {}", steps.join(", ").to_lowercase()), false);
            }
            Err(e) => self.notify(ctx, format!("The cache was not cleared: {e}"), true),
        }
        // Whatever is in the cache now is read again; after a clear that is nothing
        // until Roblox starts and writes a new index, which the watcher waits for.
        if self.settings.watch {
            self.start_watch(ctx);
        }
    }

    /// Hides the window in the tray. A sound would go on playing with no way to stop it, so it is stopped.
    fn hide_to_tray(&mut self) {
        let Some(tray) = &mut self.tray else { return };
        tray.hide();
        if let Some(audio) = &mut self.audio {
            audio.stop();
        }
    }

    /// Acts on the tray icon, and decides what closing the window does.
    fn handle_close(&mut self, ctx: &egui::Context) {
        if self.summons.as_ref().is_some_and(|summons| summons.try_iter().count() > 0) {
            match &mut self.tray {
                Some(tray) => tray.show(),
                None => {
                    ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(false));
                    ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
                }
            }
        }
        let mut quit = false;
        if let Some(tray) = &mut self.tray {
            while let Some(event) = tray.poll() {
                match event {
                    TrayEvent::Show => tray.show(),
                    TrayEvent::Quit => quit = true,
                }
            }
        }
        if quit {
            self.quitting = true;
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
        if !ctx.input(|i| i.viewport().close_requested()) {
            return;
        }
        if self.apply_job.is_some() || self.reapply_job.is_some() {
            // Leaving now could stop between writing a blob and its index row; the worker's poll closes afterwards.
            self.quitting = true;
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
        } else if self.tray.is_some() && self.settings.close_to_tray && !self.quitting {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.hide_to_tray();
        }
    }

    fn stop_watch(&mut self) {
        if let Some((_, stop)) = self.watch.take() {
            stop.store(true, Ordering::Relaxed);
        }
    }

    /// Starts a thread that reports what Roblox does to the cache from here on.
    fn start_watch(&mut self, ctx: &egui::Context) {
        self.stop_watch();
        let (tx, rx) = channel();
        let stop = Arc::new(AtomicBool::new(false));
        let stamps: Vec<store::Stamp> = self.entries.iter().map(Entry::stamp).collect();
        let (dir, ctx, flag) = (self.dir.clone(), ctx.clone(), stop.clone());
        std::thread::spawn(move || {
            // After the cache is cleared there is no index until Roblox makes a new one.
            let mut watcher = loop {
                match Watcher::new(&dir, stamps.iter().copied()) {
                    Ok(watcher) => break watcher,
                    Err(_) if flag.load(Ordering::Relaxed) => return,
                    Err(_) => std::thread::sleep(std::time::Duration::from_secs(2)),
                }
            };
            while !flag.load(Ordering::Relaxed) {
                std::thread::sleep(std::time::Duration::from_millis(1000));
                // A failed poll (the index is briefly locked, say) is simply tried again.
                if let Ok(Some(change)) = watcher.poll() {
                    if tx.send(change).is_err() {
                        return;
                    }
                    ctx.request_repaint();
                }
            }
        });
        self.watch = Some((rx, stop));
    }

    /// Folds in what the watcher saw, and re-applies the enabled configs to it.
    fn poll_watch(&mut self, ctx: &egui::Context) {
        let changes: Vec<Change> = match &self.watch {
            Some((rx, _)) => rx.try_iter().collect(),
            None => return,
        };
        if changes.is_empty() {
            return;
        }
        let now = ctx.input(|i| i.time);
        let mut written: HashSet<Id> = HashSet::new();
        for change in &changes {
            // Replacements this tool wrote come back as writes too; they are not arrivals.
            let roblox: Vec<Id> = change.written.iter().filter(|e| !self.tracker.holds(e)).map(|e| e.id).collect();
            if !roblox.is_empty() {
                // An entry written again moves to the end, so it shows as the newest.
                self.arrivals.retain(|id| !roblox.contains(id));
                self.arrivals.extend(roblox);
            }
            if !change.removed.is_empty() {
                self.arrivals.retain(|id| !change.removed.contains(id));
            }
            for verdict in self.tracker.observe(change) {
                match verdict {
                    // The first read is the news; later ones only move the counter.
                    Verdict::Read { id, reads: 1 } => {
                        self.notify(ctx, format!("Roblox read the replacement in {} and kept it", &store::hex(&id)[..12]), false)
                    }
                    Verdict::Read { .. } => {}
                    Verdict::Reverted { id, times } => {
                        let text = format!("Roblox wrote {} again, discarding the replacement ({times}x)", &store::hex(&id)[..12]);
                        self.notify(ctx, text, true);
                    }
                }
            }
            for entry in &change.written {
                written.insert(entry.id);
                self.fresh.insert(entry.id, now);
            }
            watch::merge(&mut self.entries, change);
        }
        if !written.is_empty() {
            if let (Some(protections), Ok(store)) = (&mut self.protections, Store::open(&self.dir, true)) {
                if let Ok(restored) = protections.restore(&store, written.iter().copied()) {
                    for entry in &restored {
                        self.tracker.record(entry);
                    }
                    watch::merge(&mut self.entries, &Change { written: restored, ..Default::default() });
                }
            }
        }
        self.reapply_targets.extend(written.iter().filter(|id| self.tracker.may_replace(id)));
        if self.focus.is_some_and(|id| written.contains(&id)) {
            self.preview = None;
        }
        self.reindex();
        self.selected.retain(|id| self.index.contains_key(id));
        if self.focus.is_some_and(|id| !self.index.contains_key(&id)) {
            self.focus = None;
        }
    }

    /// Puts the rules in force on the entries Roblox has just written, on a
    /// worker thread: a rule may have a file to download or a texture to
    /// compress, and neither may hold up the window.
    fn start_reapply(&mut self, ctx: &egui::Context) {
        if self.reapply_targets.is_empty() || self.reapply_job.is_some() || self.busy().is_some() {
            return;
        }
        let targets = std::mem::take(&mut self.reapply_targets);
        let rules = rules_in_force(&self.configs, &self.settings.winners);
        if rules.is_empty() {
            return;
        }
        let (tx, rx) = channel();
        let (dir, entries, idmap, ctx) = (self.dir.clone(), self.entries.clone(), self.idmap.clone(), ctx.clone());
        std::thread::spawn(move || {
            prepare_requests(&dir);
            let result = Store::open(&dir, true).map(|store| {
                let mut originals = Originals::load_default().ok();
                let reports = config::run(&store, &entries, &idmap, &rules, true, Some(&targets), originals.as_mut());
                reports.into_iter().flat_map(|r| r.written).collect()
            });
            let _ = tx.send(result.map_err(|e| format!("{e:#}")));
            ctx.request_repaint();
        });
        self.reapply_job = Some(rx);
    }

    fn poll_reapply(&mut self, ctx: &egui::Context) {
        let Some(result) = self.reapply_job.as_ref().and_then(finished) else { return };
        self.reapply_job = None;
        match result {
            Ok(rewritten) if rewritten.is_empty() => {}
            Ok(rewritten) => {
                self.status = format!("Re-applied configs to {}", format::count(rewritten.len(), "entry", "entries"));
                for entry in &rewritten {
                    self.tracker.record(entry);
                }
                if self.focus.is_some_and(|id| rewritten.iter().any(|e| e.id == id)) {
                    self.preview = None;
                }
                watch::merge(&mut self.entries, &Change { written: rewritten, ..Default::default() });
                self.reindex();
            }
            Err(e) => self.status = format!("Configs could not be re-applied: {e}"),
        }
        // A quit asked for while the cache was being written was held back until now.
        if self.quitting {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
    }

    /// Rebuilds everything derived from `entries` after it changed.
    fn reindex(&mut self) {
        self.index = self.entries.iter().enumerate().map(|(i, e)| (e.id, i)).collect();
        let mut kinds: BTreeMap<&'static str, usize> = BTreeMap::new();
        let mut categories: BTreeMap<i64, usize> = BTreeMap::new();
        for e in &self.entries {
            *kinds.entry(e.kind).or_default() += 1;
            *categories.entry(e.category).or_default() += 1;
        }
        self.group_counts = std::array::from_fn(|i| kinds.iter().filter(|(kind, _)| GROUPS[i].1.contains(*kind)).map(|(_, n)| n).sum());
        self.other_count = kinds.iter().filter(|(kind, _)| Tab::Other.holds(kind)).map(|(_, n)| n).sum();
        self.kinds = kinds.into_iter().collect();
        self.categories = categories.into_iter().collect();
        // A tab whose entries are all gone is no longer listed, so it cannot stay open.
        let emptied = match self.tab {
            Tab::Group(i) => self.group_counts[i] == 0,
            Tab::Other => self.other_count == 0,
            _ => false,
        };
        if emptied && self.loading.is_none() {
            (self.tab, self.last_browse) = (Tab::All, Tab::All);
        }
        if matches!(self.last_browse, Tab::Group(i) if self.group_counts[i] == 0) || (self.last_browse == Tab::Other && self.other_count == 0) {
            self.last_browse = Tab::All;
        }
        self.rebuild_view();
    }

    /// Applies the filters and the sort order.
    fn rebuild_view(&mut self) {
        let needle = self.search.trim().to_lowercase();
        let redirects = self.show_redirects || self.kind_filter == Some("redirect") || self.category_filter == Some(11);
        let entries = &self.entries;
        // Position of each arrival, which doubles as the Live tab's membership test.
        let arrived: HashMap<Id, usize> = match self.tab {
            Tab::Live => self.arrivals.iter().enumerate().map(|(n, id)| (*id, n)).collect(),
            _ => HashMap::new(),
        };
        let mut view: Vec<usize> = (0..entries.len())
            .filter(|&i| self.tab != Tab::Live || arrived.contains_key(&entries[i].id))
            .filter(|&i| {
                let e = &entries[i];
                (redirects || e.kind != "redirect")
                    && self.tab.holds(e.kind)
                    && self.kind_filter.is_none_or(|k| e.kind == k)
                    && self.category_filter.is_none_or(|c| e.category == c)
                    && (needle.is_empty() || matches(e, &needle))
            })
            .collect();
        let key = self.sort;
        view.sort_by(|&a, &b| {
            let (a, b) = (&entries[a], &entries[b]);
            match key {
                SortKey::Arrival => arrived.get(&a.id).cmp(&arrived.get(&b.id)),
                SortKey::Kind => a.kind.cmp(b.kind).then(b.size.cmp(&a.size)),
                SortKey::Category => a.category.cmp(&b.category).then(b.size.cmp(&a.size)),
                SortKey::Size => a.size.cmp(&b.size),
                SortKey::Hits => a.hits.cmp(&b.hits),
                SortKey::Used => a.atime.cmp(&b.atime),
                SortKey::Id => a.id.cmp(&b.id),
            }
        });
        if !self.ascending {
            view.reverse();
        }
        self.view_bytes = view.iter().map(|&i| entries[i].size).sum();
        self.view = view;
        self.anchor = None;
    }

    fn sort_by(&mut self, key: SortKey) {
        if self.sort == key {
            self.ascending = !self.ascending;
        } else {
            self.sort = key;
            self.ascending = matches!(key, SortKey::Kind | SortKey::Category | SortKey::Id);
        }
        self.rebuild_view();
    }

    fn click_row(&mut self, pos: usize, modifiers: egui::Modifiers) {
        let id = self.entries[self.view[pos]].id;
        match self.anchor {
            Some(anchor) if modifiers.shift => {
                if !modifiers.command {
                    self.selected.clear();
                }
                for p in anchor.min(pos)..=anchor.max(pos) {
                    self.selected.insert(self.entries[self.view[p]].id);
                }
            }
            _ if modifiers.command => {
                self.anchor = Some(pos);
                if self.selected.remove(&id) {
                    // Keep the detail panel on an entry that is still selected.
                    self.focus = self.selected.iter().next_back().copied();
                    return;
                }
                self.selected.insert(id);
            }
            _ => {
                self.selected.clear();
                self.selected.insert(id);
                self.anchor = Some(pos);
            }
        }
        self.focus = Some(id);
    }

    /// Moves the focus up or down the visible list with the keyboard.
    fn step_focus(&mut self, delta: isize, extend: bool) {
        if self.view.is_empty() {
            return;
        }
        let current = self.focus.and_then(|id| self.view.iter().position(|&i| self.entries[i].id == id));
        let pos = match current {
            Some(p) => p.saturating_add_signed(delta).min(self.view.len() - 1),
            None => 0,
        };
        let modifiers = egui::Modifiers { shift: extend && self.anchor.is_some(), ..Default::default() };
        let anchor = self.anchor;
        self.click_row(pos, modifiers);
        if modifiers.shift {
            self.anchor = anchor;
        }
        self.scroll_to = Some(pos);
    }

    fn focused(&self) -> Option<&Entry> {
        self.focus.and_then(|id| self.index.get(&id)).map(|&i| &self.entries[i])
    }

    fn selected_bytes(&self) -> u64 {
        self.selected.iter().filter_map(|id| self.index.get(id)).map(|&i| self.entries[i].size).sum()
    }

    fn ensure_preview(&mut self, ctx: &egui::Context) {
        let Some(entry) = self.focused() else {
            self.preview = None;
            return;
        };
        if self.preview.as_ref().is_some_and(|p| p.id == entry.id) {
            return;
        }
        let force = self.force_preview;
        self.preview = Some(build_preview(&self.dir, entry, &self.idmap, ctx, force));
        self.force_preview = false;
        if !force {
            self.panel_time = ctx.input(|i| i.time);
        }
    }

    fn notify(&mut self, ctx: &egui::Context, text: String, error: bool) {
        self.status = text.clone();
        self.toast = Some(Toast { text, error, born: ctx.input(|i| i.time) });
    }

    fn run_action(&mut self, action: Action, ctx: &egui::Context) {
        // While the cache is being read, cleared or rewritten by a config, nothing may act on the list.
        let harmless = matches!(
            action,
            Action::Quit | Action::OpenFolder | Action::ShowTab(_) | Action::CheckUpdate | Action::OpenUpdate | Action::RefreshPacks
        );
        // Switches on the Configs page only note what is wanted; the cache follows when it is free.
        let harmless = harmless
            || matches!(
                action,
                Action::ToggleConfig(_)
                    | Action::ToggleRule(..)
                    | Action::Choose(..)
                    | Action::RemoveConfig(_)
                    | Action::RemoveAllConfigs
                    | Action::AskAddAccount
                    | Action::AddAccount
                    | Action::SignOut(_)
                    | Action::UseAccount(_)
            );
        if self.busy().is_some() && !harmless {
            return;
        }
        let result = match action {
            Action::Reload => {
                self.status.clear();
                self.start_load(ctx);
                Ok(String::new())
            }
            Action::OpenFolder => open::that(store::blob_dir(&self.dir)).map(|()| String::new()).map_err(Into::into),
            Action::CopyId => match self.focused() {
                Some(e) => {
                    ctx.copy_text(e.id_hex.clone());
                    Ok("ID copied".into())
                }
                None => Ok(String::new()),
            },
            Action::AskClear => {
                self.confirm_clear = true;
                Ok(String::new())
            }
            Action::Clear => {
                self.start_clear(ctx);
                Ok(String::new())
            }
            Action::AskDelete => {
                if !self.selected.is_empty() {
                    let entries = format::count(self.selected.len(), "cache entry", "cache entries");
                    self.delete_prompt = format!("Delete {entries} ({})?", human_size(self.selected_bytes()));
                    self.confirm_delete = true;
                }
                Ok(String::new())
            }
            Action::SortBy(key) => {
                self.sort_by(key);
                Ok(String::new())
            }
            Action::ShowTab(tab) => {
                self.show_tab(tab, ctx);
                Ok(String::new())
            }
            Action::RefreshPacks => {
                self.fetch_catalog(ctx);
                Ok(String::new())
            }
            Action::ApplyPack(index) => {
                self.install_pack(ctx, index);
                Ok(String::new())
            }
            Action::ClearLive => {
                self.arrivals.clear();
                self.rebuild_view();
                Ok(String::new())
            }
            Action::SortByArrival => {
                (self.sort, self.ascending) = (SortKey::Arrival, false);
                self.rebuild_view();
                Ok(String::new())
            }
            Action::CheckUpdate => {
                self.check_update(ctx, true);
                Ok(String::new())
            }
            Action::OpenUpdate => match &self.update {
                Some(release) => open::that(&release.url).map(|()| String::new()).map_err(Into::into),
                None => Ok(String::new()),
            },
            Action::Quit => {
                self.quitting = true;
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                Ok(String::new())
            }
            Action::HideToTray => {
                self.hide_to_tray();
                Ok(String::new())
            }
            Action::Export => self.export_one(),
            Action::ExportMany => self.export_many(),
            Action::OpenExternally => self.open_externally(),
            Action::Replace => self.replace(),
            Action::Delete => self.delete_selected(),
            Action::SetSource => match self.focused() {
                Some(e) => {
                    let text = format!("{} is now the replacement source", &e.id_hex[..12]);
                    self.source = Some(e.id);
                    Ok(text)
                }
                None => Ok(String::new()),
            },
            Action::ReplaceFromSource => self.replace_from_source(),
            Action::AddConfigs => match rfd::FileDialog::new().set_title("Choose replacement configs").add_filter("Config", &["json"]).pick_files() {
                Some(paths) => self.check_configs(ctx, paths),
                None => Ok(String::new()),
            },
            Action::ReapplyConfigs => self.check_configs(ctx, self.configs.iter().filter(|c| c.enabled).map(|c| c.path.clone()).collect()),
            Action::ToggleConfig(i) => {
                if let Some(config) = self.configs.get_mut(i) {
                    config.enabled = !config.enabled;
                    self.configs_changed();
                }
                Ok(String::new())
            }
            Action::ToggleRule(i, rule) => {
                if let Some(config) = self.configs.get_mut(i) {
                    if let Some(key) = rule_keys(&config.rules).into_iter().nth(rule) {
                        if !config.off.remove(&key) {
                            config.off.insert(key);
                        }
                        self.configs_changed();
                    }
                }
                Ok(String::new())
            }
            Action::Choose(targets, i) => {
                if let Some(path) = self.configs.get(i).map(|c| c.path.clone()) {
                    self.settings.winners.extend(targets.into_iter().map(|target| (target, path.clone())));
                    self.configs_changed();
                }
                Ok(String::new())
            }
            Action::RemoveConfig(i) if i < self.configs.len() => {
                let removed = self.configs.remove(i);
                self.configs_changed();
                Ok(format!("{} removed; what it replaced is put back", removed.name))
            }
            Action::RemoveConfig(_) => Ok(String::new()),
            Action::RemoveAllConfigs => {
                let removed = format::count(self.configs.len(), "config", "configs");
                self.configs.clear();
                self.configs_changed();
                Ok(format!("{removed} removed; what they replaced is put back"))
            }
            Action::ConfirmConfigs => {
                self.accept_configs();
                Ok(String::new())
            }
            Action::CancelConfigs => {
                self.pending_configs = None;
                self.undo_pack();
                Ok(String::new())
            }
            Action::Tag => self.tag_focused(),
            Action::PlayAudio => self.play_audio(),
            Action::AskAddAccount => {
                self.ask_add_account(ctx);
                Ok(String::new())
            }
            Action::AddAccount => {
                self.account_warn_from = None;
                self.start_add_account(ctx);
                Ok(String::new())
            }
            Action::SignOut(id) => self.sign_out(id),
            Action::UseAccount(id) => self.accounts.set_active(id).map(|()| String::new()),
            Action::RunLookup | Action::LookupFor(_) if self.accounts.is_empty() => Err(anyhow!("Sign in first: fetching IDs needs an account")),
            Action::RunLookup => {
                let ids = lookup::parse_ids(&self.lookup_input);
                if ids.is_empty() {
                    Err(anyhow!("Type an asset ID first: a number such as 16530229616"))
                } else {
                    self.start_lookup(ctx, ids);
                    Ok(String::new())
                }
            }
            Action::LookupFor(i) => {
                let refused: Vec<u64> = self.configs.get(i).map_or(Vec::new(), |config| {
                    let ids = config::asset_ids(&config.rules);
                    ids.into_iter().filter(|id| self.idmap.assets.get(&id.to_string()).is_some_and(Vec::is_empty)).collect()
                });
                if !refused.is_empty() {
                    self.lookup_input = refused.iter().map(u64::to_string).collect::<Vec<_>>().join(" ");
                    self.show_tab(Tab::Lookup, ctx);
                    self.start_lookup(ctx, refused);
                }
                Ok(String::new())
            }
            Action::PlaySound(id) => Store::open(&self.dir, false).and_then(|store| store.read_body(&id)).and_then(|body| self.play(id, body)),
            Action::PlayFetched(i) => match self.lookup_found.get(i).and_then(|found| Some((found.id, found.file.as_ref()?.1.to_vec()))) {
                Some((asset, body)) => self.play(fetched_id(asset), body),
                None => Ok(String::new()),
            },
            Action::SaveFetched(i) => self.save_fetched(i),
            Action::Pick(asset, entry) => self.pick(asset, entry),
            Action::ShowEntry(id) => {
                self.show_tab(Tab::All, ctx);
                (self.search, self.kind_filter, self.category_filter) = (String::new(), None, None);
                self.rebuild_view();
                self.selected = BTreeSet::from([id]);
                self.focus = Some(id);
                self.scroll_to = self.view.iter().position(|&i| self.entries[i].id == id);
                Ok(String::new())
            }
            Action::CopyFingerprints => {
                let mut known = IdMap::default();
                for found in &self.lookup_found {
                    for fp in self.idmap.assets.get(&found.id.to_string()).into_iter().flatten() {
                        known.add(found.id, fp.clone());
                    }
                }
                serde_json::to_string_pretty(&known).map_err(Into::into).map(|json| {
                    ctx.copy_text(json);
                    format!("Copied the fingerprints of {}", format::count(known.assets.len(), "asset ID", "asset IDs"))
                })
            }
            Action::ExportConverted => self.export_converted(),
            Action::ForcePreview => {
                self.force_preview = true;
                self.preview = None;
                Ok(String::new())
            }
        };
        match result {
            Ok(msg) if msg.is_empty() => {}
            Ok(msg) => self.notify(ctx, msg, false),
            Err(e) => self.notify(ctx, format!("{e:#}"), true),
        }
    }

    fn export_one(&mut self) -> Result<String> {
        let Some(e) = self.focused() else { return Ok(String::new()) };
        let Some(path) = rfd::FileDialog::new().set_file_name(export_name(e, false)).save_file() else {
            return Ok(String::new());
        };
        let body = Store::open(&self.dir, false)?.read_body(&e.id)?;
        std::fs::write(&path, &body).with_context(|| format!("writing {}", path.display()))?;
        Ok(format!("Exported {} to {}", human_size(body.len() as u64), path.display()))
    }

    /// Saves a texture as PNG or a mesh as OBJ.
    fn export_converted(&mut self) -> Result<String> {
        let Some(e) = self.focused() else { return Ok(String::new()) };
        let Some(extension) = viewers::converted_extension(e.kind) else { return Ok(String::new()) };
        let Some(path) = rfd::FileDialog::new().set_file_name(format!("{}.{extension}", e.id_hex)).save_file() else {
            return Ok(String::new());
        };
        let (data, _) = viewers::convert(e.kind, &Store::open(&self.dir, false)?.read_body(&e.id)?)?;
        std::fs::write(&path, &data).with_context(|| format!("writing {}", path.display()))?;
        Ok(format!("Exported {} to {}", human_size(data.len() as u64), path.display()))
    }

    fn export_many(&mut self) -> Result<String> {
        if self.selected.is_empty() {
            return Ok(String::new());
        }
        let Some(folder) = rfd::FileDialog::new().pick_folder() else { return Ok(String::new()) };
        let store = Store::open(&self.dir, false)?;
        let (mut written, mut first_error) = (0, None);
        for e in self.selected.iter().filter_map(|id| self.index.get(id)).map(|&i| &self.entries[i]) {
            let path = folder.join(export_name(e, false));
            match store.read_body(&e.id).and_then(|body| std::fs::write(&path, body).with_context(|| format!("writing {}", path.display()))) {
                Ok(()) => written += 1,
                Err(error) => {
                    first_error.get_or_insert(error);
                }
            }
        }
        let exported = format!("Exported {} to {}", format::count(written, "entry", "entries"), folder.display());
        match first_error {
            Some(e) => Err(anyhow!("{exported}; {} failed: {e:#}", self.selected.len() - written)),
            None => Ok(exported),
        }
    }

    fn open_externally(&mut self) -> Result<String> {
        let Some(e) = self.focused() else { return Ok(String::new()) };
        let folder = open_dir();
        std::fs::create_dir_all(&folder)?;
        let path = folder.join(export_name(e, false));
        std::fs::write(&path, Store::open(&self.dir, false)?.read_body(&e.id)?)?;
        open::that(&path).with_context(|| format!("opening {}", path.display()))?;
        Ok(format!("Opened a copy at {}", path.display()))
    }

    fn replace(&mut self) -> Result<String> {
        let Some(e) = self.focused() else { return Ok(String::new()) };
        let id = e.id;
        let Some(path) = rfd::FileDialog::new().set_title("Choose the replacement file").pick_file() else {
            return Ok(String::new());
        };
        let body = std::fs::read(&path).with_context(|| format!("reading {}", path.display()))?;
        let written = config::replace_by_hand(&Store::open(&self.dir, true)?, &self.entries, &id, body)?;
        let msg = format!("Replaced {} with {}", replaced_names(&written), path.display());
        self.took_replacements(written)?;
        Ok(msg)
    }

    /// Copies the source entry's content over the focused entry.
    fn replace_from_source(&mut self) -> Result<String> {
        let (Some(e), Some(source)) = (self.focused(), self.source) else { return Ok(String::new()) };
        let id = e.id;
        let store = Store::open(&self.dir, true)?;
        // Of a texture's packs, the most detailed one is the picture to copy.
        let sources = config::siblings(&store, &self.entries, &source);
        if sources.contains(&id) {
            return Err(anyhow!("This entry is the replacement source itself"));
        }
        let body = store.read_body(sources.first().unwrap_or(&source))?;
        let written = config::replace_by_hand(&store, &self.entries, &id, body)?;
        let msg = format!("Replaced {} with the content of {}", replaced_names(&written), &store::hex(&source)[..12]);
        self.took_replacements(written)?;
        Ok(msg)
    }

    /// Protects and takes in entries that were replaced by hand.
    fn took_replacements(&mut self, written: Vec<(Entry, Vec<u8>)>) -> Result<()> {
        for (entry, body) in written {
            if let Some(protections) = &mut self.protections {
                protections.remember(&entry.id, &body)?;
            }
            self.replaced(entry);
        }
        Ok(())
    }

    /// Takes in an entry this tool has just rewritten.
    fn replaced(&mut self, updated: Entry) {
        self.tracker.record(&updated);
        if let Some(&i) = self.index.get(&updated.id) {
            self.entries[i] = updated;
        }
        self.preview = None;
        self.reindex();
    }

    fn delete_selected(&mut self) -> Result<String> {
        let store = Store::open(&self.dir, true)?;
        let mut deleted = BTreeSet::new();
        let mut first_error = None;
        for id in &self.selected {
            match store.delete(id) {
                Ok(()) => {
                    deleted.insert(*id);
                }
                Err(e) => {
                    first_error.get_or_insert(e);
                }
            }
        }
        let failed = self.selected.len() - deleted.len();
        self.entries.retain(|e| !deleted.contains(&e.id));
        self.selected.retain(|id| !deleted.contains(id));
        if self.focus.is_some_and(|id| deleted.contains(&id)) {
            self.focus = None;
            self.preview = None;
        }
        self.reindex();
        match first_error {
            Some(e) => Err(anyhow!("Deleted {}, {failed} failed: {e:#}", deleted.len())),
            None => Ok(format!("Deleted {}", format::count(deleted.len(), "entry", "entries"))),
        }
    }

    /// Remembers which configs are loaded for the next run.
    fn save_configs(&mut self) {
        let saved = |c: &LoadedConfig| SavedConfig { path: c.path.clone(), enabled: c.enabled, off: c.off.iter().cloned().collect() };
        self.settings.configs = self.configs.iter().map(saved).collect();
        // A choice between configs lasts only as long as the config that was chosen.
        self.settings.winners.retain(|_, path| self.configs.iter().any(|c| c.path == *path));
        self.settings.save();
    }

    /// Called when a config or a rule was switched, removed or preferred: the
    /// change is remembered and the cache brought in line with it, quietly.
    fn configs_changed(&mut self) {
        self.save_configs();
        (self.sync_wanted, self.quiet) = (true, true);
    }

    /// Gives configs that share a file name their folder as well, so they can be told apart.
    fn name_configs(configs: &mut [LoadedConfig]) {
        let stems: Vec<String> = configs.iter().map(|c| config::name_of(&c.path)).collect();
        for (i, config) in configs.iter_mut().enumerate() {
            let shared = stems.iter().filter(|s| **s == stems[i]).count() > 1;
            let folder = config.path.parent().and_then(|p| p.file_name()).map(|f| f.to_string_lossy().into_owned());
            config.name = match (shared, folder) {
                (true, Some(folder)) => format!("{} ({folder})", stems[i]),
                _ => stems[i].clone(),
            };
        }
    }

    /// Loads the configs that were loaded last time. They are re-applied to what
    /// Roblox writes from now on; nothing already in the cache is touched. One
    /// that cannot be read stays in the list, so a drive that is not there yet
    /// does not cost the user their setup.
    fn restore_configs(&mut self) {
        for saved in self.settings.configs.clone() {
            let (rules, error) = match config::load(&saved.path) {
                Ok(rules) => (rules, None),
                Err(e) => (Vec::new(), Some(format!("{e:#}"))),
            };
            let off = saved.off.into_iter().collect();
            self.configs.push(LoadedConfig { name: String::new(), path: saved.path, rules, enabled: saved.enabled, off, error });
        }
        Self::name_configs(&mut self.configs);
        let unreadable: Vec<&str> = self.configs.iter().filter(|c| c.error.is_some()).map(|c| c.name.as_str()).collect();
        if !unreadable.is_empty() {
            let note = format!("Could not read {}: {}", format::count(unreadable.len(), "config", "configs"), unreadable.join(", "));
            self.status = if self.status.is_empty() { note } else { format!("{}. {note}", self.status) };
        }
    }

    /// On a worker thread, looks up the asset IDs the configs at `paths` name and
    /// works out what they would change together. The result is put up for confirmation.
    fn check_configs(&mut self, ctx: &egui::Context, paths: Vec<PathBuf>) -> Result<String> {
        if self.config_job.is_some() || paths.is_empty() {
            return Ok(String::new());
        }
        let mut configs: Vec<LoadedConfig> = Vec::new();
        for path in paths {
            if !configs.iter().any(|c| config::same_file(&c.path, &path)) {
                configs.push(LoadedConfig {
                    name: String::new(),
                    rules: config::load(&path)?,
                    path,
                    enabled: true,
                    off: BTreeSet::new(),
                    error: None,
                });
            }
        }
        Self::name_configs(&mut configs);
        let rules = combined(&configs);
        let ids = config::asset_ids(&rules);
        let (tx, rx) = channel();
        let (dir, entries, mut idmap, ctx) = (self.dir.clone(), self.entries.clone(), self.idmap.clone(), ctx.clone());
        std::thread::spawn(move || {
            prepare_requests(&dir);
            let result = Store::open(&dir, false).map(|store| {
                let mut lookup = String::new();
                if !ids.is_empty() {
                    let report = |done, total| {
                        let _ = tx.send(ConfigMsg::Progress(done, total));
                        ctx.request_repaint();
                    };
                    lookup = resolve::summarize(&resolve::identify(&store, &entries, &mut idmap, &ids, false, &report));
                }
                let mut originals = Originals::load_default().ok();
                let reports = config::run(&store, &entries, &idmap, &rules, false, None, originals.as_mut());
                let notes = reports.iter().flat_map(|r| r.notes.iter().map(move |n| format!("{}: {n}", r.name))).collect();
                (PendingConfigs { configs, summary: config::summarize(&reports, false), lookup, notes }, idmap)
            });
            let _ = tx.send(ConfigMsg::Done(result.map_err(|e| format!("{e:#}"))));
            ctx.request_repaint();
        });
        self.config_job = Some(rx);
        self.config_progress = (0, 0);
        Ok(String::new())
    }

    fn poll_config(&mut self, ctx: &egui::Context) {
        let Some(rx) = &self.config_job else { return };
        let mut finished = None;
        if let Err(TryRecvError::Disconnected) = rx.try_recv().map(|msg| match msg {
            ConfigMsg::Progress(done, total) => self.config_progress = (done, total),
            ConfigMsg::Done(result) => finished = Some(result),
        }) {
            finished = Some(Err("checking the configs stopped unexpectedly".to_string()));
        }
        for msg in rx.try_iter() {
            match msg {
                ConfigMsg::Progress(done, total) => self.config_progress = (done, total),
                ConfigMsg::Done(result) => finished = Some(result),
            }
        }
        match finished {
            Some(Ok((pending, idmap))) => {
                self.config_job = None;
                self.status = pending.lookup.clone();
                self.idmap = idmap;
                self.idmap.merge(&self.catalog.fingerprints);
                // A map that failed to load is never written, so the broken file stays as it was.
                if self.idmap_error.is_none() {
                    if let Err(e) = self.idmap.save(&self.idmap_path) {
                        self.notify(ctx, format!("The ID map could not be saved: {e:#}"), true);
                    }
                }
                self.pending_configs = Some(pending);
                self.confirm_configs = true;
                self.preview = None;
            }
            Some(Err(e)) => {
                self.config_job = None;
                self.undo_pack();
                self.notify(ctx, e, true);
            }
            None => {}
        }
    }

    /// Takes the configs that were checked and confirmed into the list. One
    /// that was already loaded is replaced by what was just read and moves to
    /// the end with the rest, where it has any entry it shares with the
    /// others; it keeps its switched-off rules.
    fn accept_configs(&mut self) {
        let Some(pending) = self.pending_configs.take() else { return };
        self.pack_undo = None;
        for mut config in pending.configs {
            if let Some(loaded) = self.configs.iter().find(|c| config::same_file(&c.path, &config.path)) {
                config.off = loaded.off.clone();
            }
            self.configs.retain(|c| !config::same_file(&c.path, &config.path));
            self.configs.push(config);
        }
        Self::name_configs(&mut self.configs);
        self.save_configs();
        (self.sync_wanted, self.quiet) = (true, false);
    }

    /// Brings the cache in line with the configs on a worker thread: the rules
    /// in force are applied, and whatever was replaced earlier and is no
    /// longer wanted is put back. Matching rules to entries hashes their
    /// content and `cdn` rules download files, which is too slow to do while drawing.
    fn start_sync(&mut self, ctx: &egui::Context) {
        self.sync_wanted = false;
        // The watcher would take the worker's writes for Roblox's and re-apply
        // the configs on top of them; it waits until this is done.
        self.stop_watch();
        let rules = rules_in_force(&self.configs, &self.settings.winners);
        let (tx, rx) = channel();
        let (dir, entries, idmap, ctx) = (self.dir.clone(), self.entries.clone(), self.idmap.clone(), ctx.clone());
        std::thread::spawn(move || {
            prepare_requests(&dir);
            let result = Store::open(&dir, true).and_then(|store| {
                let mut originals = Originals::load_default()?;
                let (reports, restored) = config::sync(&store, &entries, &idmap, &rules, &mut originals);
                let mut summary = config::summarize(&reports, true);
                if !restored.is_empty() {
                    summary.push_str(&format!(", {} put back", format::count(restored.len(), "entry", "entries")));
                }
                Ok(Synced { summary, rewritten: reports.into_iter().flat_map(|r| r.written).collect(), restored })
            });
            let _ = tx.send(result.map_err(|e| format!("{e:#}")));
            ctx.request_repaint();
        });
        self.apply_job = Some(rx);
    }

    fn poll_apply(&mut self, ctx: &egui::Context) {
        let Some(result) = self.apply_job.as_ref().and_then(finished) else { return };
        self.apply_job = None;
        match result {
            Ok(synced) => {
                for entry in &synced.rewritten {
                    self.tracker.record(entry);
                }
                for entry in &synced.restored {
                    self.tracker.forget(&entry.id);
                }
                let written = synced.rewritten.into_iter().chain(synced.restored).collect();
                watch::merge(&mut self.entries, &Change { written, ..Default::default() });
                self.preview = None;
                self.reindex();
                self.notify(ctx, synced.summary, false);
            }
            Err(e) => self.notify(ctx, e, true),
        }
        if self.settings.watch {
            self.start_watch(ctx);
        }
        // A quit asked for while the cache was being written was held back until now.
        if self.quitting {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
    }

    fn play_audio(&mut self) -> Result<String> {
        let Some(e) = self.focused() else { return Ok(String::new()) };
        if !format::is_audio(e.kind) {
            return Ok(String::new());
        }
        let id = e.id;
        let body = Store::open(&self.dir, false)?.read_body(&id)?;
        self.play(id, body)
    }

    /// Plays a sound, or pauses and resumes it when it is the one already playing.
    fn play(&mut self, id: Id, body: Vec<u8>) -> Result<String> {
        if let Some(audio) = self.audio.as_mut().filter(|a| a.current == Some(id) && a.active()) {
            audio.toggle_pause();
            return Ok(String::new());
        }
        if self.audio.is_none() {
            self.audio = Some(Audio::open()?);
        }
        self.audio.as_mut().expect("audio was just opened").play(id, body)?;
        Ok(String::new())
    }

    /// Opens the warning. The cookie is not sent until it has been up for [`account::SUBMIT_HOLD`].
    fn ask_add_account(&mut self, ctx: &egui::Context) {
        if self.account_warn_from.is_some() || self.account_job.is_some() {
            return;
        }
        if account::extract(&self.account_input).is_none() {
            self.account_error = Some("That did not look like a cookie. Paste the whole .ROBLOSECURITY value.".to_string());
            return;
        }
        self.account_error = None;
        self.account_warn_from = Some(ctx.input(|i| i.time));
    }

    /// Checks the pasted cookie with Roblox on a worker thread: this is a
    /// network call, too slow for the window to wait on directly.
    fn start_add_account(&mut self, ctx: &egui::Context) {
        if self.account_job.is_some() {
            return;
        }
        let Some(cookie) = account::extract(&self.account_input) else {
            self.account_error = Some("That did not look like a cookie. Paste the whole .ROBLOSECURITY value.".to_string());
            return;
        };
        let (tx, rx) = channel();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let result = account::whoami(&cookie).map(|who| (who, cookie)).map_err(|e| format!("{e:#}"));
            let _ = tx.send(result);
            ctx.request_repaint();
        });
        self.account_job = Some(AddingAccount { result: rx });
        self.account_error = None;
    }

    fn poll_add_account(&mut self, ctx: &egui::Context) {
        let Some(result) = self.account_job.as_ref().and_then(|job| finished(&job.result)) else { return };
        self.account_job = None;
        match result {
            Ok((who, cookie)) => {
                // What is on disk may have changed since this window opened, so add to that.
                let mut accounts = Accounts::load();
                match accounts.add(who.clone(), &cookie) {
                    Ok(()) => {
                        self.accounts = accounts;
                        self.account_input.clear();
                        self.fetch_avatars(ctx);
                        self.notify(ctx, format!("Added {}", who.display), false);
                        // What a guest was refused may be there for this account.
                        self.recheck_configs = self.configs.iter().any(|c| c.enabled && id_stats(&c.rules, &self.idmap).2 > 0);
                    }
                    Err(e) => self.notify(ctx, format!("{e:#}"), true),
                }
            }
            Err(e) => {
                self.account_error = Some(e.clone());
                self.notify(ctx, e, true);
            }
        }
    }

    /// Removes an account from this computer at once. This cannot end the
    /// account's session on Roblox's side; there is no API for that with a
    /// cookie, unlike the OAuth tokens this app no longer uses.
    fn sign_out(&mut self, id: u64) -> Result<String> {
        let mut accounts = Accounts::load();
        accounts.remove(id)?;
        self.accounts = accounts;
        self.avatars.remove(&id);
        self.avatar_asked.remove(&id);
        if self.accounts.is_empty() {
            resolve::set_auth(None);
        }
        Ok("Removed from this computer. To end that account's Roblox session too, sign out of it on roblox.com.".to_string())
    }

    /// Asks for the avatar of every signed-in account that has none yet.
    fn fetch_avatars(&mut self, ctx: &egui::Context) {
        for id in self.accounts.accounts.iter().map(|a| a.user_id) {
            if self.avatar_asked.insert(id) {
                let (tx, ctx) = (self.avatar_tx.clone(), ctx.clone());
                std::thread::spawn(move || {
                    let _ = tx.send((id, account::avatar(id)));
                    ctx.request_repaint();
                });
            }
        }
    }

    fn poll_avatars(&mut self, ctx: &egui::Context) {
        for (id, bytes) in self.avatar_rx.try_iter().collect::<Vec<_>>() {
            let picture = bytes.and_then(|bytes| image::load_from_memory(&bytes).ok()).map(|picture| picture.to_rgba8());
            match picture {
                Some(picture) if self.accounts.accounts.iter().any(|a| a.user_id == id) => {
                    let pixels = egui::ColorImage::from_rgba_unmultiplied([picture.width() as usize, picture.height() as usize], picture.as_raw());
                    self.avatars.insert(id, ctx.load_texture(format!("avatar-{id}"), pixels, egui::TextureOptions::LINEAR));
                }
                // Not found, or signed out in the meantime: asked for again next time round.
                _ => drop(self.avatar_asked.remove(&id)),
            }
        }
    }

    /// Looks asset IDs up on a worker thread: what they are, the file when
    /// Roblox hands it out, and the cached sounds one of them might be.
    fn start_lookup(&mut self, ctx: &egui::Context, ids: Vec<u64>) {
        if self.lookup_job.is_some() {
            return;
        }
        let (tx, rx) = channel();
        let (dir, entries, mut idmap, ctx) = (self.dir.clone(), self.entries.clone(), self.idmap.clone(), ctx.clone());
        std::thread::spawn(move || {
            prepare_requests(&dir);
            // Fetches are sent as the active account, so it must still be signed in before anything is asked for.
            let result = account::check_active().and_then(|_| Store::open(&dir, false)).map(|store| {
                let report = |done, total| {
                    let _ = tx.send(LookupMsg::Progress(done, total));
                    ctx.request_repaint();
                };
                let found = lookup::find(&store, &entries, &mut idmap, &ids, &report);
                let sounds = lookup::sounds(&store, &entries, &idmap);
                (found, sounds, idmap)
            });
            let _ = tx.send(LookupMsg::Done(result.map_err(|e| format!("{e:#}"))));
            ctx.request_repaint();
        });
        self.lookup_job = Some(rx);
        self.lookup_progress = (0, 0);
    }

    fn poll_lookup(&mut self, ctx: &egui::Context) {
        let Some(rx) = &self.lookup_job else { return };
        let mut finished = None;
        loop {
            match rx.try_recv() {
                Ok(LookupMsg::Progress(done, total)) => self.lookup_progress = (done, total),
                Ok(LookupMsg::Done(result)) => finished = Some(result),
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    finished.get_or_insert(Err("the lookup stopped unexpectedly".to_string()));
                    break;
                }
            }
        }
        let Some(result) = finished else { return };
        self.lookup_job = None;
        match result {
            Ok((found, sounds, idmap)) => {
                self.idmap = idmap;
                self.idmap.merge(&self.catalog.fingerprints);
                // A map that failed to load is never written, so the broken file stays as it was.
                if self.idmap_error.is_none() {
                    if let Err(e) = self.idmap.save(&self.idmap_path) {
                        self.notify(ctx, format!("The ID map could not be saved: {e:#}"), true);
                    }
                }
                for item in &found {
                    let picture = item
                        .file
                        .as_ref()
                        .filter(|(kind, _)| matches!(*kind, "png" | "jpeg" | "gif" | "webp"))
                        .and_then(|(_, body)| image::load_from_memory(body).ok());
                    if let Some(picture) = picture {
                        let small = picture.thumbnail(256, 256).to_rgba8();
                        let pixels = egui::ColorImage::from_rgba_unmultiplied([small.width() as usize, small.height() as usize], small.as_raw());
                        self.lookup_pictures.insert(item.id, ctx.load_texture("fetched", pixels, egui::TextureOptions::LINEAR));
                    }
                }
                // The newest answers go on top; an ID asked about again replaces its earlier answer.
                self.lookup_found.retain(|old| !found.iter().any(|new| new.id == old.id));
                self.lookup_found.splice(0..0, found);
                self.lookup_sounds = sounds;
                self.lookup_input.clear();
                self.preview = None;
                self.tab_time = ctx.input(|i| i.time);
                // What was learned may be what a config was waiting for.
                (self.sync_wanted, self.quiet) = (true, true);
            }
            Err(e) => self.notify(ctx, e, true),
        }
    }

    /// Ties an asset ID to the cached entry the user says it is.
    fn pick(&mut self, asset: u64, entry: Id) -> Result<String> {
        if let Some(error) = &self.idmap_error {
            return Err(anyhow!("The ID map at {} could not be read ({error}); fix or remove it first", self.idmap_path.display()));
        }
        let fp = config::fingerprint(&Store::open(&self.dir, false)?.read_body(&entry)?);
        if self.idmap.add(asset, fp) {
            self.idmap.save(&self.idmap_path)?;
        }
        if let Some(sound) = self.lookup_sounds.iter_mut().find(|s| s.id == entry) {
            sound.taken = Some(asset.to_string());
        }
        if let Some(found) = self.lookup_found.iter_mut().find(|f| f.id == asset) {
            found.outcome = resolve::Outcome::Known;
        }
        self.preview = None;
        (self.sync_wanted, self.quiet) = (true, true);
        Ok(format!("Asset {asset} is now tied to {}; configs that name it will use it", &store::hex(&entry)[..12]))
    }

    fn save_fetched(&mut self, index: usize) -> Result<String> {
        let Some((id, kind, body)) = self.lookup_found.get(index).and_then(|f| f.file.as_ref().map(|(kind, body)| (f.id, *kind, body.clone())))
        else {
            return Ok(String::new());
        };
        let Some(path) = rfd::FileDialog::new().set_file_name(format!("{id}.{}", format::extension(kind))).save_file() else {
            return Ok(String::new());
        };
        std::fs::write(&path, &body[..]).with_context(|| format!("writing {}", path.display()))?;
        Ok(format!("Saved {} to {}", human_size(body.len() as u64), path.display()))
    }

    fn tag_focused(&mut self) -> Result<String> {
        let Some(e) = self.focused() else { return Ok(String::new()) };
        if let Some(error) = &self.idmap_error {
            return Err(anyhow!("The ID map at {} could not be read ({error}); fix or remove it first", self.idmap_path.display()));
        }
        let asset: u64 = self.tag_input.trim().parse().ok().filter(|&n| n > 0).context("Enter the numeric Roblox asset ID first")?;
        let fp = config::fingerprint(&Store::open(&self.dir, false)?.read_body(&e.id)?);
        let msg = format!("Asset {asset} now points at {}", e.id_hex);
        if self.idmap.add(asset, fp) {
            self.idmap.save(&self.idmap_path)?;
        }
        self.tag_input.clear();
        self.preview = None;
        Ok(msg)
    }

    fn menu_bar(&mut self, ui: &mut egui::Ui, actions: &mut Vec<Action>) {
        let one = self.focus.is_some() && self.selected.len() <= 1;
        let any = !self.selected.is_empty();
        egui::menu::bar(ui, |ui| {
            ui.menu_button("File", |ui| {
                ui.set_min_width(230.0);
                item(ui, actions, true, "Reload cache", "F5", Action::Reload);
                item(ui, actions, true, "Open cache folder", "", Action::OpenFolder);
                item(ui, actions, true, "Clear cache...", "", Action::AskClear);
                ui.separator();
                if self.tray.is_some() {
                    item(ui, actions, true, "Hide to tray", "", Action::HideToTray);
                }
                item(ui, actions, true, "Quit", "", Action::Quit);
            });
            ui.menu_button("Entry", |ui| {
                ui.set_min_width(230.0);
                entry_items(ui, actions, one, any, self.source.is_some());
            });
            ui.menu_button("Configs", |ui| {
                ui.set_min_width(280.0);
                item(ui, actions, true, "Add configs...", "Ctrl+O", Action::AddConfigs);
                item(ui, actions, true, "Manage configs", "", Action::ShowTab(Tab::Configs));
                item(ui, actions, true, "Browse packs", "", Action::ShowTab(Tab::Packs));
                ui.separator();
                if self.configs.is_empty() {
                    ui.weak("No configs loaded");
                }
                egui::ScrollArea::vertical().max_height(320.0).show(ui, |ui| {
                    for (i, config) in self.configs.iter().enumerate() {
                        ui.horizontal(|ui| {
                            let mut enabled = config.enabled;
                            let (label, hint) = match &config.error {
                                Some(error) => (
                                    egui::RichText::new(format!("{} (unreadable)", config.name)).color(theme::WARN),
                                    format!("{}\n{error}", config.path.display()),
                                ),
                                None => {
                                    let hint = "Ticked: applied, and put on what Roblox writes while the cache is watched. Unticked: what it replaced is put back.";
                                    (egui::RichText::new(&config.name), format!("{}\n{hint}", config.path.display()))
                                }
                            };
                            if ui.checkbox(&mut enabled, label).on_hover_text(hint).changed() {
                                actions.push(Action::ToggleConfig(i));
                            }
                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                if ui.small_button("Remove").on_hover_text("Stop using this config and put back what it replaced").clicked() {
                                    actions.push(Action::RemoveConfig(i));
                                    ui.close_menu();
                                }
                            });
                        });
                    }
                });
                ui.separator();
                let enabled = self.configs.iter().any(|c| c.enabled);
                item(ui, actions, enabled, "Reload configs from their files", "", Action::ReapplyConfigs);
                item(ui, actions, !self.configs.is_empty(), "Remove all", "", Action::RemoveAllConfigs);
            });
            ui.menu_button("Live", |ui| {
                ui.set_min_width(240.0);
                if ui.checkbox(&mut self.settings.watch, "Watch cache live").changed() {
                    match self.settings.watch {
                        true => self.start_watch(ui.ctx()),
                        false => self.stop_watch(),
                    }
                    self.settings.save();
                }
                ui.separator();
                let (tab, label) = if self.tab == Tab::Live { (Tab::All, "Show all assets") } else { (Tab::Live, "Show the Live list") };
                item(ui, actions, true, label, "", Action::ShowTab(tab));
                item(ui, actions, self.tab == Tab::Live && self.sort != SortKey::Arrival, "Newest first", "", Action::SortByArrival);
                item(ui, actions, !self.arrivals.is_empty(), "Clear the Live list", "", Action::ClearLive);
                ui.separator();
                let applying = format::count(self.configs.iter().filter(|c| c.enabled).count(), "config", "configs");
                ui.weak(format!("{} since the list was last cleared", format::count(self.arrivals.len(), "arrival", "arrivals")));
                ui.weak(match self.watch.is_some() {
                    true => format!("{applying} put on new arrivals"),
                    false => format!("{applying} on, but not put on new arrivals while watching is off"),
                });
            });
            ui.menu_button("View", |ui| {
                ui.set_min_width(210.0);
                if ui.checkbox(&mut self.show_redirects, "Show redirect records").changed() {
                    self.rebuild_view();
                }
                let mut changed = ui.checkbox(&mut self.settings.animations, "Animations").changed();
                if self.tray.is_some() {
                    let hint = "Closing the window keeps rbxcache running in the notification area";
                    changed |= ui.checkbox(&mut self.settings.close_to_tray, "Close to tray").on_hover_text(hint).changed();
                }
                if changed {
                    self.settings.save();
                }
                ui.separator();
                ui.menu_button("Sort by", |ui| {
                    for (title, key) in COLUMNS {
                        if ui.radio(self.sort == key, title).clicked() {
                            actions.push(Action::SortBy(key));
                            ui.close_menu();
                        }
                    }
                    ui.separator();
                    if ui.checkbox(&mut self.ascending, "Ascending").changed() {
                        self.rebuild_view();
                    }
                });
                if ui.button("Clear filters").clicked() {
                    self.search.clear();
                    self.kind_filter = None;
                    self.category_filter = None;
                    self.rebuild_view();
                    ui.close_menu();
                }
            });
            ui.menu_button("Help", |ui| {
                item(ui, actions, true, "Info and shortcuts", "F1", Action::ShowTab(Tab::Info));
                item(ui, actions, self.update_check.is_none(), "Check for updates", "", Action::CheckUpdate);
            });
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if self.roblox_running {
                    theme::pill(ui, "Roblox is running - changes may be overwritten", theme::WARN);
                }
                ui.label(egui::RichText::new(self.dir.display().to_string()).color(theme::MUTED).small());
            });
        });
    }

    fn toolbar(&mut self, ui: &mut egui::Ui, actions: &mut Vec<Action>) {
        ui.horizontal(|ui| {
            if self.tab == Tab::Live
                && ui.add_enabled(!self.arrivals.is_empty(), egui::Button::new("Clear")).on_hover_text("Empty the Live list").clicked()
            {
                actions.push(Action::ClearLive);
            }
            if self.tab == Tab::Live && self.sort != SortKey::Arrival && ui.button("Newest first").on_hover_text("Go back to arrival order").clicked()
            {
                actions.push(Action::SortByArrival);
            }
            let search_id = egui::Id::new("search");
            let mut changed = ui
                .add(egui::TextEdit::singleline(&mut self.search).id(search_id).hint_text("Search id, type or URL   (Ctrl+F)").desired_width(300.0))
                .changed();
            if std::mem::take(&mut self.focus_search) {
                ui.memory_mut(|m| m.request_focus(search_id));
            }
            let kind_text = self.kind_filter.unwrap_or("All types");
            egui::ComboBox::from_id_salt("kind").selected_text(kind_text).height(420.0).show_ui(ui, |ui| {
                changed |= ui.selectable_value(&mut self.kind_filter, None, "All types").changed();
                for &(kind, n) in self.kinds.iter().filter(|(kind, _)| self.tab.holds(kind)) {
                    changed |= ui.selectable_value(&mut self.kind_filter, Some(kind), format!("{kind}  ({n})")).changed();
                }
            });
            let category_text = self.category_filter.map_or("All categories".to_string(), |c| format!("{c} {}", category_name(c)));
            egui::ComboBox::from_id_salt("category").selected_text(category_text).show_ui(ui, |ui| {
                changed |= ui.selectable_value(&mut self.category_filter, None, "All categories").changed();
                for &(c, n) in &self.categories {
                    changed |= ui.selectable_value(&mut self.category_filter, Some(c), format!("{c} {}  ({n})", category_name(c))).changed();
                }
            });
            if changed {
                self.rebuild_view();
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let text = format!("{} of {} shown  -  {}", self.view.len(), self.entries.len(), human_size(self.view_bytes));
                ui.label(egui::RichText::new(text).color(theme::MUTED));
            });
        });
    }

    fn details(&mut self, ui: &mut egui::Ui, actions: &mut Vec<Action>) {
        // Keep the panel at its dragged width instead of shrinking to the content.
        ui.set_min_width(ui.available_width());
        if self.selected.len() > 1 {
            ui.heading(format!("{} entries selected", self.selected.len()));
            ui.label(egui::RichText::new(format!("{} in total", human_size(self.selected_bytes()))).color(theme::MUTED));
            ui.horizontal(|ui| {
                if ui.button("Export selected to folder...").clicked() {
                    actions.push(Action::ExportMany);
                }
                if ui.button(egui::RichText::new("Delete").color(theme::DANGER)).clicked() {
                    actions.push(Action::AskDelete);
                }
            });
            ui.separator();
        }
        let Some(preview) = &mut self.preview else {
            if self.selected.is_empty() {
                ui.add_space(ui.available_height() * 0.32);
                ui.vertical_centered(|ui| {
                    ui.label(egui::RichText::new("Nothing selected").size(18.0).color(theme::TEXT));
                    ui.add_space(4.0);
                    ui.label(egui::RichText::new("Pick an entry to see its details and preview.").color(theme::MUTED));
                    ui.label(egui::RichText::new("Ctrl-click and Shift-click select several. Right-click for actions.").color(theme::MUTED));
                });
            }
            return;
        };

        // Each newly focused entry fades in while sliding up into place.
        let now = ui.input(|i| i.time);
        let t = if self.settings.animations { theme::ease_out(((now - self.panel_time) as f32) / PANEL_TIME) } else { 1.0 };
        if t < 1.0 {
            ui.ctx().request_repaint();
        }
        ui.set_opacity(0.15 + 0.85 * t);
        let slide = (1.0 - t) * 22.0;

        if self.selected.len() <= 1 {
            let entry = self.focus.and_then(|id| self.index.get(&id)).map(|&i| &self.entries[i]);
            if let Some(e) = entry {
                ui.horizontal(|ui| {
                    theme::pill(ui, e.kind, theme::kind_color(e.kind));
                    ui.label(egui::RichText::new(human_size(e.size)).strong());
                    ui.label(egui::RichText::new(&e.id_hex[..12]).monospace().color(theme::MUTED));
                    match self.tracker.reads(&e.id) {
                        Some(0) => theme::pill(ui, "replaced, not read by Roblox yet", theme::WARN),
                        Some(reads) => theme::pill(ui, &format!("replaced, read {reads}x by Roblox"), theme::OK),
                        None => {}
                    }
                });
            }
            ui.horizontal_wrapped(|ui| {
                if ui.button("Export...").clicked() {
                    actions.push(Action::Export);
                }
                if let Some(label) = entry.and_then(|e| match e.kind {
                    "ktx2" => Some("Export PNG..."),
                    "mesh" => Some("Export OBJ..."),
                    _ => None,
                }) {
                    if ui.button(label).on_hover_text("Convert to a common format and save").clicked() {
                        actions.push(Action::ExportConverted);
                    }
                }
                if ui.button("Replace...").on_hover_text("Swap the payload for a file of your choice").clicked() {
                    actions.push(Action::Replace);
                }
                if ui.button("Open").on_hover_text("Open a copy in the default program").clicked() {
                    actions.push(Action::OpenExternally);
                }
                if ui.button("Copy ID").clicked() {
                    actions.push(Action::CopyId);
                }
                if ui.button(egui::RichText::new("Delete").color(theme::DANGER)).clicked() {
                    actions.push(Action::AskDelete);
                }
            });
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new("Asset ID").color(theme::MUTED));
                let field = ui.add(egui::TextEdit::singleline(&mut self.tag_input).hint_text("e.g. 14147882149").desired_width(160.0));
                let entered = field.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                if ui.button("Tag").on_hover_text("Remember that this entry is that Roblox asset, so configs can refer to it by ID").clicked()
                    || entered
                {
                    actions.push(Action::Tag);
                }
            });
            ui.separator();
        }
        let (motion, audio) = (self.settings.animations, &mut self.audio);
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            ui.add_space(slide);
            if let Some(error) = &preview.error {
                ui.colored_label(theme::DANGER, error);
            }
            if preview.deferred {
                ui.label(egui::RichText::new("This file is large, so its preview was not built automatically.").color(theme::MUTED));
                if ui.button("Load preview").clicked() {
                    actions.push(Action::ForcePreview);
                }
                ui.add_space(8.0);
            }
            if let Some(texture) = &preview.texture {
                let size = texture.size_vec2();
                let scale = (ui.available_width() / size.x).min(380.0 / size.y).min(8.0);
                let (rect, _) = ui.allocate_exact_size(size * scale, egui::Sense::hover());
                let uv = Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0));
                let painter = ui.painter();
                checkerboard(painter, rect);
                // Streak of fading copies below the image while it is still travelling.
                if t < 1.0 {
                    for k in (1..=4).rev() {
                        let offset = Vec2::new(0.0, slide * 0.55 * k as f32);
                        painter.image(texture.id(), rect.translate(offset), uv, Color32::from_white_alpha((34.0 * (1.0 - t)) as u8));
                    }
                }
                painter.image(texture.id(), rect, uv, Color32::from_white_alpha((255.0 * (0.15 + 0.85 * t)) as u8));
                ui.add_space(10.0);
            }
            if let Some(view) = &mut preview.mesh {
                mesh_view(ui, view, motion);
                ui.add_space(10.0);
            }
            if let (Some(view), Some(animation)) = (&mut preview.anim, preview.model.as_ref().and_then(|m| m.animation.as_ref())) {
                anim_view(ui, animation, view);
                ui.add_space(10.0);
            }
            if preview.audio {
                audio_controls(ui, audio.as_mut(), preview.id, actions);
                ui.add_space(10.0);
            }
            if let Some((family, registered)) = &preview.font {
                // A font added this pass only becomes usable on the next one.
                if ui.ctx().cumulative_pass_nr() > *registered {
                    for (size, sample) in [
                        (30.0, "Roblox Cache 0123"),
                        (20.0, "The quick brown fox jumps over the lazy dog"),
                        (14.0, "ABCDEFGHIJKLMNOPQRSTUVWXYZ abcdefghijklmnopqrstuvwxyz 0123456789 !?&@#"),
                    ] {
                        ui.add(egui::Label::new(egui::RichText::new(sample).font(egui::FontId::new(size, family.clone()))).wrap());
                    }
                } else {
                    ui.ctx().request_repaint();
                }
                ui.add_space(10.0);
            }
            let value_width = (ui.available_width() - 120.0).max(120.0);
            egui::Grid::new("facts").num_columns(2).spacing([14.0, 7.0]).max_col_width(value_width).show(ui, |ui| {
                for (key, value) in &preview.facts {
                    ui.label(egui::RichText::new(*key).color(theme::MUTED));
                    ui.add(egui::Label::new(value).wrap().selectable(true));
                    ui.end_row();
                }
            });
            if let Some(model) = &preview.model {
                ui.add_space(10.0);
                let classes: Vec<String> = model.classes.iter().map(|(c, n)| format!("{n} {c}")).collect();
                let title = if model.animation.is_some() { "Keyframes and poses" } else { "Model tree" };
                ui.label(egui::RichText::new(format!("{title}  -  {} instances", model.instances)).color(theme::MUTED));
                ui.label(egui::RichText::new(classes.join(",  ")).color(theme::MUTED).small());
                if model.truncated {
                    ui.label(egui::RichText::new("Only the first part of the tree is shown.").color(theme::WARN).small());
                }
                for (i, node) in model.roots.iter().enumerate() {
                    node_ui(ui, node, egui::Id::new(("model", i)), 0);
                }
            }
            if let Some(text) = &preview.text {
                ui.add_space(10.0);
                ui.label(egui::RichText::new("Text preview").color(theme::MUTED));
                ui.add(egui::TextEdit::multiline(&mut text.as_str()).font(egui::TextStyle::Monospace).desired_width(f32::INFINITY));
            }
            if !preview.hex.is_empty() {
                ui.add_space(10.0);
                ui.label(egui::RichText::new("First bytes of the payload").color(theme::MUTED));
                egui::ScrollArea::horizontal().id_salt("hex").show(ui, |ui| {
                    ui.add(egui::Label::new(egui::RichText::new(&preview.hex).monospace()).extend().selectable(true));
                });
            }
        });
    }

    fn table(&mut self, ui: &mut egui::Ui, actions: &mut Vec<Action>) {
        let mut clicked: Option<usize> = None;
        let now = ui.input(|i| i.time);
        let mut context_row: Option<usize> = None;
        let mut focus_rect: Option<Rect> = None;
        let mut body_clip: Option<Rect> = None;
        let (sort, ascending, focus) = (self.sort, self.ascending, self.focus);
        let (one, any) = (self.focus.is_some() && self.selected.len() <= 1, !self.selected.is_empty());
        // Reserved now so the focus highlight ends up underneath the row text.
        let painter = ui.painter().clone();
        let blob_slot = painter.add(Shape::Noop);

        let mut table = TableBuilder::new(ui)
            .striped(false)
            .resizable(true)
            .sense(egui::Sense::click())
            .cell_layout(egui::Layout::left_to_right(egui::Align::Center))
            .column(Column::initial(96.0).at_least(60.0))
            .column(Column::initial(120.0).at_least(60.0))
            .column(Column::initial(90.0).at_least(60.0))
            .column(Column::initial(60.0).at_least(40.0))
            .column(Column::initial(160.0).at_least(80.0))
            .column(Column::remainder().at_least(120.0))
            .min_scrolled_height(0.0);
        if let Some(row) = self.scroll_to.take() {
            table = table.scroll_to_row(row, None);
        }
        table
            .header(28.0, |mut header| {
                for (title, key) in COLUMNS {
                    header.col(|ui| {
                        let active = sort == key;
                        let color = if active { theme::TEXT } else { theme::MUTED };
                        let label = egui::Label::new(egui::RichText::new(title).color(color).strong()).sense(egui::Sense::click());
                        if ui.add(label).on_hover_text("Click to sort").clicked() {
                            actions.push(Action::SortBy(key));
                        }
                        if active {
                            let (rect, _) = ui.allocate_exact_size(Vec2::splat(8.0), egui::Sense::hover());
                            let (c, h) = (rect.center(), if ascending { -3.0 } else { 3.0 });
                            let points = vec![egui::pos2(c.x - 4.0, c.y - h), egui::pos2(c.x + 4.0, c.y - h), egui::pos2(c.x, c.y + h)];
                            ui.painter().add(Shape::convex_polygon(points, theme::ACCENT, Stroke::NONE));
                        }
                    });
                }
            })
            .body(|mut body| {
                body_clip = Some(body.ui_mut().clip_rect());
                body.rows(ROW_HEIGHT, self.view.len(), |mut row| {
                    let pos = row.index();
                    let e = &self.entries[self.view[pos]];
                    let is_focus = focus == Some(e.id);
                    // The focused row is highlighted by the animated blob instead.
                    row.set_selected(!is_focus && self.selected.contains(&e.id));
                    row.col(|ui| {
                        theme::pill(ui, e.kind, theme::kind_color(e.kind));
                        // A dot that fades out marks what Roblox has just written.
                        if let Some(age) = self.fresh.get(&e.id).map(|at| (now - at) as f32).filter(|age| *age < FRESH_SECONDS) {
                            let (dot, _) = ui.allocate_exact_size(Vec2::splat(8.0), egui::Sense::hover());
                            ui.painter().circle_filled(dot.center(), 3.5, alpha(theme::OK, 1.0 - age / FRESH_SECONDS));
                        }
                    });
                    row.col(|ui| {
                        ui.label(egui::RichText::new(format!("{} {}", e.category, category_name(e.category))).color(theme::MUTED));
                    });
                    row.col(|ui| {
                        ui.label(human_size(e.size));
                    });
                    row.col(|ui| {
                        ui.label(e.hits.to_string());
                    });
                    row.col(|ui| {
                        ui.label(egui::RichText::new(format_time_ms(e.atime)).color(theme::MUTED));
                    });
                    row.col(|ui| {
                        ui.label(egui::RichText::new(&e.id_hex).monospace());
                    });
                    let response = row.response();
                    if is_focus {
                        focus_rect = Some(response.rect);
                    }
                    if response.double_clicked() {
                        actions.push(Action::OpenExternally);
                    }
                    if response.clicked() {
                        clicked = Some(pos);
                    }
                    if response.secondary_clicked() {
                        context_row = Some(pos);
                    }
                    response.context_menu(|ui| {
                        ui.set_min_width(220.0);
                        entry_items(ui, actions, one, any, self.source.is_some());
                    });
                });
            });

        self.paint_blob(&painter, blob_slot, focus_rect, body_clip);
        if let Some(pos) = clicked {
            let modifiers = ui.input(|i| i.modifiers);
            self.click_row(pos, modifiers);
        }
        // Right-clicking a row outside the selection acts on that row alone.
        if let Some(pos) = context_row {
            if !self.selected.contains(&self.entries[self.view[pos]].id) {
                self.click_row(pos, egui::Modifiers::NONE);
            }
        }
    }

    /// Draws the focus highlight, gliding and stretching toward `target` when the focus moved.
    fn paint_blob(&mut self, painter: &egui::Painter, slot: egui::layers::ShapeIdx, target: Option<Rect>, clip: Option<Rect>) {
        let ctx = painter.ctx().clone();
        let (Some(row), Some(clip)) = (target, clip) else {
            self.blob.visible = false;
            self.blob.id = self.focus;
            return;
        };
        let top = row.top();
        let blob = &mut self.blob;
        if blob.id != self.focus {
            blob.id = self.focus;
            blob.moving = self.settings.animations && blob.visible;
        }
        if blob.moving {
            let dt = ctx.input(|i| i.stable_dt).min(0.05);
            blob.lead = theme::approach(blob.lead, top, 30.0, dt);
            blob.tail = theme::approach(blob.tail, top, 12.0, dt);
            if (blob.tail - top).abs() < 0.4 {
                blob.moving = false;
            }
            ctx.request_repaint();
        }
        if !blob.moving {
            // At rest the highlight is pinned to its row, so scrolling carries it along.
            (blob.lead, blob.tail) = (top, top);
        }
        blob.visible = true;

        let radius = CornerRadius::same(7);
        let at = |y: f32| Rect::from_min_size(egui::pos2(row.left(), y), row.size());
        let mut shapes = Vec::new();
        let stretch = blob.lead - blob.tail;
        if stretch.abs() > 0.5 {
            // The stretched body between the two edges, plus a fading trail behind the leading one.
            let body = Rect::from_min_max(
                egui::pos2(row.left(), blob.lead.min(blob.tail)),
                egui::pos2(row.right(), blob.lead.max(blob.tail) + row.height()),
            );
            shapes.push(Shape::rect_filled(body, radius, alpha(theme::ACCENT, 0.10)));
            let strength = (stretch.abs() / 90.0).clamp(0.15, 1.0);
            shapes.push(theme::motion_trail(at(blob.lead), Vec2::new(0.0, -stretch), theme::ACCENT, radius, strength));
        }
        let lead = at(blob.lead);
        shapes.push(Shape::rect_filled(lead, radius, alpha(theme::ACCENT, 0.26)));
        let bar = Rect::from_min_size(lead.min + Vec2::new(0.0, 4.0), Vec2::new(3.0, lead.height() - 8.0));
        shapes.push(Shape::rect_filled(bar, CornerRadius::same(2), theme::ACCENT));
        painter.with_clip_rect(clip).set(slot, Shape::Vec(shapes));
    }

    /// Who is signed in, at the bottom right: their avatar and name, or a way to sign in.
    fn account_chip(&self, ui: &mut egui::Ui, actions: &mut Vec<Action>) {
        let (label, hint) = match self.accounts.active() {
            Some(account) => (
                if account.display.is_empty() { account.name.clone() } else { account.display.clone() },
                "Your Roblox account: open the Account page",
            ),
            None => ("Sign in".to_string(), "Sign in with Roblox to fetch sounds and other assets guests cannot get"),
        };
        let avatar = self.accounts.active().and_then(|account| self.avatars.get(&account.user_id));
        let response = match avatar {
            Some(texture) => {
                let picture = egui::Image::new(texture).fit_to_exact_size(Vec2::splat(18.0)).corner_radius(CornerRadius::same(9));
                ui.add(egui::Button::image_and_text(picture, egui::RichText::new(label).size(12.5)))
            }
            None => ui.add(egui::Button::new(egui::RichText::new(label).size(12.5))),
        };
        if response.on_hover_text(hint).clicked() {
            actions.push(Action::ShowTab(Tab::Account));
        }
    }

    /// The sidebar: the two halves of the app, Browse and Manage.
    fn sidebar(&mut self, ui: &mut egui::Ui, actions: &mut Vec<Action>) {
        let now = ui.input(|i| i.time);
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            ui.add_space(10.0);
            ui.label(egui::RichText::new("pirxcy").font(egui::FontId::new(27.0, theme::wordmark_font())).color(theme::SPLASH_INK));
        });
        ui.horizontal(|ui| {
            ui.add_space(11.0);
            ui.label(egui::RichText::new(format!("rbxcache {}", update::CURRENT)).color(theme::MUTED).small());
        });
        ui.add_space(16.0);

        let enabled = self.configs.iter().filter(|c| c.enabled).count();
        let browsing = self.tab.lists();
        let rows = [
            NavRow { label: "Browse", hint: "What is in the cache", count: Some(self.listed()), selected: browsing, opens: self.last_browse },
            NavRow {
                label: "Manage",
                hint: "Configs, packs and help",
                count: (enabled > 0).then_some(enabled),
                selected: !browsing,
                opens: self.last_manage,
            },
        ];
        ui.spacing_mut().item_spacing.y = 4.0;
        let highlight = ui.painter().add(Shape::Noop);
        let mut active = None;
        for (index, row) in rows.iter().enumerate() {
            let rect = self.nav_row(ui, now, index, row, actions);
            if row.selected {
                active = Some(rect);
            }
        }
        self.paint_nav_highlight(ui, highlight, active);

        ui.with_layout(egui::Layout::bottom_up(egui::Align::Min), |ui| {
            ui.add_space(4.0);
            let state = |ui: &mut egui::Ui, on: bool, text: &str| {
                ui.horizontal(|ui| {
                    ui.add_space(10.0);
                    let (dot, _) = ui.allocate_exact_size(Vec2::splat(8.0), egui::Sense::hover());
                    ui.painter().circle_filled(dot.center(), 3.0, if on { theme::OK } else { alpha(theme::MUTED, 0.5) });
                    ui.label(egui::RichText::new(text).color(theme::MUTED).small());
                });
            };
            state(ui, self.watch.is_some(), if self.watch.is_some() { "Watching the cache" } else { "Not watching" });
            state(ui, self.roblox_running, if self.roblox_running { "Roblox is running" } else { "Roblox is closed" });
            if let Some(release) = &self.update {
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    ui.add_space(8.0);
                    let label = egui::RichText::new(format!("Update to {}", release.version)).color(theme::LIME);
                    if ui.button(label).on_hover_text("See what is new").clicked() {
                        actions.push(Action::ShowTab(Tab::Info));
                    }
                });
            }
        });
    }

    /// How many entries the All list has: redirect records only count while they are listed.
    fn listed(&self) -> usize {
        let redirects = self.kinds.iter().find(|(kind, _)| *kind == "redirect").map_or(0, |(_, n)| *n);
        self.entries.len() - if self.show_redirects { 0 } else { redirects }
    }

    /// One row of the sidebar, sliding in when the window opens. Returns where it is.
    fn nav_row(&mut self, ui: &mut egui::Ui, now: f64, index: usize, row: &NavRow, actions: &mut Vec<Action>) -> Rect {
        let seen = *self.nav_seen.entry(row.label).or_insert(now);
        let t = if self.settings.animations { theme::ease_out(((now - seen) as f32 - index as f32 * 0.06) / 0.4) } else { 1.0 };
        if t < 1.0 {
            ui.ctx().request_repaint();
        }
        let (rect, response) = ui.allocate_exact_size(Vec2::new(ui.available_width(), NAV_ROW), egui::Sense::click());
        let response = response.on_hover_cursor(egui::CursorIcon::PointingHand);
        let painter = ui.painter();
        if response.hovered() && !row.selected {
            painter.rect_filled(rect.shrink2(Vec2::new(6.0, 1.0)), CornerRadius::same(8), alpha(theme::RAISED, 0.7));
        }
        let left = rect.left() + 20.0 - (1.0 - t) * 16.0;
        let ink = if row.selected { Color32::WHITE } else { theme::TEXT };
        painter.text(egui::pos2(left, rect.center().y - 8.0), egui::Align2::LEFT_CENTER, row.label, egui::FontId::proportional(15.0), alpha(ink, t));
        painter.text(
            egui::pos2(left, rect.center().y + 10.0),
            egui::Align2::LEFT_CENTER,
            row.hint,
            egui::FontId::proportional(11.5),
            alpha(theme::MUTED, t),
        );
        if let Some(count) = row.count {
            let at = egui::pos2(rect.right() - 16.0, rect.center().y - 8.0);
            painter.text(at, egui::Align2::RIGHT_CENTER, count.to_string(), egui::FontId::proportional(12.0), alpha(theme::MUTED, t));
        }
        if response.clicked() {
            actions.push(Action::ShowTab(row.opens));
        }
        rect
    }

    /// The lists Browse offers, as a row of chips: everything, what is new,
    /// and one for each kind of asset the cache holds.
    fn browse_chips(&mut self, ui: &mut egui::Ui, actions: &mut Vec<Action>) {
        let mut chips = vec![(Tab::All, "All", Some(self.listed()), theme::MUTED), (Tab::Live, "Live", Some(self.arrivals.len()), theme::OK)];
        for (i, (name, kinds)) in GROUPS.iter().enumerate() {
            if self.group_counts[i] > 0 {
                chips.push((Tab::Group(i), name, Some(self.group_counts[i]), theme::kind_color(kinds[0])));
            }
        }
        if self.other_count > 0 {
            chips.push((Tab::Other, "Other", Some(self.other_count), theme::MUTED));
        }
        self.chips(ui, &chips, actions);
    }

    /// The pages of Manage, as a row of chips.
    fn manage_chips(&mut self, ui: &mut egui::Ui, actions: &mut Vec<Action>) {
        let enabled = self.configs.iter().filter(|c| c.enabled).count();
        let chips = [
            (Tab::Configs, "Configs", (enabled > 0).then_some(enabled), theme::ACCENT),
            (Tab::Packs, "Packs", Some(self.catalog.packs.len()), theme::LIME),
            (Tab::Lookup, "IDs", None, if self.accounts.is_empty() { theme::MUTED } else { theme::WARN }),
            (Tab::Account, "Account", None, if self.accounts.is_empty() { theme::MUTED } else { theme::OK }),
            (Tab::Guide, "Guide", None, theme::OK),
            (Tab::Info, "Info", None, if self.update.is_some() { theme::LIME } else { theme::MUTED }),
        ];
        self.chips(ui, &chips, actions);
    }

    /// A row of tabs. A chip fades in when it first appears, which for the
    /// kinds of asset is when the cache has been read.
    fn chips(&mut self, ui: &mut egui::Ui, chips: &[(Tab, &'static str, Option<usize>, Color32)], actions: &mut Vec<Action>) {
        let now = ui.input(|i| i.time);
        egui::ScrollArea::horizontal().id_salt(chips[0].1).show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 6.0;
                for (index, &(tab, label, count, colour)) in chips.iter().enumerate() {
                    let seen = *self.nav_seen.entry(label).or_insert(now);
                    let t = if self.settings.animations { theme::ease_out(((now - seen) as f32 - index as f32 * 0.03) / 0.35) } else { 1.0 };
                    if t < 1.0 {
                        ui.ctx().request_repaint();
                    }
                    if chip(ui, label, count, colour, self.tab == tab, t).clicked() {
                        actions.push(Action::ShowTab(tab));
                    }
                }
            });
        });
    }

    /// The marker behind the open tab's row. It glides to a newly opened tab, smearing on the way.
    fn paint_nav_highlight(&mut self, ui: &egui::Ui, slot: egui::layers::ShapeIdx, active: Option<Rect>) {
        let Some(row) = active else { return };
        let dt = ui.input(|i| i.stable_dt).min(0.05);
        let mut y = match self.nav_y {
            Some(y) if self.settings.animations => theme::approach(y, row.top(), 22.0, dt),
            _ => row.top(),
        };
        let left = row.top() - y;
        if left.abs() < 0.4 {
            y = row.top();
        } else {
            ui.ctx().request_repaint();
        }
        self.nav_y = Some(y);
        let radius = CornerRadius::same(8);
        let at = Rect::from_min_size(egui::pos2(row.left() + 6.0, y), Vec2::new(row.width() - 12.0, row.height()));
        let mut shapes = Vec::new();
        if left.abs() > 1.0 {
            shapes.push(theme::motion_trail(
                at,
                Vec2::new(0.0, -left.clamp(-60.0, 60.0) * 0.5),
                theme::ACCENT,
                radius,
                (left.abs() / 50.0).clamp(0.2, 1.0),
            ));
        }
        shapes.push(Shape::rect_filled(at, radius, alpha(theme::ACCENT, 0.22)));
        let bar = Rect::from_min_size(at.min + Vec2::new(0.0, 10.0), Vec2::new(3.0, at.height() - 20.0));
        shapes.push(Shape::rect_filled(bar, CornerRadius::same(2), theme::LIME));
        ui.painter().set(slot, Shape::Vec(shapes));
    }

    /// Lays out one of Manage's pages under its row of tabs, in a centred column.
    fn page(&mut self, ui: &mut egui::Ui, actions: &mut Vec<Action>) {
        let width = ui.available_width().min(PAGE_WIDTH);
        let margin = ((ui.available_width() - width) / 2.0).max(0.0);
        ui.add_space(12.0);
        ui.horizontal(|ui| {
            ui.add_space(margin);
            self.manage_chips(ui, actions);
        });
        ui.add_space(6.0);
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            ui.horizontal_top(|ui| {
                ui.add_space(margin);
                ui.vertical(|ui| {
                    ui.set_width(width);
                    ui.add_space(8.0);
                    match self.tab {
                        Tab::Configs => self.configs_page(ui, actions),
                        Tab::Packs => self.packs_page(ui, actions),
                        Tab::Lookup => self.lookup_page(ui, actions),
                        Tab::Account => self.account_page(ui, actions),
                        Tab::Guide => self.guide_page(ui, actions),
                        _ => self.info_page(ui, actions),
                    }
                    ui.add_space(24.0);
                });
            });
        });
    }

    /// Title and one line of explanation at the top of a page.
    fn page_header(&self, ui: &mut egui::Ui, title: &str, about: &str) {
        let t = self.entrance(ui.ctx(), 0);
        ui.scope(|ui| {
            ui.set_opacity(t);
            ui.add_space((1.0 - t) * 12.0);
            ui.label(egui::RichText::new(title).font(egui::FontId::new(30.0, theme::wordmark_font())).color(theme::SPLASH_INK));
            ui.add(egui::Label::new(egui::RichText::new(about).color(theme::MUTED)).wrap());
        });
        ui.add_space(10.0);
    }

    fn configs_page(&mut self, ui: &mut egui::Ui, actions: &mut Vec<Action>) {
        let signed_in = !self.accounts.is_empty();
        self.page_header(
            ui,
            "Configs",
            "Replacement rules that stay applied. What is switched on is put on whatever Roblox downloads; what is switched off is put back.",
        );
        let idle = self.busy().is_none();
        let t = self.entrance(ui.ctx(), 1);
        ui.scope(|ui| {
            ui.set_opacity(t);
            ui.horizontal(|ui| {
                if ui.add_enabled(idle, primary_button("Add configs...")).clicked() {
                    actions.push(Action::AddConfigs);
                }
                let any = self.configs.iter().any(|c| c.enabled);
                if ui
                    .add_enabled(idle && any, egui::Button::new("Reload from files"))
                    .on_hover_text("Read the config files again and apply them to the whole cache")
                    .clicked()
                {
                    actions.push(Action::ReapplyConfigs);
                }
                if self.apply_job.is_some() || self.sync_wanted {
                    ui.spinner();
                    ui.label(egui::RichText::new("Applying").color(theme::MUTED));
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if self.watch.is_some() {
                        theme::pill(ui, "Watching: new downloads are replaced too", theme::OK);
                    } else {
                        theme::pill(ui, "Not watching: new downloads are left alone", theme::WARN);
                    }
                });
            });
        });
        ui.add_space(8.0);

        if self.configs.is_empty() {
            card(ui, self.entrance(ui.ctx(), 2), |ui| {
                ui.vertical_centered(|ui| {
                    ui.add_space(18.0);
                    ui.label(egui::RichText::new("No configs yet").size(18.0));
                    ui.label(
                        egui::RichText::new("Add a config file of your own, or pick a pack. The Guide tab walks through it.").color(theme::MUTED),
                    );
                    ui.add_space(18.0);
                });
            });
            return;
        }
        let waiting: usize = self.configs.iter().filter(|c| c.enabled).map(|c| id_stats(&c.rules, &self.idmap).2).sum();
        if waiting > 0 && !signed_in {
            card(ui, self.entrance(ui.ctx(), 2), |ui| {
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new("Locked until you sign in").size(16.0).strong().color(theme::WARN));
                    theme::pill(ui, &format::count(waiting, "asset ID", "asset IDs"), theme::WARN);
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.add(primary_button("Sign in")).clicked() {
                            actions.push(Action::ShowTab(Tab::Account));
                        }
                    });
                });
                let why = "Sounds, and anything else Roblox only gives to signed-in players, cannot be fetched without an account, so the rules \
                           that name them do nothing for now. Everything else in your configs works as before.";
                ui.add(egui::Label::new(egui::RichText::new(why).color(theme::MUTED)).wrap());
            });
        }
        self.conflicts_card(ui, actions);
        for (i, config) in self.configs.iter().enumerate() {
            card(ui, self.entrance(ui.ctx(), i + 3), |ui| {
                ui.horizontal(|ui| {
                    if switch(ui, config.enabled).on_hover_text("Off puts back everything this config replaced").clicked() {
                        actions.push(Action::ToggleConfig(i));
                    }
                    ui.add_space(4.0);
                    ui.label(egui::RichText::new(&config.name).size(16.0).strong());
                    match &config.error {
                        Some(_) => theme::pill(ui, "unreadable", theme::WARN),
                        None => {
                            let on = config.rules.len() - config.off.len().min(config.rules.len());
                            theme::pill(ui, &format!("{on} of {} on", format::count(config.rules.len(), "rule", "rules")), theme::ACCENT);
                        }
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.button("Remove").on_hover_text("Stop using this config and put back what it replaced").clicked() {
                            actions.push(Action::RemoveConfig(i));
                        }
                    });
                });
                ui.add(egui::Label::new(egui::RichText::new(config.path.display().to_string()).color(theme::MUTED).small()).truncate());
                if let Some(error) = &config.error {
                    ui.add(egui::Label::new(egui::RichText::new(error).color(theme::WARN).small()).wrap());
                    return;
                }
                let (ids, known, refused) = id_stats(&config.rules, &self.idmap);
                if ids > 0 {
                    let mut text = format!("{}: {known} identified", format::count(ids, "asset ID", "asset IDs"));
                    if refused > 0 {
                        text.push_str(&format!(", {refused} that Roblox would not hand out"));
                    }
                    if ids > known + refused {
                        text.push_str(&format!(", {} not looked up yet", ids - known - refused));
                    }
                    ui.add(egui::Label::new(egui::RichText::new(text).color(theme::MUTED).small()).wrap());
                    if refused > 0 {
                        if signed_in {
                            let hint = "Ask again as your account, and pick by ear what Roblox still refuses";
                            if ui.add_enabled(idle, egui::Button::new("Identify them")).on_hover_text(hint).clicked() {
                                actions.push(Action::LookupFor(i));
                            }
                        } else if ui.button("Sign in to use them").on_hover_text("Roblox only gives these to a signed-in player").clicked() {
                            actions.push(Action::ShowTab(Tab::Account));
                        }
                    }
                }
                let keys = rule_keys(&config.rules);
                egui::CollapsingHeader::new(egui::RichText::new("Rules").color(theme::MUTED)).id_salt(("rules", &config.path)).show(ui, |ui| {
                    egui::ScrollArea::vertical().id_salt(("rule list", &config.path)).max_height(300.0).show(ui, |ui| {
                        for (rule_index, (rule, key)) in config.rules.iter().zip(&keys).enumerate() {
                            ui.horizontal(|ui| {
                                if switch(ui, config.enabled && !config.off.contains(key)).clicked() {
                                    actions.push(Action::ToggleRule(i, rule_index));
                                }
                                ui.label(&rule.name);
                                ui.label(egui::RichText::new(rule_summary(rule)).color(theme::MUTED).small());
                            });
                        }
                    });
                });
            });
        }
    }

    /// Where enabled configs set the same thing, lets the user say which one has it.
    fn conflicts_card(&self, ui: &mut egui::Ui, actions: &mut Vec<Action>) {
        let found = conflicts(&self.configs);
        if found.is_empty() {
            return;
        }
        // Conflicts between the same configs are shown together.
        let mut groups: BTreeMap<Vec<usize>, Vec<&Conflict>> = BTreeMap::new();
        for conflict in &found {
            groups.entry(conflict.sides.iter().map(|(config, _)| *config).collect()).or_default().push(conflict);
        }
        card(ui, self.entrance(ui.ctx(), 2), |ui| {
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new("Conflicts").size(16.0).strong());
                theme::pill(ui, &found.len().to_string(), theme::WARN);
            });
            let about = "These configs change the same things. Each one can only go to one config: pick which, for all of them or one by one.";
            ui.add(egui::Label::new(egui::RichText::new(about).color(theme::MUTED)).wrap());
            for (sides, list) in &groups {
                ui.add_space(6.0);
                let names: Vec<&str> = sides.iter().map(|config| self.configs[*config].name.as_str()).collect();
                ui.label(format!("{}: {} in common", names.join(" and "), list.len()));
                ui.horizontal_wrapped(|ui| {
                    for &config in sides {
                        let all = list.iter().all(|conflict| conflict.winner(&self.configs, &self.settings.winners) == config);
                        let label = format!("All from {}", self.configs[config].name);
                        if ui.selectable_label(all, label).clicked() {
                            actions.push(Action::Choose(list.iter().map(|conflict| conflict.target.clone()).collect(), config));
                        }
                    }
                });
                egui::CollapsingHeader::new(egui::RichText::new("One by one").color(theme::MUTED)).id_salt(("conflicts", sides)).show(ui, |ui| {
                    egui::ScrollArea::vertical().id_salt(("conflict list", sides)).max_height(260.0).show(ui, |ui| {
                        for conflict in list {
                            let winner = conflict.winner(&self.configs, &self.settings.winners);
                            ui.horizontal_wrapped(|ui| {
                                ui.label(egui::RichText::new(&conflict.target).monospace().color(theme::MUTED).small());
                                for (config, rule) in &conflict.sides {
                                    let label = format!("{}: {rule}", self.configs[*config].name);
                                    if ui.selectable_label(winner == *config, label).clicked() {
                                        actions.push(Action::Choose(vec![conflict.target.clone()], *config));
                                    }
                                }
                            });
                        }
                    });
                });
            }
        });
    }

    fn account_page(&mut self, ui: &mut egui::Ui, actions: &mut Vec<Action>) {
        self.page_header(
            ui,
            "Account",
            "Add a Roblox account by its cookie to unlock fetching IDs and replacing sounds, which Roblox only allows for signed-in players.",
        );
        let bold_red = |ui: &mut egui::Ui, text: &str| {
            ui.add(egui::Label::new(egui::RichText::new(text).strong().size(15.0).color(theme::DANGER)).wrap());
        };
        card(ui, self.entrance(ui.ctx(), 1), |ui| {
            bold_red(ui, "This can be a main account. An alternate account is recommended.");
            bold_red(ui, "NEVER paste anyone else's cookie, and never paste this value anywhere other than rbxcache.");
            ui.add_space(4.0);
            let text = "A throwaway account works exactly as well as your main one: it only has to exist and be able to sign in. \
                        Whoever holds this cookie can act as that account, including spending its Robux, so an alt with nothing of \
                        value on it is what to use.";
            ui.add(egui::Label::new(egui::RichText::new(text).color(theme::TEXT)).wrap());
        });
        card(ui, self.entrance(ui.ctx(), 2), |ui| {
            ui.label(egui::RichText::new("Why an account").size(16.0).strong());
            for (title, text) in [
                (
                    "Why it is needed",
                    "Roblox only hands most sounds to signed-in players. Without an account rbxcache cannot download a sound to compare it \
                     with your cache, so it cannot tell which cached sound belongs to which ID, and the sound rules in a config do nothing.",
                ),
                (
                    "What is pasted",
                    "rbxcache has no Roblox login page of its own. You copy your account's .ROBLOSECURITY cookie out of your own, already \
                     signed-in browser and paste it here; rbxcache never reads it from your browser or from Roblox's own files by itself.",
                ),
                (
                    "Where it is kept",
                    "Encrypted for your Windows user, in accounts.json in the data folder, so the file is no use on another user or computer. \
                     Remove deletes it from this computer at once.",
                ),
                (
                    "Ending the session",
                    "Remove only deletes the cookie from this computer; it does not end the Roblox session itself, since there is no way to \
                     do that with a cookie alone. To be sure a cookie can never be used again, sign that account out on roblox.com.",
                ),
                ("What works without it", "Browsing, previews, exporting, replacing and deleting entries, and configs for images, fonts, meshes and anything else Roblox gives to everyone. Fetching IDs and replacing sounds need an account, and stay locked until one is added."),
            ] {
                ui.add_space(4.0);
                ui.label(egui::RichText::new(title).color(theme::LIME));
                ui.add(egui::Label::new(text).wrap());
            }
        });

        let active = self.accounts.active().map(|a| a.user_id);
        for (index, account) in self.accounts.accounts.iter().enumerate() {
            card(ui, self.entrance(ui.ctx(), index + 3), |ui| {
                ui.horizontal(|ui| {
                    match self.avatars.get(&account.user_id) {
                        Some(texture) => {
                            ui.add(egui::Image::new(texture).fit_to_exact_size(Vec2::splat(52.0)).corner_radius(CornerRadius::same(26)));
                        }
                        None => {
                            let (rect, _) = ui.allocate_exact_size(Vec2::splat(52.0), egui::Sense::hover());
                            ui.painter().circle_filled(rect.center(), 26.0, theme::RAISED);
                        }
                    }
                    ui.vertical(|ui| {
                        ui.horizontal(|ui| {
                            let name = if account.display.is_empty() { &account.name } else { &account.display };
                            ui.label(egui::RichText::new(name).size(16.0).strong());
                            if active == Some(account.user_id) {
                                theme::pill(ui, "In use", theme::OK);
                            }
                        });
                        ui.label(egui::RichText::new(format!("@{}  -  {}", account.name, account.user_id)).color(theme::MUTED).small());
                    });
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let out = egui::RichText::new("Remove").color(theme::DANGER).strong();
                        if ui.button(out).on_hover_text("Delete this cookie from this computer. Does not end its Roblox session.").clicked() {
                            actions.push(Action::SignOut(account.user_id));
                        }
                        if active != Some(account.user_id) && ui.button("Use this account").clicked() {
                            actions.push(Action::UseAccount(account.user_id));
                        }
                    });
                });
            });
        }

        card(ui, self.entrance(ui.ctx(), self.accounts.accounts.len() + 3), |ui| {
            let label = if self.accounts.is_empty() { "Add an account" } else { "Add another account" };
            ui.label(egui::RichText::new(label).size(16.0).strong());
            bold_red(ui, "This can be a main account. An alternate account is recommended.");
            let hint = "How to copy it: in your browser, with that account signed in, open developer tools (F12), go to \
                        Application (or Storage) > Cookies > https://www.roblox.com, find .ROBLOSECURITY, and copy its value.";
            ui.add(egui::Label::new(egui::RichText::new(hint).color(theme::MUTED).small()).wrap());
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                let field = ui
                    .add(egui::TextEdit::singleline(&mut self.account_input).password(true).hint_text(".ROBLOSECURITY cookie").desired_width(420.0));
                let entered = field.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                let busy = self.account_job.is_some();
                if (ui.add_enabled(!busy, primary_button("Check and add")).clicked() || entered) && !busy {
                    actions.push(Action::AskAddAccount);
                }
                if busy {
                    ui.spinner();
                    ui.label(egui::RichText::new("Checking with Roblox").color(theme::MUTED));
                }
            });
            if let Some(error) = &self.account_error {
                ui.add_space(4.0);
                ui.add(egui::Label::new(egui::RichText::new(error).color(theme::WARN)).wrap());
            }
        });
    }

    fn lookup_page(&mut self, ui: &mut egui::Ui, actions: &mut Vec<Action>) {
        self.page_header(
            ui,
            "IDs",
            "Type a Roblox asset ID to see what it is, get the file when Roblox hands it out, and tie it to the entry in your cache.",
        );
        if self.accounts.is_empty() {
            card(ui, self.entrance(ui.ctx(), 1), |ui| {
                ui.label(egui::RichText::new("Locked until an account is added").size(17.0).strong().color(theme::WARN));
                let why = "Fetching an ID asks Roblox for the asset as a signed-in player. Sounds and many other assets are only given to them, so \
                           without an account there is nothing to fetch. Browsing, previews and the rest of rbxcache work as before.";
                ui.add(egui::Label::new(egui::RichText::new(why).color(theme::TEXT)).wrap());
                ui.add(
                    egui::Label::new(
                        egui::RichText::new("This can be a main account. An alternate account is recommended.").strong().color(theme::DANGER),
                    )
                    .wrap(),
                );
                ui.add_space(4.0);
                if ui.add(primary_button("Go to the Account page")).clicked() {
                    actions.push(Action::ShowTab(Tab::Account));
                }
            });
            return;
        }
        let idle = self.busy().is_none();
        ui.horizontal(|ui| {
            let field = ui.add(
                egui::TextEdit::singleline(&mut self.lookup_input).hint_text("Asset IDs, links or rbxassetid:// addresses").desired_width(420.0),
            );
            let entered = field.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
            if (ui.add_enabled(idle, primary_button("Fetch")).clicked() || entered) && idle {
                actions.push(Action::RunLookup);
            }
            if !self.lookup_found.is_empty()
                && ui
                    .button("Copy fingerprints")
                    .on_hover_text("What is known about these IDs, in the form the fingerprints section of packs.json takes")
                    .clicked()
            {
                actions.push(Action::CopyFingerprints);
            }
        });
        if let Some(who) = self.accounts.active() {
            let name = if who.display.is_empty() { &who.name } else { &who.display };
            let note =
                format!("Fetches are sent as {name} (@{}). Each fetch first checks with Roblox that this account is still signed in.", who.name);
            ui.label(egui::RichText::new(note).color(theme::MUTED).small());
        }
        ui.add_space(8.0);
        if self.lookup_found.is_empty() {
            card(ui, self.entrance(ui.ctx(), 1), |ui| {
                let about = "Images, meshes, fonts and other public assets are downloaded as a guest, shown here, and matched to your cache. \
                             Roblox does not hand most sounds out that way. With an account signed in on the Account tab they are asked for again as \
                             that account. What is still refused is listed as the cached sounds of the right length: play them, pick the one it is, \
                             and configs that name the ID will work from then on.";
                ui.add(egui::Label::new(egui::RichText::new(about).color(theme::MUTED)).wrap());
            });
            return;
        }
        let playing = self.audio.as_ref().filter(|a| a.active() && !a.paused()).and_then(|a| a.current);
        if playing.is_some() {
            ui.ctx().request_repaint_after(std::time::Duration::from_millis(200));
        }
        for (index, found) in self.lookup_found.iter().enumerate() {
            card(ui, self.entrance(ui.ctx(), index + 1), |ui| {
                let details = found.details.clone().unwrap_or_default();
                ui.horizontal(|ui| {
                    let name = if details.name.is_empty() { format!("Asset {}", found.id) } else { details.name.clone() };
                    ui.label(egui::RichText::new(name).size(16.0).strong());
                    if let Some(kind) = details.type_id {
                        theme::pill(ui, format::asset_type_name(kind), theme::ACCENT);
                    }
                    ui.label(egui::RichText::new(found.id.to_string()).monospace().color(theme::MUTED));
                    if !details.creator.is_empty() {
                        ui.label(egui::RichText::new(format!("by {}", details.creator)).color(theme::MUTED));
                    }
                });
                if let Some((kind, body)) = &found.file {
                    ui.horizontal(|ui| {
                        theme::pill(ui, kind, theme::kind_color(kind));
                        ui.label(format!("Roblox handed it out: {}", human_size(body.len() as u64)));
                        if ui.button("Save as...").clicked() {
                            actions.push(Action::SaveFetched(index));
                        }
                        if format::is_audio(kind) {
                            let label = if playing == Some(fetched_id(found.id)) { "Pause" } else { "Play" };
                            if ui.button(label).clicked() {
                                actions.push(Action::PlayFetched(index));
                            }
                        }
                    });
                    if let Some(picture) = self.lookup_pictures.get(&found.id) {
                        let (rect, _) = ui.allocate_exact_size(picture.size_vec2().min(Vec2::splat(160.0)), egui::Sense::hover());
                        checkerboard(ui.painter(), rect);
                        ui.painter().image(picture.id(), rect, Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)), Color32::WHITE);
                    }
                }
                use resolve::Outcome;
                match &found.outcome {
                    Outcome::Known | Outcome::Exact | Outcome::Rendition => {
                        ui.horizontal(|ui| {
                            theme::pill(ui, "In your cache", theme::OK);
                            ui.label(egui::RichText::new("Configs that name this ID will find it.").color(theme::MUTED));
                        });
                    }
                    Outcome::NotCached => {
                        ui.label(
                            egui::RichText::new("Known, but not in your cache right now. It will be recognised when Roblox downloads it.")
                                .color(theme::MUTED),
                        );
                    }
                    Outcome::Gone => drop(ui.colored_label(theme::WARN, "Roblox has no asset with this ID.")),
                    Outcome::Failed(why) => drop(ui.colored_label(theme::DANGER, format!("The lookup failed: {why}"))),
                    Outcome::Denied(_) => self.pick_by_ear(ui, found, playing, actions),
                }
            });
        }
    }

    /// For an ID Roblox would not hand out: the cached sounds it could be, to play and choose from.
    fn pick_by_ear(&self, ui: &mut egui::Ui, found: &Found, playing: Option<Id>, actions: &mut Vec<Action>) {
        ui.colored_label(theme::WARN, "Roblox does not hand this one out to a guest.");
        if self.accounts.is_empty() {
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new("Signing in with an account may get it directly.").color(theme::MUTED));
                if ui.small_button("Sign in").clicked() {
                    actions.push(Action::ShowTab(Tab::Account));
                }
            });
        } else {
            ui.label(egui::RichText::new("Asked again as each of its creator's games, and none of them handed it out.").color(theme::MUTED));
        }
        // With nothing known about it, a refused asset is taken to be a sound: those are the ones Roblox keeps back.
        if found.details.is_some() && !found.is_sound() {
            let how = "To tie it to an entry by hand: find the entry under Browse, type this ID in its Asset ID box and press Tag.";
            ui.add(egui::Label::new(egui::RichText::new(how).color(theme::MUTED)).wrap());
            return;
        }
        let about = found.details.as_ref().and_then(|d| d.seconds);
        let candidates: Vec<&Sound> = lookup::candidates(&self.lookup_sounds, about).into_iter().filter(|s| self.index.contains_key(&s.id)).collect();
        let length = about.map_or(String::new(), |seconds| format!(" about {seconds} s long"));
        if candidates.is_empty() {
            let none = format!("No cached sound{length} is free to be it. Play the game with rbxcache running so Roblox downloads the sound, then fetch this ID again.");
            ui.add(egui::Label::new(egui::RichText::new(none).color(theme::MUTED)).wrap());
            return;
        }
        let which = format!(
            "It is one of these {}{length}. Play them and pick the one it is.",
            format::count(candidates.len(), "cached sound", "cached sounds")
        );
        ui.add(egui::Label::new(which).wrap());
        egui::ScrollArea::vertical().id_salt(("candidates", found.id)).max_height(230.0).show(ui, |ui| {
            for sound in candidates {
                ui.horizontal(|ui| {
                    let label = if playing == Some(sound.id) { "Pause" } else { "Play" };
                    if ui.button(label).clicked() {
                        actions.push(Action::PlaySound(sound.id));
                    }
                    if ui.button("This is it").on_hover_text("Tie the asset ID to this entry").clicked() {
                        actions.push(Action::Pick(found.id, sound.id));
                    }
                    let seconds = sound.seconds.map_or("length unknown".to_string(), |s| format!("{s:.2} s"));
                    ui.label(egui::RichText::new(seconds).monospace());
                    ui.label(egui::RichText::new(human_size(sound.size)).color(theme::MUTED));
                    ui.label(egui::RichText::new(format!("last used {}", format_time_ms(sound.atime))).color(theme::MUTED).small());
                    if ui.small_button("Show").on_hover_text("Open this entry under Browse").clicked() {
                        actions.push(Action::ShowEntry(sound.id));
                    }
                });
            }
        });
    }

    fn guide_page(&mut self, ui: &mut egui::Ui, actions: &mut Vec<Action>) {
        self.page_header(ui, "Guide", "Using a config, from getting one to seeing it in game.");
        let steps = [
            ("Get a config", "A config is a file of replacement rules: swap this asset for that file, remove that one. Packs lists ready-made ones. Any config made for Fleasion works too; it is the same format."),
            ("Add it", "Press Apply on a pack, or Add configs on the Configs tab for a file of your own. rbxcache looks up the asset IDs the config names and shows what it would change. Nothing is written until you press Apply in that window."),
            ("Play", "Leave rbxcache open, or close it and let it sit in the tray. Roblox only saves an asset when a game first needs it, so most of a pack has nothing to replace until you have joined the game. Each asset is replaced as it arrives."),
            ("Rejoin", "Roblox keeps what it has already loaded in memory, so a replacement shows the next time the asset is loaded. The first time with a pack: join the game, leave, and join again. After that the replacements are already in place when you start."),
            ("Tune it", "Every config has a switch, and under Rules so does each thing it changes. When two configs change the same thing, a Conflicts box appears at the top of Configs and you choose which one has it."),
            ("Undo it", "Switch a config or a rule off and what it replaced is put back from the copy rbxcache kept. Remove does that for the whole config. File > Clear cache makes Roblox download everything fresh."),
        ];
        for (n, (title, text)) in steps.iter().enumerate() {
            card(ui, self.entrance(ui.ctx(), n + 1), |ui| {
                ui.horizontal(|ui| {
                    theme::pill(ui, &(n + 1).to_string(), theme::LIME);
                    ui.label(egui::RichText::new(*title).size(16.0).strong());
                });
                ui.add(egui::Label::new(*text).wrap());
                if n == 0 && ui.button("Open Packs").clicked() {
                    actions.push(Action::ShowTab(Tab::Packs));
                }
                if n == 1 && ui.add_enabled(self.busy().is_none(), egui::Button::new("Add configs...")).clicked() {
                    actions.push(Action::AddConfigs);
                }
            });
        }
        card(ui, self.entrance(ui.ctx(), steps.len() + 1), |ui| {
            ui.label(egui::RichText::new("When something does not change").size(16.0).strong());
            for (what, why) in [
                ("Sounds", "Roblox only gives most sounds to signed-in players, so the rules that name them are locked until you sign in on the Account tab (an alternate account is fine). Once signed in, rbxcache asks again as that account, and then as each of the sound's creator's games, so the game does not have to have been joined first. What is still refused is listed on the IDs tab as the cached sounds of the right length, to play and pick from."),
                ("It still looks the same", "The asset was loaded before it was replaced. Rejoin or restart Roblox."),
                ("It changed, then changed back", "Roblox downloaded the asset again. With rbxcache open and Live > Watch cache live on, it is replaced again as it arrives."),
                ("Nothing matched", "Roblox has not downloaded those assets on this computer yet. Play the game once with rbxcache running."),
            ] {
                ui.add_space(4.0);
                ui.label(egui::RichText::new(what).color(theme::LIME));
                ui.add(egui::Label::new(why).wrap());
            }
        });
    }

    fn packs_page(&mut self, ui: &mut egui::Ui, actions: &mut Vec<Action>) {
        self.page_header(
            ui,
            "Packs",
            "Known packs. Applying one downloads its config and loads it like any other, after showing what it would change.",
        );
        let fetching = self.catalog_job.is_some();
        ui.horizontal(|ui| {
            if ui.add_enabled(!fetching, egui::Button::new("Refresh")).on_hover_text("Fetch the list again").clicked() {
                actions.push(Action::RefreshPacks);
            }
            if fetching {
                ui.spinner();
                ui.label(egui::RichText::new("Fetching the pack list").color(theme::MUTED));
            }
        });
        ui.add_space(8.0);
        // Until the list has arrived once, placeholders stand where its cards will be.
        if fetching && !self.catalog_fetched {
            let now = ui.input(|i| i.time);
            for _ in 0..3 {
                skeleton_card(ui, now, self.settings.animations);
            }
            ui.ctx().request_repaint();
            return;
        }
        if let Some(note) = &self.catalog_note {
            ui.add(egui::Label::new(egui::RichText::new(note).color(theme::WARN).small()).wrap());
            ui.add_space(4.0);
        }
        let idle = self.busy().is_none();
        for (i, pack) in self.catalog.packs.iter().enumerate() {
            let loaded = pack.path().is_ok_and(|path| self.configs.iter().any(|c| c.path == path));
            card(ui, self.entrance(ui.ctx(), i + 1), |ui| {
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new(&pack.name).size(17.0).strong());
                    if !pack.game.is_empty() {
                        theme::pill(ui, &pack.game, theme::LIME);
                    }
                    if !pack.author.is_empty() {
                        ui.label(egui::RichText::new(format!("by {}", pack.author)).color(theme::MUTED));
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let button = if loaded { egui::Button::new("Update and re-apply") } else { primary_button("Apply") };
                        if ui.add_enabled(idle, button).on_hover_text("Download the pack's config and check it against the cache").clicked() {
                            actions.push(Action::ApplyPack(i));
                        }
                        if loaded {
                            theme::pill(ui, "Loaded", theme::OK);
                        }
                    });
                });
                if !pack.description.is_empty() {
                    ui.add(egui::Label::new(&pack.description).wrap());
                }
            });
        }
        if self.catalog.packs.is_empty() {
            card(ui, self.entrance(ui.ctx(), 1), |ui| {
                ui.label(egui::RichText::new("The list has no packs right now.").color(theme::MUTED));
            });
        }
        let note = "Images and fonts are matched by looking each asset ID up with Roblox, as a guest. Roblox refuses that for most sounds and \
                    for some images. Signed in on the Account tab, rbxcache asks again as that account; for what is still refused, pick the sound \
                    by ear on the IDs tab or tag the entries yourself. The Guide tab has more.";
        ui.add_space(4.0);
        ui.add(egui::Label::new(egui::RichText::new(note).color(theme::MUTED).small()).wrap());
    }

    fn info_page(&mut self, ui: &mut egui::Ui, actions: &mut Vec<Action>) {
        let t = self.entrance(ui.ctx(), 0);
        ui.scope(|ui| {
            ui.set_opacity(t);
            ui.add_space((1.0 - t) * 12.0);
            ui.label(egui::RichText::new("pirxcy").font(egui::FontId::new(46.0, theme::wordmark_font())).color(theme::SPLASH_INK));
            ui.label(
                egui::RichText::new(format!("rbxcache {}  -  the Roblox asset cache, edited where it sits", update::CURRENT)).color(theme::MUTED),
            );
        });
        ui.add_space(12.0);

        if let Some(release) = &self.update {
            card(ui, self.entrance(ui.ctx(), 1), |ui| {
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new(format!("Version {} is out", release.version)).size(17.0).strong().color(theme::LIME));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.add(primary_button("Open the release page")).clicked() {
                            actions.push(Action::OpenUpdate);
                        }
                    });
                });
                if release.news.is_empty() {
                    ui.label(egui::RichText::new("The list of changes could not be fetched; the release page has it.").color(theme::MUTED));
                } else {
                    ui.label(egui::RichText::new(format!("Everything new since {}:", update::CURRENT)).color(theme::MUTED));
                    versions_list(ui, &release.news);
                }
            });
        }

        let facts = [
            ("No frames lost", "Nothing runs inside Roblox: no injected code, no overlay, no hooks. The game renders exactly as it does on its own."),
            ("No lag added", "Nothing sits between Roblox and its servers. There is no proxy to slow a connection down or drop it."),
            ("Files only", "A replacement is a file swapped in the cache on disk. Roblox loads it the way it loads any cached asset."),
        ];
        let t = self.entrance(ui.ctx(), 2);
        ui.scope(|ui| {
            ui.set_opacity(t);
            ui.add_space((1.0 - t) * 16.0);
            ui.columns(3, |columns| {
                for (column, (title, text)) in columns.iter_mut().zip(facts) {
                    egui::Frame::new()
                        .fill(theme::CARD)
                        .stroke(Stroke::new(1.0_f32, theme::BORDER))
                        .corner_radius(CornerRadius::same(12))
                        .inner_margin(Margin::same(16))
                        .show(column, |ui| {
                            ui.set_min_height(118.0);
                            // Columns justify their content, which spreads the words of a wrapped line apart.
                            ui.with_layout(egui::Layout::top_down(egui::Align::Min), |ui| {
                                ui.label(egui::RichText::new(title).size(17.0).strong().color(theme::LIME));
                                ui.add(egui::Label::new(egui::RichText::new(text).color(theme::TEXT)).wrap());
                            });
                        });
                }
            });
        });
        ui.add_space(4.0);

        card(ui, self.entrance(ui.ctx(), 3), |ui| {
            ui.label(egui::RichText::new("Lightweight by design").size(17.0).strong());
            let text = "rbxcache changes what Roblox has already saved to disk, and nothing else. It is not a proxy and it installs no \
                        certificate. Adding an account is optional, and it is a cookie you paste yourself; rbxcache never reads it from your \
                        browser or Roblox's own files on its own, and never asks for a password. While it sits in the tray it looks at the \
                        cache index once a second and puts your configs on what is new. A texture is written at the size of the one it replaces.";
            ui.add(egui::Label::new(egui::RichText::new(text).color(theme::TEXT)).wrap());
            ui.add_space(4.0);
            for (n, step) in [
                "Roblox downloads an asset and saves it in its cache.",
                "rbxcache recognises the entry and swaps its content for yours.",
                "The next time Roblox loads that asset, it reads the replacement.",
            ]
            .iter()
            .enumerate()
            {
                ui.horizontal(|ui| {
                    theme::pill(ui, &(n + 1).to_string(), theme::ACCENT);
                    ui.label(*step);
                });
            }
            let caveat = "An asset Roblox already has in memory keeps its old look until it is loaded again: rejoin, or restart the client.";
            ui.add(egui::Label::new(egui::RichText::new(caveat).color(theme::MUTED).small()).wrap());
        });

        card(ui, self.entrance(ui.ctx(), 4), |ui| {
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new("What is new").size(17.0).strong());
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.add_enabled(self.update_check.is_none(), egui::Button::new("Check for updates")).clicked() {
                        actions.push(Action::CheckUpdate);
                    }
                });
            });
            match self.changelog.split_first() {
                Some((current, earlier)) => {
                    versions_list(ui, std::slice::from_ref(current));
                    if !earlier.is_empty() {
                        egui::CollapsingHeader::new(egui::RichText::new("Earlier versions").color(theme::MUTED))
                            .show(ui, |ui| versions_list(ui, earlier));
                    }
                }
                None => drop(ui.label(egui::RichText::new("No changelog was built into this copy.").color(theme::MUTED))),
            }
        });

        card(ui, self.entrance(ui.ctx(), 5), |ui| {
            ui.label(egui::RichText::new("Shortcuts").size(17.0).strong());
            egui::Grid::new("shortcuts").num_columns(2).spacing([18.0, 6.0]).show(ui, |ui| {
                for (keys, what) in [
                    ("Up / Down", "Move through the list"),
                    ("Page Up / Page Down", "Move twenty rows"),
                    ("Shift + Up / Down", "Extend the selection"),
                    ("Ctrl / Shift + click", "Select several"),
                    ("Double-click", "Open in the default program"),
                    ("Right-click", "Entry menu"),
                    ("Space", "Play or pause a sound or animation"),
                    ("Ctrl+F", "Search"),
                    ("Ctrl+E", "Export"),
                    ("Ctrl+O", "Add configs"),
                    ("Delete", "Delete the selection"),
                    ("F5", "Reload the cache"),
                    ("F1", "This page"),
                ] {
                    ui.label(egui::RichText::new(keys).monospace().color(theme::ACCENT));
                    ui.label(what);
                    ui.end_row();
                }
            });
        });
    }

    /// Shown on the Live tab until Roblox writes something.
    fn live_placeholder(&self, ui: &mut egui::Ui) {
        ui.add_space(ui.available_height() * 0.36);
        ui.vertical_centered(|ui| {
            if self.watch.is_none() {
                ui.label(egui::RichText::new("Live watching is off").size(20.0));
                ui.label(egui::RichText::new("Turn on Live > Watch cache live to fill this list.").color(theme::MUTED));
                return;
            }
            // A dot that pulses while waiting.
            let (dot, _) = ui.allocate_exact_size(Vec2::splat(16.0), egui::Sense::hover());
            let pulse = if self.settings.animations { (ui.input(|i| i.time) * 2.2).sin() as f32 * 0.5 + 0.5 } else { 1.0 };
            ui.painter().circle_filled(dot.center(), 4.0 + 2.0 * pulse, alpha(theme::OK, 0.45 + 0.55 * pulse));
            ui.add_space(4.0);
            ui.label(egui::RichText::new("Waiting for Roblox").size(20.0));
            let state = if self.roblox_running {
                "Assets appear here as Roblox downloads them."
            } else {
                "Roblox is not running. Assets appear here as it downloads them."
            };
            ui.label(egui::RichText::new(state).color(theme::MUTED));
            if self.settings.animations {
                ui.ctx().request_repaint();
            }
        });
    }

    /// What the app is busy with, if anything: a short title, a sentence of
    /// what that work is doing, and, when it can be measured, how far along it is.
    fn busy(&self) -> Option<(String, String, Option<f32>)> {
        let fraction = |(done, total): (usize, usize)| (total > 0).then(|| done as f32 / total as f32);
        if self.clearing.is_some() {
            Some((
                "Clearing the cache".into(),
                "Deleting every cached entry on this computer so Roblox can download them fresh. Replacements made here go with them.".into(),
                None,
            ))
        } else if self.loading.is_some() {
            let (done, total) = self.progress;
            let caption = if total > 0 { format!("Reading the Roblox cache  {done} / {total}") } else { "Opening the cache index".into() };
            let detail = if total > 0 {
                "Classifying each file in rbx-storage so Browse and Live have something to show, without changing any of them.".into()
            } else {
                "Opening rbx-storage.db and preparing to walk the files Roblox has kept on this computer.".into()
            };
            Some((caption, detail, fraction(self.progress)))
        } else if self.lookup_job.is_some() {
            let (done, total) = self.lookup_progress;
            let caption = if total > 0 { format!("Fetching asset IDs  {done} / {total}") } else { "Preparing to fetch asset IDs".into() };
            let detail = if resolve::signed_in() {
                "Asking Roblox as your signed-in account, matching what comes back to your cache, and listing sounds that still need a pick by ear."
                    .into()
            } else {
                "Asking Roblox for each ID and matching what comes back to your cache. Sign in on the Account tab for sounds and other locked assets."
                    .into()
            };
            Some((caption, detail, fraction(self.lookup_progress)))
        } else if let Some((_, name)) = &self.pack_job {
            Some((
                format!("Downloading {name}"),
                "Fetching the pack's config so it can be checked against your cache before anything is written.".into(),
                None,
            ))
        } else if self.apply_job.is_some() {
            Some((
                "Applying configs".into(),
                "Rewriting the cached entries your enabled configs name, and putting back anything no longer wanted from the copies rbxcache kept."
                    .into(),
                None,
            ))
        } else if self.config_job.is_some() {
            let (done, total) = self.config_progress;
            let caption = if total > 0 { format!("Checking configs  {done} / {total}") } else { "Checking configs".into() };
            Some((
                caption,
                "Looking up every asset ID the configs name so rbxcache knows which cached entries they hit, before anything is written.".into(),
                fraction(self.config_progress),
            ))
        } else if self.account_job.is_some() {
            Some((
                "Checking the account".into(),
                "Asking Roblox who this cookie belongs to. Nothing is written until that answer comes back.".into(),
                None,
            ))
        } else {
            None
        }
    }

    /// The full-window splash shown while the app is busy: the "pirxcy" wordmark
    /// bounces in, holds, and dissolves once the work is done.
    fn show_splash(&mut self, ctx: &egui::Context) {
        /// Seconds the wordmark takes to bounce in, stays at least, and takes to dissolve.
        const BOUNCE: f32 = 0.9;
        const HOLD: f32 = 0.28;
        const DISSOLVE: f32 = 0.62;
        /// Work shorter than this passes without a splash, except the first load.
        const PATIENCE: f64 = 0.15;
        /// A switch on the Configs page gets this long before its work is given a splash.
        const QUIET_PATIENCE: f64 = 2.5;
        use theme::{SPLASH_BAR, SPLASH_INK as INK, SPLASH_MUTED as MUTED};

        let now = ctx.input(|i| i.time);
        let busy = self.busy();
        match (&mut self.splash, &busy) {
            (None, Some(_)) => {
                let since = *self.busy_since.get_or_insert(now);
                let patience = if self.quiet { QUIET_PATIENCE } else { PATIENCE };
                if now - since >= patience || (self.entries.is_empty() && !self.quiet) {
                    self.splash = Some(Splash { shown: now, leaving: None, caption: String::new(), detail: String::new(), fraction: None });
                } else {
                    ctx.request_repaint();
                }
            }
            // Let the entrance finish before leaving, however quick the work was.
            (Some(splash), None) if splash.leaving.is_none() => {
                let earliest = splash.shown + if self.settings.animations { (BOUNCE + HOLD) as f64 } else { 0.0 };
                splash.leaving = Some(now.max(earliest));
            }
            // More work arrived while dissolving: stay.
            (Some(splash), Some(_)) => splash.leaving = None,
            _ => {}
        }
        if busy.is_none() {
            (self.busy_since, self.quiet) = (None, false);
        }
        let Some(splash) = &mut self.splash else { return };
        if let Some((caption, detail, fraction)) = busy {
            (splash.caption, splash.detail, splash.fraction) = (caption, detail, fraction);
        }
        let leaving = splash.leaving.map_or(0.0, |at| ((now - at) as f32 / if self.settings.animations { DISSOLVE } else { 0.001 }).clamp(0.0, 1.0));
        if leaving >= 1.0 {
            self.splash = None;
            return;
        }

        // The entrance: up from below, past its place, a small settle back.
        let entered = if self.settings.animations { ((now - splash.shown) as f32 / BOUNCE).clamp(0.0, 1.0) } else { 1.0 };
        let between = |from: f32, to: f32, start: f32, end: f32| from + (to - from) * theme::ease_out((entered - start) / (end - start));
        let (rise, scale) = match entered {
            t if t < 0.55 => (between(22.0, -10.0, 0.0, 0.55), between(0.94, 1.02, 0.0, 0.55)),
            t if t < 0.78 => (between(-10.0, 4.0, 0.55, 0.78), between(1.02, 0.995, 0.55, 0.78)),
            _ => (between(4.0, 0.0, 0.78, 1.0), between(0.995, 1.0, 0.78, 1.0)),
        };
        let opacity = (entered / 0.55).min(1.0) * (1.0 - leaving);
        let scale = scale * (1.0 + 0.04 * leaving);

        let (caption, detail, fraction, motion) = (splash.caption.clone(), splash.detail.clone(), splash.fraction, self.settings.animations);
        let shown = ctx.animate_value_with_time(egui::Id::new("splash_progress"), fraction.unwrap_or(0.0), 0.25);
        let screen = ctx.screen_rect();
        egui::Area::new(egui::Id::new("splash")).order(egui::Order::Foreground).fixed_pos(screen.min).show(ctx, |ui| {
            // Takes the clicks while there is work going on; once dissolving, the app is usable again.
            if leaving == 0.0 {
                ui.allocate_rect(screen, egui::Sense::click_and_drag());
            }
            let painter = ui.painter();
            theme::splash_gradient(painter, screen, 1.0 - leaving);

            let size = (screen.width() * 0.12).clamp(52.0, 120.0) * scale;
            let mut job = egui::text::LayoutJob::default();
            let format = egui::TextFormat {
                font_id: egui::FontId::new(size, theme::wordmark_font()),
                // The wordmark stays white on every splash, over the purple gradient.
                color: Color32::WHITE,
                extra_letter_spacing: -0.07 * size,
                ..Default::default()
            };
            job.append("pirxcy", 0.0, format);
            let galley = ui.fonts(|f| f.layout_job(job));
            let at = screen.center() - galley.size() / 2.0 + Vec2::new(0.0, rise - 28.0);
            // Dissolving, the word spreads into soft copies of itself, which reads as a blur.
            if leaving > 0.0 {
                let spread = 10.0 * leaving;
                for step in 0..8 {
                    let angle = step as f32 * std::f32::consts::TAU / 8.0;
                    let offset = Vec2::new(angle.cos(), angle.sin()) * spread;
                    painter.galley_with_override_text_color(at + offset, galley.clone(), alpha(INK, opacity * 0.12));
                }
            }
            painter.galley_with_override_text_color(at, galley.clone(), alpha(INK, opacity * (1.0 - leaving * 0.6)));

            // Below the word: a short title, a sentence of what the work is doing, and a purple line for how far along it is.
            let below = at.y + galley.size().y + 16.0;
            let title_font = egui::FontId::proportional(14.0);
            let detail_font = egui::FontId::proportional(12.0);
            painter.text(egui::pos2(screen.center().x, below), egui::Align2::CENTER_TOP, &caption, title_font, alpha(Color32::WHITE, opacity * 0.92));
            let detail_width = (screen.width() * 0.55).clamp(260.0, 460.0);
            let mut detail_job = egui::text::LayoutJob::default();
            detail_job.append(&detail, 0.0, egui::TextFormat { font_id: detail_font, color: alpha(MUTED, opacity), ..Default::default() });
            detail_job.wrap.max_width = detail_width;
            let detail_galley = ui.fonts(|f| f.layout_job(detail_job));
            let detail_at = egui::pos2(screen.center().x - detail_galley.size().x / 2.0, below + 22.0);
            painter.galley(detail_at, detail_galley.clone(), alpha(MUTED, opacity));
            let track = Rect::from_center_size(egui::pos2(screen.center().x, detail_at.y + detail_galley.size().y + 18.0), Vec2::new(220.0, 3.0));
            painter.rect_filled(track, CornerRadius::same(2), alpha(SPLASH_BAR, 0.18 * opacity));
            let filled = match fraction {
                Some(_) => Rect::from_min_size(track.min, Vec2::new(track.width() * shown.clamp(0.0, 1.0), track.height())),
                // Work that cannot be measured gets a short bar sweeping across.
                None => {
                    let phase = if motion { (now * 0.9).fract() as f32 } else { 0.4 };
                    let left = track.left() + (track.width() + 60.0) * phase - 60.0;
                    Rect::from_min_size(egui::pos2(left, track.top()), Vec2::new(60.0, track.height())).intersect(track)
                }
            };
            painter.rect_filled(filled, CornerRadius::same(2), alpha(SPLASH_BAR, opacity));
        });
        ctx.request_repaint();
    }

    /// A centred dialog that fades and slides in over a dimmed window. Returns
    /// true when the user dismissed it (Escape or a click outside).
    fn modal(&self, ctx: &egui::Context, name: &str, open: bool, add: impl FnOnce(&mut egui::Ui)) -> bool {
        let time = if self.settings.animations { 0.22 } else { 0.0 };
        let t = ctx.animate_bool_with_time_and_easing(egui::Id::new(name).with("t"), open, time, egui::emath::easing::cubic_out);
        if t <= 0.0 {
            return false;
        }
        let frame = egui::Frame::new()
            .fill(alpha(theme::CARD, t))
            .stroke(Stroke::new(1.0_f32, alpha(theme::BORDER, t)))
            .corner_radius(CornerRadius::same(14))
            .inner_margin(Margin::same(22))
            .shadow(egui::Shadow { offset: [0, 16], blur: 48, spread: 0, color: Color32::from_black_alpha((150.0 * t) as u8) });
        let response =
            egui::Modal::new(egui::Id::new(name)).backdrop_color(Color32::from_black_alpha((140.0 * t) as u8)).frame(frame).show(ctx, |ui| {
                ui.set_width(400.0);
                ui.set_opacity(t);
                ui.add_space((1.0 - t) * 26.0);
                add(ui);
            });
        open && response.should_close()
    }

    /// The warning that has to sit for [`account::SUBMIT_HOLD`] before the cookie is sent.
    fn account_warn_dialog(&mut self, ctx: &egui::Context, actions: &mut Vec<Action>) {
        let Some(opened) = self.account_warn_from else { return };
        let left = (account::SUBMIT_HOLD.as_secs_f64() - (ctx.input(|i| i.time) - opened)).max(0.0);
        if left > 0.0 {
            ctx.request_repaint_after(std::time::Duration::from_millis(100));
        }
        let mut close = false;
        let mut send = false;
        let dismissed = self.modal(ctx, "account-warn", true, |ui| {
            ui.label(egui::RichText::new("Before this cookie is sent").size(18.0));
            ui.add_space(4.0);
            ui.add(egui::Label::new(egui::RichText::new(account::WARNING).strong().color(theme::DANGER)).wrap());
            ui.add_space(12.0);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let ready = left <= 0.0;
                let label = if ready { "Add this account".to_string() } else { format!("Add in {}", left.ceil() as u32) };
                if ui.add_enabled(ready, primary_button(&label)).clicked() {
                    send = true;
                    close = true;
                }
                if ui.button("Cancel").clicked() {
                    close = true;
                }
            });
        });
        if send {
            actions.push(Action::AddAccount);
        }
        if close || dismissed {
            self.account_warn_from = None;
        }
    }

    fn clear_dialog(&mut self, ctx: &egui::Context, actions: &mut Vec<Action>) {
        let mut close = false;
        let dismissed = self.modal(ctx, "clear", self.confirm_clear, |ui| {
            ui.label(egui::RichText::new("Clear the cache").size(18.0));
            ui.add_space(4.0);
            ui.label(format!("Delete all {}?", format::count(self.entries.len(), "cached entry", "cached entries")));
            ui.label(
                egui::RichText::new("Roblox downloads everything again as it needs it. Replacements made here are lost with the rest.")
                    .color(theme::MUTED),
            );
            if self.roblox_running {
                ui.add_space(4.0);
                ui.colored_label(theme::WARN, "Roblox is running and will be closed first.");
            }
            ui.add_space(12.0);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let label = if self.roblox_running { "Close Roblox and clear" } else { "Clear" };
                if ui.add(egui::Button::new(egui::RichText::new(label).color(Color32::WHITE)).fill(alpha(theme::DANGER, 0.75))).clicked()
                    && self.confirm_clear
                {
                    actions.push(Action::Clear);
                    close = true;
                }
                if ui.button("Cancel").clicked() {
                    close = true;
                }
            });
        });
        if close || dismissed {
            self.confirm_clear = false;
        }
    }

    fn delete_dialog(&mut self, ctx: &egui::Context, actions: &mut Vec<Action>) {
        let mut close = false;
        let dismissed = self.modal(ctx, "delete", self.confirm_delete, |ui| {
            ui.label(egui::RichText::new("Delete entries").size(18.0));
            ui.add_space(4.0);
            ui.label(&self.delete_prompt);
            ui.label(egui::RichText::new("Roblox downloads deleted assets again the next time it needs them.").color(theme::MUTED));
            if self.roblox_running {
                ui.add_space(4.0);
                ui.colored_label(theme::WARN, "Roblox is running and may still be using these.");
            }
            ui.add_space(12.0);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.add(egui::Button::new(egui::RichText::new("Delete").color(Color32::WHITE)).fill(alpha(theme::DANGER, 0.75))).clicked()
                    && self.confirm_delete
                {
                    actions.push(Action::Delete);
                    close = true;
                }
                if ui.button("Cancel").clicked() {
                    close = true;
                }
            });
        });
        if close || dismissed {
            self.confirm_delete = false;
        }
    }

    fn configs_dialog(&mut self, ctx: &egui::Context, actions: &mut Vec<Action>) {
        let Some(pending) = &self.pending_configs else { return };
        let (mut close, mut accepted) = (false, false);
        let dismissed = self.modal(ctx, "config", self.confirm_configs, |ui| {
            ui.label(egui::RichText::new(format!("Apply {}", format::count(pending.configs.len(), "config", "configs"))).size(18.0));
            ui.add_space(4.0);
            egui::ScrollArea::vertical().id_salt("config paths").max_height(110.0).show(ui, |ui| {
                for config in &pending.configs {
                    ui.label(egui::RichText::new(config.path.display().to_string()).color(theme::MUTED).small());
                }
            });
            if !pending.lookup.is_empty() {
                ui.label(egui::RichText::new(&pending.lookup).color(theme::MUTED));
            }
            ui.label(&pending.summary);
            const SHOWN: usize = 6;
            for note in pending.notes.iter().take(SHOWN) {
                ui.add(egui::Label::new(egui::RichText::new(note).color(theme::WARN).small()).wrap());
            }
            if pending.notes.len() > SHOWN {
                ui.label(egui::RichText::new(format!("and {} more", pending.notes.len() - SHOWN)).color(theme::MUTED).small());
            }
            if self.roblox_running {
                ui.add_space(4.0);
                ui.colored_label(theme::WARN, "Roblox is running and may overwrite or ignore the changes.");
            }
            ui.add_space(12.0);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.add(egui::Button::new(egui::RichText::new("Apply").color(Color32::WHITE)).fill(alpha(theme::ACCENT, 0.75))).clicked()
                    && self.confirm_configs
                {
                    actions.push(Action::ConfirmConfigs);
                    (close, accepted) = (true, true);
                }
                if ui.button("Cancel").clicked() {
                    close = true;
                }
            });
        });
        if close || dismissed {
            self.confirm_configs = false;
            if !accepted {
                actions.push(Action::CancelConfigs);
            }
        }
    }

    /// Shown once after an update: what changed since the version last used.
    fn news_dialog(&mut self, ctx: &egui::Context) {
        if !self.show_news {
            return;
        }
        let seen = &self.settings.seen_version;
        // A first install is shown this version only; an update, everything in between.
        let news: Vec<Version> = match seen.is_empty() {
            true => self.changelog.first().cloned().into_iter().collect(),
            false => changelog::between(self.changelog.clone(), seen, update::CURRENT),
        };
        let mut close = news.is_empty();
        // It waits for the loading screen to clear.
        let open = !news.is_empty() && self.busy().is_none() && self.splash.is_none();
        let dismissed = self.modal(ctx, "news", open, |ui| {
            ui.label(egui::RichText::new("What is new").font(egui::FontId::new(24.0, theme::wordmark_font())).color(theme::SPLASH_INK));
            ui.label(egui::RichText::new(format!("rbxcache {}", update::CURRENT)).color(theme::MUTED));
            egui::ScrollArea::vertical().id_salt("news").max_height(380.0).show(ui, |ui| versions_list(ui, &news));
            ui.add_space(12.0);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.add(primary_button("Got it")).clicked() {
                    close = true;
                }
            });
        });
        if close || dismissed {
            self.show_news = false;
            self.settings.seen_version = update::CURRENT.to_string();
            self.settings.save();
        }
    }

    /// The last message, springing up from the bottom edge and fading away after a few seconds.
    fn show_toast(&mut self, ctx: &egui::Context) {
        const IN: f32 = 0.42;
        const HOLD: f32 = 3.8;
        const OUT: f32 = 0.35;
        let Some(toast) = &self.toast else { return };
        let age = (ctx.input(|i| i.time) - toast.born) as f32;
        if age > IN + HOLD + OUT {
            self.toast = None;
            return;
        }
        // `travel` is how far from rest the toast still is; `speed` drives the trail.
        let (opacity, travel, speed) = if !self.settings.animations {
            (1.0, 0.0, 0.0)
        } else if age < IN {
            let k = age / IN;
            ((k * 2.0).min(1.0), 1.0 - theme::ease_out_back(k), 1.0 - k)
        } else if age < IN + HOLD {
            (1.0, 0.0, 0.0)
        } else {
            let k = (age - IN - HOLD) / OUT;
            (1.0 - k, k * k, k)
        };
        let accent = if toast.error { theme::DANGER } else { theme::OK };
        let radius = CornerRadius::same(12);
        egui::Area::new(egui::Id::new("toast"))
            .order(egui::Order::Foreground)
            .anchor(egui::Align2::CENTER_BOTTOM, [0.0, -46.0 + travel * 60.0])
            .interactable(false)
            .show(ctx, |ui| {
                ui.set_opacity(opacity);
                let trail_slot = ui.painter().add(Shape::Noop);
                let rect = egui::Frame::new()
                    .fill(theme::CARD)
                    .stroke(Stroke::new(1.0_f32, alpha(accent, 0.55)))
                    .corner_radius(radius)
                    .inner_margin(Margin::symmetric(16, 10))
                    .shadow(egui::Shadow { offset: [0, 8], blur: 28, spread: 0, color: Color32::from_black_alpha(130) })
                    .show(ui, |ui| {
                        ui.set_max_width(680.0);
                        ui.horizontal(|ui| {
                            let (dot, _) = ui.allocate_exact_size(Vec2::splat(8.0), egui::Sense::hover());
                            ui.painter().circle_filled(dot.center(), 4.0, accent);
                            ui.add(egui::Label::new(&toast.text).wrap());
                        });
                    })
                    .response
                    .rect;
                if speed > 0.02 {
                    ui.painter().set(trail_slot, theme::motion_trail(rect, Vec2::new(0.0, 46.0 * speed), theme::CARD, radius, opacity * speed));
                }
            });
        ctx.request_repaint();
    }

    fn shortcuts(&mut self, ctx: &egui::Context, actions: &mut Vec<Action>) {
        if self.confirm_delete || self.confirm_clear || self.confirm_configs || self.show_news || self.account_warn_from.is_some() {
            return;
        }
        let typing = ctx.wants_keyboard_input();
        ctx.input_mut(|i| {
            use egui::{Key, KeyboardShortcut, Modifiers};
            if i.consume_shortcut(&KeyboardShortcut::new(Modifiers::COMMAND, Key::F)) {
                self.focus_search = true;
            }
            if i.consume_shortcut(&KeyboardShortcut::new(Modifiers::COMMAND, Key::O)) {
                actions.push(Action::AddConfigs);
            }
            if i.consume_shortcut(&KeyboardShortcut::new(Modifiers::COMMAND, Key::E)) {
                actions.push(if self.selected.len() > 1 { Action::ExportMany } else { Action::Export });
            }
            if i.key_pressed(Key::F5) {
                actions.push(Action::Reload);
            }
            if i.key_pressed(Key::F1) {
                actions.push(Action::ShowTab(Tab::Info));
            }
            // The keys below act on the entry list, which the pages do not show.
            if typing || self.busy().is_some() || !self.tab.lists() {
                return;
            }
            if i.key_pressed(Key::Space) {
                let animation = self.preview.as_mut().and_then(|p| Some((p.model.as_ref()?.animation.as_ref()?.length, p.anim.as_mut()?)));
                match animation {
                    Some((length, view)) => view.toggle(length),
                    None => actions.push(Action::PlayAudio),
                }
            }
            if i.key_pressed(Key::Delete) {
                actions.push(Action::AskDelete);
            }
            let extend = i.modifiers.shift;
            if i.key_pressed(Key::ArrowDown) {
                self.step_focus(1, extend);
            }
            if i.key_pressed(Key::ArrowUp) {
                self.step_focus(-1, extend);
            }
            if i.key_pressed(Key::PageDown) {
                self.step_focus(20, extend);
            }
            if i.key_pressed(Key::PageUp) {
                self.step_focus(-20, extend);
            }
        });
    }
}

/// One of the sidebar's rows.
struct NavRow {
    label: &'static str,
    hint: &'static str,
    count: Option<usize>,
    selected: bool,
    /// The tab a click on it goes to.
    opens: Tab,
}

/// A rounded tab with a coloured dot and, for a list, how many it holds.
fn chip(ui: &mut egui::Ui, label: &str, count: Option<usize>, colour: Color32, selected: bool, t: f32) -> egui::Response {
    let ink = if selected { Color32::WHITE } else { theme::TEXT };
    let name = ui.painter().layout_no_wrap(label.to_string(), egui::FontId::proportional(13.5), alpha(ink, t));
    let number = count.map(|n| ui.painter().layout_no_wrap(n.to_string(), egui::FontId::proportional(11.5), alpha(theme::MUTED, t)));
    let width = 30.0 + name.size().x + number.as_ref().map_or(0.0, |n| n.size().x + 7.0) + 12.0;
    let (rect, response) = ui.allocate_exact_size(Vec2::new(width, 28.0), egui::Sense::click());
    let response = response.on_hover_cursor(egui::CursorIcon::PointingHand);
    let painter = ui.painter();
    let (fill, edge) = match (selected, response.hovered()) {
        (true, _) => (alpha(theme::ACCENT, 0.26), alpha(theme::ACCENT, 0.7)),
        (false, true) => (theme::RAISED, theme::BORDER),
        (false, false) => (theme::CARD, theme::BORDER),
    };
    painter.rect(rect, CornerRadius::same(14), alpha(fill, t), Stroke::new(1.0_f32, alpha(edge, t)), egui::StrokeKind::Inside);
    painter.circle_filled(egui::pos2(rect.left() + 15.0, rect.center().y), 3.5, alpha(colour, t));
    let mut x = rect.left() + 26.0;
    painter.galley(egui::pos2(x, rect.center().y - name.size().y / 2.0), name.clone(), ink);
    x += name.size().x + 7.0;
    if let Some(number) = number {
        painter.galley(egui::pos2(x, rect.center().y - number.size().y / 2.0), number, theme::MUTED);
    }
    response
}

/// What a rule does, in a few words.
fn rule_summary(rule: &config::Rule) -> String {
    let what = match &rule.action {
        config::Action::Remove => "removed".to_string(),
        config::Action::Local(path) => {
            format!("replaced with {}", path.file_name().map_or_else(|| path.display().to_string(), |n| n.to_string_lossy().into_owned()))
        }
        config::Action::Cdn(url) => format!("replaced with {}", url.rsplit('/').next().unwrap_or(url)),
        config::Action::With(target) => format!("replaced with {target}"),
        config::Action::Invalid(why) => format!("not usable: {why}"),
    };
    format!("{}, {what}", format::count(rule.targets.len(), "target", "targets"))
}

/// A rounded panel for one thing on a page. `t` is how far its entrance has come.
fn card(ui: &mut egui::Ui, t: f32, add: impl FnOnce(&mut egui::Ui)) {
    ui.scope(|ui| {
        ui.set_opacity(t);
        ui.add_space((1.0 - t) * 16.0);
        egui::Frame::new()
            .fill(theme::CARD)
            .stroke(Stroke::new(1.0_f32, theme::BORDER))
            .corner_radius(CornerRadius::same(12))
            .inner_margin(Margin::same(16))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                add(ui);
            });
    });
    ui.add_space(4.0);
}

/// A card-shaped placeholder with a band of light passing over it, shown while a list loads.
fn skeleton_card(ui: &mut egui::Ui, now: f64, motion: bool) {
    card(ui, 1.0, |ui| {
        let phase = if motion { (now * 0.9).fract() as f32 } else { 0.5 };
        for (width, height) in [(0.34, 18.0), (0.92, 11.0), (0.7, 11.0)] {
            let (bar, _) = ui.allocate_exact_size(Vec2::new(ui.available_width() * width, height), egui::Sense::hover());
            let painter = ui.painter().with_clip_rect(bar);
            painter.rect_filled(bar, CornerRadius::same(5), theme::RAISED);
            // The band crosses the whole card, not each bar on its own.
            let span = ui.max_rect();
            let x = span.left() - 140.0 + (span.width() + 280.0) * phase;
            for (offset, strength) in [(0.0, 0.05), (30.0, 0.09), (60.0, 0.05)] {
                let band = Rect::from_min_size(egui::pos2(x + offset, bar.top()), Vec2::new(40.0, height));
                painter.rect_filled(band, CornerRadius::ZERO, alpha(Color32::WHITE, strength));
            }
            ui.add_space(3.0);
        }
    });
}

/// The one button on a page that does its main thing.
fn primary_button(text: &str) -> egui::Button<'_> {
    egui::Button::new(egui::RichText::new(text).color(Color32::WHITE)).fill(alpha(theme::ACCENT, 0.75))
}

/// An on/off switch whose knob slides.
fn switch(ui: &mut egui::Ui, on: bool) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(Vec2::new(36.0, 20.0), egui::Sense::click());
    let t = ui.ctx().animate_bool_with_time(response.id, on, 0.16);
    let painter = ui.painter();
    painter.rect_filled(rect, CornerRadius::same(10), theme::RAISED.lerp_to_gamma(alpha(theme::ACCENT, 0.9), t));
    let x = rect.left() + 10.0 + (rect.width() - 20.0) * t;
    painter.circle_filled(egui::pos2(x, rect.center().y), 7.0, Color32::WHITE);
    response.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// Versions of the changelog with their changes as a list.
fn versions_list(ui: &mut egui::Ui, versions: &[Version]) {
    for version in versions {
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            theme::pill(ui, &version.version, theme::LIME);
            ui.label(egui::RichText::new(&version.date).color(theme::MUTED).small());
        });
        for change in &version.changes {
            ui.horizontal_top(|ui| {
                ui.label(egui::RichText::new("-").color(theme::MUTED));
                ui.add(egui::Label::new(change).wrap());
            });
        }
    }
}

/// Of the asset IDs `rules` name: how many there are, how many are identified,
/// and how many Roblox refused to hand out.
fn id_stats(rules: &[config::Rule], idmap: &IdMap) -> (usize, usize, usize) {
    let ids = config::asset_ids(rules);
    let looked_up: Vec<&Vec<config::Fingerprint>> = ids.iter().filter_map(|id| idmap.assets.get(&id.to_string())).collect();
    let refused = looked_up.iter().filter(|prints| prints.is_empty()).count();
    (ids.len(), looked_up.len() - refused, refused)
}

/// The entries a replacement went into, for the message about it.
fn replaced_names(written: &[(Entry, Vec<u8>)]) -> String {
    match written {
        [(entry, _)] => format!("{} ({})", &entry.id_hex[..12], human_size(entry.size)),
        _ => format!("the {} packs of the texture", written.len()),
    }
}

/// A menu entry with a right-aligned shortcut hint.
fn item(ui: &mut egui::Ui, actions: &mut Vec<Action>, enabled: bool, text: &str, shortcut: &str, action: Action) {
    let button = egui::Button::new(text).shortcut_text(egui::RichText::new(shortcut).color(theme::MUTED));
    if ui.add_enabled(enabled, button).clicked() {
        actions.push(action);
        ui.close_menu();
    }
}

/// The per-entry commands, shared by the Entry menu and the row context menu.
fn entry_items(ui: &mut egui::Ui, actions: &mut Vec<Action>, one: bool, any: bool, source: bool) {
    item(ui, actions, one, "Export...", "Ctrl+E", Action::Export);
    item(ui, actions, any, "Export selected to folder...", "", Action::ExportMany);
    item(ui, actions, one, "Open", "Double-click", Action::OpenExternally);
    ui.separator();
    item(ui, actions, one, "Replace...", "", Action::Replace);
    item(ui, actions, one, "Use as replacement source", "", Action::SetSource);
    item(ui, actions, one && source, "Replace with source", "", Action::ReplaceFromSource);
    item(ui, actions, one, "Copy ID", "", Action::CopyId);
    ui.separator();
    let delete = egui::Button::new(egui::RichText::new("Delete").color(theme::DANGER)).shortcut_text(egui::RichText::new("Del").color(theme::MUTED));
    if ui.add_enabled(any, delete).clicked() {
        actions.push(Action::AskDelete);
        ui.close_menu();
    }
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.handle_close(ctx);
        self.poll_load(ctx);
        self.poll_watch(ctx);
        self.poll_update(ctx);
        self.poll_catalog(ctx);
        self.poll_pack(ctx);
        self.poll_lookup(ctx);
        self.poll_add_account(ctx);
        self.poll_avatars(ctx);
        self.poll_config(ctx);
        self.poll_apply(ctx);
        self.poll_clear(ctx);
        self.poll_reapply(ctx);
        if self.recheck_configs && self.busy().is_none() {
            self.recheck_configs = false;
            self.run_action(Action::ReapplyConfigs, ctx);
        }
        if self.lookup_wanted && self.busy().is_none() {
            self.lookup_wanted = false;
            self.run_action(Action::RunLookup, ctx);
        }
        if self.sync_wanted && self.busy().is_none() && self.reapply_job.is_none() {
            self.start_sync(ctx);
        }
        self.start_reapply(ctx);
        // In the tray the cache is still followed, but there is nothing to draw.
        if self.tray.as_ref().is_some_and(Tray::hidden) {
            return;
        }
        let now = ctx.input(|i| i.time);
        self.fresh.retain(|_, at| ((now - *at) as f32) < FRESH_SECONDS);
        if !self.fresh.is_empty() {
            ctx.request_repaint();
        }
        self.roblox_running = self.roblox.load(Ordering::Relaxed);
        // A sound belongs to the entry it was started from.
        if let Some(audio) = self.audio.as_mut().filter(|a| self.tab.lists() && a.current.is_some() && a.current != self.focus) {
            audio.stop();
        }
        self.ensure_preview(ctx);
        let mut actions = Vec::new();
        self.shortcuts(ctx, &mut actions);

        let bar = egui::Frame::new().fill(theme::PANEL).inner_margin(Margin::symmetric(10, 6));
        egui::TopBottomPanel::top("menu").frame(bar).show(ctx, |ui| self.menu_bar(ui, &mut actions));
        egui::TopBottomPanel::bottom("status").frame(bar).show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new(&self.status).color(theme::MUTED).small());
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    self.account_chip(ui, &mut actions);
                    if self.watch.is_some() {
                        theme::pill(ui, "Live", theme::OK);
                    }
                    // Configs are only re-applied while the cache is watched.
                    match self.configs.iter().filter(|c| c.enabled && self.watch.is_some()).collect::<Vec<_>>().as_slice() {
                        [] => {}
                        [only] => theme::pill(ui, &format!("Applying {}", only.name), theme::ACCENT),
                        several => theme::pill(ui, &format!("Applying {} configs", several.len()), theme::ACCENT),
                    }
                    if !self.selected.is_empty() && self.tab.lists() {
                        let text = format!("{} selected  -  {}", self.selected.len(), human_size(self.selected_bytes()));
                        ui.label(egui::RichText::new(text).color(theme::ACCENT).small());
                    }
                });
            });
        });
        egui::SidePanel::left("nav")
            .resizable(false)
            .exact_width(NAV_WIDTH)
            .frame(egui::Frame::new().fill(theme::PANEL).inner_margin(Margin::symmetric(0, 8)))
            .show(ctx, |ui| self.sidebar(ui, &mut actions));
        if !self.tab.lists() {
            egui::CentralPanel::default()
                .frame(egui::Frame::new().fill(theme::BG).inner_margin(Margin::symmetric(18, 0)))
                .show(ctx, |ui| self.page(ui, &mut actions));
        } else {
            egui::TopBottomPanel::top("toolbar").frame(bar.inner_margin(Margin::symmetric(10, 8))).show(ctx, |ui| {
                self.browse_chips(ui, &mut actions);
                ui.add_space(2.0);
                self.toolbar(ui, &mut actions);
            });
            egui::SidePanel::right("details")
                .resizable(true)
                .default_width(500.0)
                .width_range(340.0..=820.0)
                .frame(egui::Frame::new().fill(theme::PANEL).inner_margin(Margin::same(14)))
                .show(ctx, |ui| self.details(ui, &mut actions));
            egui::CentralPanel::default().frame(egui::Frame::new().fill(theme::BG).inner_margin(Margin::symmetric(10, 6))).show(ctx, |ui| {
                // A newly opened tab's list fades in as it settles into place.
                let t = self.entrance(ui.ctx(), 0);
                ui.set_opacity(0.2 + 0.8 * t);
                ui.add_space((1.0 - t) * 10.0);
                if let Some(error) = &self.load_error {
                    ui.add_space(ui.available_height() * 0.36);
                    ui.vertical_centered(|ui| {
                        ui.label(egui::RichText::new("Could not load the cache").size(20.0));
                        ui.colored_label(theme::DANGER, error);
                        if ui.button("Try again").clicked() {
                            actions.push(Action::Reload);
                        }
                    });
                } else if self.loading.is_some() || self.clearing.is_some() {
                    // The splash covers the window while this goes on.
                } else if self.tab == Tab::Live && self.arrivals.is_empty() {
                    self.live_placeholder(ui);
                } else {
                    self.table(ui, &mut actions);
                }
            });
        }
        self.delete_dialog(ctx, &mut actions);
        self.clear_dialog(ctx, &mut actions);
        self.account_warn_dialog(ctx, &mut actions);
        self.configs_dialog(ctx, &mut actions);
        self.news_dialog(ctx);
        self.show_toast(ctx);
        self.show_splash(ctx);

        for action in actions {
            self.run_action(action, ctx);
            // Actions run after this frame was laid out, so draw another to show the result.
            ctx.request_repaint();
        }
    }
}

/// The answer of a worker thread once it has one. A worker that ended without
/// answering (it panicked) counts as having failed, so nothing waits on it for ever.
fn finished<T>(rx: &Receiver<Result<T, String>>) -> Option<Result<T, String>> {
    match rx.try_recv() {
        Ok(result) => Some(result),
        Err(TryRecvError::Empty) => None,
        Err(TryRecvError::Disconnected) => Some(Err("the work stopped unexpectedly".to_string())),
    }
}

/// Where "Open" puts the copies it hands to other programs.
fn open_dir() -> PathBuf {
    std::env::temp_dir().join("rbxcache-preview")
}

/// Starts a thread that checks every few seconds whether Roblox is running.
/// Looking that up spawns a process, which is too slow to do while drawing.
fn watch_roblox() -> Arc<AtomicBool> {
    let running = Arc::new(AtomicBool::new(false));
    let shared = running.clone();
    std::thread::spawn(move || loop {
        shared.store(store::roblox_running(), Ordering::Relaxed);
        std::thread::sleep(std::time::Duration::from_secs(4));
    });
    running
}

/// Light and dark tiles behind an image, so transparent areas read as transparent.
fn checkerboard(painter: &egui::Painter, rect: Rect) {
    const TILE: f32 = 12.0;
    painter.rect_filled(rect, CornerRadius::ZERO, Color32::from_rgb(34, 38, 48));
    let painter = painter.with_clip_rect(rect);
    let (columns, rows) = ((rect.width() / TILE).ceil() as usize, (rect.height() / TILE).ceil() as usize);
    for row in 0..rows.min(80) {
        for column in ((row % 2)..columns.min(160)).step_by(2) {
            let min = rect.min + Vec2::new(column as f32 * TILE, row as f32 * TILE);
            painter.rect_filled(Rect::from_min_size(min, Vec2::splat(TILE)), CornerRadius::ZERO, Color32::from_rgb(46, 51, 64));
        }
    }
}

impl AnimView {
    /// Pauses or resumes. Resuming from the end starts again from the beginning.
    fn toggle(&mut self, length: f32) {
        if !self.playing && self.time >= length {
            self.time = 0.0;
        }
        self.playing = !self.playing;
    }
}

impl Orbit {
    fn new((yaw, pitch): (f32, f32)) -> Self {
        Self { yaw, pitch, zoom: 1.0, texture: None, drawn: None }
    }

    fn reset(&mut self, (yaw, pitch): (f32, f32)) {
        (self.yaw, self.pitch, self.zoom) = (yaw, pitch, 1.0);
    }

    fn view(&self, bounds: ([f32; 3], f32)) -> viewers::View {
        viewers::View { zoom: self.zoom, ..viewers::View::new(bounds, (self.yaw, self.pitch)) }
    }

    /// Reserves the canvas and applies dragging and scrolling to the camera.
    /// Returns the canvas, its size in pixels, and whether the user is dragging.
    fn interact(&mut self, ui: &mut egui::Ui) -> (Rect, usize, usize, bool) {
        let width = ui.available_width();
        let height = (width * 0.72).min(380.0);
        let (rect, response) = ui.allocate_exact_size(Vec2::new(width, height), egui::Sense::click_and_drag());
        if response.dragged() {
            let delta = response.drag_delta();
            self.yaw += delta.x * 0.012;
            self.pitch = (self.pitch + delta.y * 0.012).clamp(-1.5, 1.5);
        }
        if response.hovered() {
            // Taken here so the surrounding panel does not scroll as well.
            let scroll = ui.input_mut(|i| std::mem::take(&mut i.smooth_scroll_delta).y);
            if scroll != 0.0 {
                self.zoom = (self.zoom * (1.0 + scroll * 0.002)).clamp(0.2, 12.0);
            }
        }
        let scale = ui.ctx().pixels_per_point();
        let pixels = |points: f32, most: usize| ((points * scale) as usize).clamp(64, most);
        (rect, pixels(width, 960), pixels(height, 640), response.dragged())
    }

    /// Shows the picture, calling `draw` again only when the camera, size or `stamp` changed.
    fn paint(&mut self, ui: &egui::Ui, rect: Rect, width: usize, height: usize, stamp: f32, draw: impl FnOnce(&Orbit) -> viewers::Rgba) {
        let key = (self.yaw, self.pitch, self.zoom, width, height, stamp);
        if self.drawn != Some(key) {
            let image = draw(self);
            let pixels = egui::ColorImage::from_rgba_unmultiplied([width, height], &image.pixels);
            match &mut self.texture {
                Some(texture) => texture.set(pixels, egui::TextureOptions::LINEAR),
                None => self.texture = Some(ui.ctx().load_texture("orbit", pixels, egui::TextureOptions::LINEAR)),
            }
            self.drawn = Some(key);
        }
        let painter = ui.painter();
        painter.rect_filled(rect, CornerRadius::same(10), theme::BG);
        if let Some(texture) = &self.texture {
            let uv = Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0));
            painter.image(texture.id(), rect, uv, Color32::WHITE);
        }
    }
}

/// The 3D mesh view: drag to rotate, scroll to zoom. It turns slowly until touched.
fn mesh_view(ui: &mut egui::Ui, view: &mut MeshView, motion: bool) {
    let (rect, width, height, dragged) = view.orbit.interact(ui);
    if dragged {
        view.spin = false;
    }
    if view.spin && motion {
        view.orbit.yaw += ui.input(|i| i.stable_dt).min(0.05) * 0.6;
        ui.ctx().request_repaint();
    }
    let (mesh, bounds) = (&view.mesh, view.bounds);
    view.orbit.paint(ui, rect, width, height, 0.0, |o| mesh.render(&o.view(bounds), width, height, viewers::MESH_TINT));
    ui.horizontal(|ui| {
        if ui.button("Reset view").clicked() {
            view.orbit.reset(viewers::MESH_ANGLES);
        }
        ui.checkbox(&mut view.spin, "Spin");
        ui.label(egui::RichText::new("Drag to rotate, scroll to zoom").color(theme::MUTED).small());
    });
}

/// Plays a keyframe animation on its rig, with a timeline to scrub through it.
fn anim_view(ui: &mut egui::Ui, animation: &Animation, view: &mut AnimView) {
    if view.playing {
        view.time += ui.input(|i| i.stable_dt).min(0.1) * view.speed;
        if view.time >= animation.length {
            if view.looping && animation.length > 0.0 {
                view.time %= animation.length;
            } else {
                (view.time, view.playing) = (animation.length, false);
            }
        }
        ui.ctx().request_repaint();
    }
    let (rect, width, height, _) = view.orbit.interact(ui);
    let (bounds, time) = (view.bounds, view.time);
    view.orbit.paint(ui, rect, width, height, time, |o| animation.pose(time).render(&o.view(bounds), width, height, viewers::MESH_TINT));
    ui.horizontal(|ui| {
        if ui.button(if view.playing { "Pause" } else { "Play" }).clicked() {
            view.toggle(animation.length);
        }
        ui.spacing_mut().slider_width = (ui.available_width() - 130.0).max(80.0);
        if ui.add(egui::Slider::new(&mut view.time, 0.0..=animation.length).show_value(false)).changed() {
            view.playing = false;
        }
        ui.label(egui::RichText::new(format!("{:.2} / {:.2}s", view.time, animation.length)).monospace());
    });
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new("Speed").color(theme::MUTED));
        ui.spacing_mut().slider_width = 120.0;
        ui.add(egui::Slider::new(&mut view.speed, 0.1..=2.0).fixed_decimals(1).suffix("x"));
        ui.checkbox(&mut view.looping, "Loop");
        if ui.button("Reset view").clicked() {
            view.orbit.reset(viewers::FIGURE_ANGLES);
        }
    });
}

/// Play, pause, seek and volume for the focused sound.
fn audio_controls(ui: &mut egui::Ui, audio: Option<&mut Audio>, id: Id, actions: &mut Vec<Action>) {
    let Some(audio) = audio.filter(|a| a.current == Some(id) && a.active()) else {
        ui.horizontal(|ui| {
            if ui.button("Play").clicked() {
                actions.push(Action::PlayAudio);
            }
            ui.label(egui::RichText::new("Space plays and pauses").color(theme::MUTED).small());
        });
        return;
    };
    let playing = !audio.paused();
    let position = audio.position();

    ui.horizontal(|ui| {
        if ui.button(if playing { "Pause" } else { "Resume" }).clicked() {
            audio.toggle_pause();
        }
        if ui.button("Stop").clicked() {
            audio.stop();
            return;
        }
        match audio.duration {
            Some(total) if total.as_secs_f32() > 0.0 => {
                let mut seconds = position.as_secs_f32().min(total.as_secs_f32());
                ui.spacing_mut().slider_width = (ui.available_width() - 190.0).max(80.0);
                if ui.add(egui::Slider::new(&mut seconds, 0.0..=total.as_secs_f32()).show_value(false)).changed() {
                    audio.seek(std::time::Duration::from_secs_f32(seconds));
                }
                ui.label(egui::RichText::new(format!("{} / {}", audio::clock(position), audio::clock(total))).monospace());
            }
            _ => {
                ui.label(egui::RichText::new(audio::clock(position)).monospace());
            }
        }
    });
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new("Volume").color(theme::MUTED));
        let mut volume = audio.volume();
        if ui.add(egui::Slider::new(&mut volume, 0.0..=1.0).show_value(false)).changed() {
            audio.set_volume(volume);
        }
    });
    if playing {
        ui.ctx().request_repaint();
    }
}

/// One instance of a model tree, with its properties and children folded inside.
fn node_ui(ui: &mut egui::Ui, node: &viewers::Node, id: egui::Id, depth: usize) {
    let mut title = egui::text::LayoutJob::default();
    let font = egui::TextStyle::Body.resolve(ui.style());
    title.append(&node.class, 0.0, egui::TextFormat { font_id: font.clone(), color: theme::ACCENT, ..Default::default() });
    title.append(&node.name, 8.0, egui::TextFormat { font_id: font, color: theme::TEXT, ..Default::default() });
    egui::CollapsingHeader::new(title).id_salt(id).default_open(depth == 0).show(ui, |ui| {
        if !node.properties.is_empty() {
            let label = egui::RichText::new(format!("{} properties", node.properties.len())).color(theme::MUTED).small();
            egui::CollapsingHeader::new(label).id_salt(id.with("properties")).show(ui, |ui| {
                let value_width = (ui.available_width() - 150.0).max(120.0);
                egui::Grid::new(id.with("grid")).num_columns(2).spacing([12.0, 4.0]).max_col_width(value_width).show(ui, |ui| {
                    for (key, value) in &node.properties {
                        ui.label(egui::RichText::new(key).color(theme::MUTED));
                        ui.add(egui::Label::new(value).wrap().selectable(true));
                        ui.end_row();
                    }
                });
            });
        }
        for (i, child) in node.children.iter().enumerate() {
            node_ui(ui, child, id.with(i), depth + 1);
        }
    });
}

/// Halves an image until neither side exceeds `max_side`.
fn shrink(mut image: viewers::Rgba, max_side: usize) -> viewers::Rgba {
    while image.width.max(image.height) > max_side {
        let (w, h) = ((image.width / 2).max(1), (image.height / 2).max(1));
        let mut pixels = Vec::with_capacity(w * h * 4);
        for y in 0..h {
            for x in 0..w {
                let at = ((y * 2).min(image.height - 1) * image.width + (x * 2).min(image.width - 1)) * 4;
                pixels.extend_from_slice(&image.pixels[at..at + 4]);
            }
        }
        image = viewers::Rgba { width: w, height: h, pixels };
    }
    image
}

fn build_preview(dir: &std::path::Path, entry: &Entry, idmap: &IdMap, ctx: &egui::Context, force: bool) -> Preview {
    let mut preview = Preview {
        id: entry.id,
        facts: Vec::new(),
        texture: None,
        mesh: None,
        anim: None,
        model: None,
        font: None,
        audio: false,
        deferred: false,
        text: None,
        hex: String::new(),
        error: None,
    };
    let loaded = Store::open(dir, false).and_then(|store| {
        let (raw, header) = store.read_parsed(&entry.id)?;
        Ok((store, raw, header))
    });
    let (store, raw, header) = match loaded {
        Ok(loaded) => loaded,
        Err(e) => {
            preview.error = Some(format!("{e:#}"));
            return preview;
        }
    };
    preview.facts = describe(&store, entry, &raw, header.as_ref());
    let body = &raw[header.as_ref().map_or(0, |h| h.body_offset)..];
    // Fingerprinting hashes the whole payload, so skip it when nothing is tagged.
    if !idmap.assets.is_empty() {
        let tagged = idmap.assets_for(&config::fingerprint(body));
        if !tagged.is_empty() {
            preview.facts.insert(1, ("Asset ID", tagged.join(", ")));
        }
    }
    let show_image = |preview: &mut Preview, image: viewers::Rgba| {
        let image = shrink(image, 2048);
        let pixels = egui::ColorImage::from_rgba_unmultiplied([image.width, image.height], &image.pixels);
        preview.texture = Some(ctx.load_texture("preview", pixels, egui::TextureOptions::LINEAR));
    };
    // Models and meshes this large take a moment to parse, so wait to be asked.
    let heavy = body.len() > 6 << 20 && matches!(entry.kind, "rbxm" | "rbxmx" | "mesh");
    match entry.kind {
        _ if heavy && !force => preview.deferred = true,
        "png" | "jpeg" | "gif" | "webp" => match image::load_from_memory(body) {
            Ok(img) => {
                let rgba = img.to_rgba8();
                show_image(&mut preview, viewers::Rgba { width: rgba.width() as usize, height: rgba.height() as usize, pixels: rgba.into_raw() });
            }
            Err(e) => preview.error = Some(format!("Could not decode the image: {e}")),
        },
        "ktx2" => match viewers::decode_ktx2(body, 2048) {
            Ok(image) => {
                if let Some(pack) = crate::texture::Pack::parse(body) {
                    let whole = pack.full_size().map_or(String::new(), |(w, h)| format!(" of a {w} x {h} texture"));
                    let levels = format::count(pack.levels as usize, "mip level", "mip levels");
                    preview.facts.push(("Texture pack", format!("{} x {}, {levels}{whole}", pack.width, pack.height)));
                    preview.facts.push(("Content hash", pack.content_hash));
                }
                show_image(&mut preview, image)
            }
            Err(e) => preview.error = Some(format!("Could not decode the texture: {e:#}")),
        },
        "mesh" => match viewers::parse_mesh(body) {
            Ok(mesh) => {
                preview
                    .facts
                    .push(("Geometry", format!("version {}, {} vertices, {} triangles", mesh.version, mesh.positions.len(), mesh.faces.len())));
                // Very dense meshes are redrawn only when moved, not spun continuously.
                let spin = mesh.faces.len() <= 300_000;
                preview.mesh = Some(MeshView { bounds: mesh.bounds(), mesh, orbit: Orbit::new(viewers::MESH_ANGLES), spin });
            }
            Err(e) => preview.error = Some(format!("Could not read the mesh: {e:#}")),
        },
        "rbxm" | "rbxmx" => match viewers::parse_model(body, entry.kind == "rbxmx") {
            Ok(model) => {
                if let Some(animation) = &model.animation {
                    let summary = format!(
                        "{} rig, {} keyframes, {:.2} s{}",
                        animation.rig.name(),
                        animation.keyframes,
                        animation.length,
                        if animation.looped { ", loops" } else { "" }
                    );
                    preview.facts.push(("Animation", summary));
                    preview.anim = Some(AnimView {
                        orbit: Orbit::new(viewers::FIGURE_ANGLES),
                        bounds: animation.bounds(),
                        time: 0.0,
                        playing: true,
                        speed: 1.0,
                        looping: animation.looped,
                    });
                }
                preview.model = Some(model);
            }
            Err(e) => {
                preview.error = Some(format!("Could not read the model: {e:#}"));
                if entry.kind == "rbxmx" {
                    preview.text = Some(String::from_utf8_lossy(&body[..body.len().min(8000)]).into_owned());
                }
            }
        },
        kind if format::is_audio(kind) => preview.audio = true,
        "ttf" | "otf" => match ab_glyph::FontRef::try_from_slice(body) {
            Ok(_) => {
                let name = format!("cache-font-{}", entry.id_hex);
                let family = egui::FontFamily::Name(name.clone().into());
                ctx.add_font(egui::epaint::text::FontInsert::new(
                    &name,
                    egui::FontData::from_owned(body.to_vec()),
                    vec![egui::epaint::text::InsertFontFamily { family: family.clone(), priority: egui::epaint::text::FontPriority::Highest }],
                ));
                preview.font = Some((family, ctx.cumulative_pass_nr()));
            }
            Err(e) => preview.error = Some(format!("Could not read the font: {e}")),
        },
        "json" | "asset-map" => {
            // Re-indenting parses the whole document, so large ones are shown as stored.
            let pretty = if body.len() <= 2 << 20 { viewers::pretty_json(body) } else { None };
            let text = pretty.unwrap_or_else(|| String::from_utf8_lossy(&body[..body.len().min(80_000)]).into_owned());
            preview.text = Some(text.chars().take(20_000).collect());
        }
        kind if format::is_text(kind) => preview.text = Some(String::from_utf8_lossy(&body[..body.len().min(8000)]).into_owned()),
        _ => {}
    }
    preview.hex = format::hex_dump(body, 256);
    preview
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{Action as Does, Rule, Target};

    fn config(name: &str, rules: &[(&str, &[u64])]) -> LoadedConfig {
        let rules = rules
            .iter()
            .map(|(rule, ids)| Rule { name: rule.to_string(), targets: ids.iter().map(|id| Target::Asset(*id)).collect(), action: Does::Remove })
            .collect();
        LoadedConfig { name: name.into(), path: format!("{name}.json").into(), rules, enabled: true, off: BTreeSet::new(), error: None }
    }

    /// The asset IDs each rule in force sets, by rule name.
    fn in_force(configs: &[LoadedConfig], winners: &BTreeMap<String, PathBuf>) -> Vec<(String, Vec<String>)> {
        rules_in_force(configs, winners).into_iter().map(|r| (r.name, r.targets.iter().map(|t| t.to_string()).collect())).collect()
    }

    #[test]
    fn a_shared_target_goes_to_one_config() {
        let mut configs = vec![config("a", &[("sky", &[1, 2]), ("font", &[3])]), config("b", &[("night", &[2, 3, 4])])];
        let found = conflicts(&configs);
        assert_eq!(found.iter().map(|c| c.target.as_str()).collect::<Vec<_>>(), ["asset 2", "asset 3"]);
        assert_eq!(found[0].sides, [(0, "sky".to_string()), (1, "night".to_string())]);

        // Left alone, the config lower in the list has what both set.
        let mut winners = BTreeMap::new();
        let own = |name: &str, ids: &[&str]| (name.to_string(), ids.iter().map(|id| id.to_string()).collect::<Vec<_>>());
        assert_eq!(in_force(&configs, &winners), [own("a / sky", &["asset 1"]), own("b / night", &["asset 2", "asset 3", "asset 4"])]);
        // Chosen one by one.
        winners.insert("asset 2".to_string(), PathBuf::from("a.json"));
        assert_eq!(in_force(&configs, &winners), [own("a / sky", &["asset 1", "asset 2"]), own("b / night", &["asset 3", "asset 4"])]);

        // A rule that is switched off takes no part, so there is nothing left to choose for its targets.
        configs[1].off.insert("night".to_string());
        assert!(conflicts(&configs).is_empty());
        assert_eq!(in_force(&configs, &winners), [own("a / sky", &["asset 1", "asset 2"]), own("a / font", &["asset 3"])]);
        // Nor does a config that is switched off, whatever was chosen for it.
        configs[1].off.clear();
        configs[0].enabled = false;
        assert_eq!(in_force(&configs, &winners), [own("night", &["asset 2", "asset 3", "asset 4"])]);
    }

    #[test]
    fn rules_that_share_a_name_are_told_apart() {
        let config = config("a", &[("font", &[1]), ("icon", &[2]), ("font", &[3]), ("font", &[4])]);
        assert_eq!(rule_keys(&config.rules), ["font", "icon", "font #2", "font #3"]);
        assert_eq!(Tab::named("Textures"), Some(Tab::Group(1)));
        assert_eq!((Tab::named("guide"), Tab::named("nope")), (Some(Tab::Guide), None));
        assert!(Tab::Other.holds("binary") && !Tab::Other.holds("redirect") && !Tab::Other.holds("png"));
    }
}
