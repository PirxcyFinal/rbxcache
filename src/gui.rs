//! The window: entry table, detail panel with previews, menus and dialogs.

use crate::anim::Animation;
use crate::audio::{self, Audio};
use crate::cli::{describe, export_name, matches};
use crate::config::{self, IdMap};
use crate::format::{self, human_size};
use crate::protection::Protections;
use crate::store::{self, category_name, format_time_ms, Entry, Id, Store};
use crate::theme::{self, alpha};
use crate::viewers;
use crate::watch::{self, Change, Tracker, Verdict, Watcher};
use anyhow::{anyhow, Context, Result};
use eframe::egui::{self, Color32, CornerRadius, Margin, Rect, Shape, Stroke, Vec2};
use egui_extras::{Column, TableBuilder};
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{channel, Receiver};
use std::sync::Arc;

const ROW_HEIGHT: f32 = 24.0;
/// Seconds a newly written entry keeps its marker in the table.
const FRESH_SECONDS: f32 = 10.0;
/// Seconds the detail panel takes to settle after the focused entry changes.
const PANEL_TIME: f32 = 0.32;

pub fn run(dir: PathBuf, idmap_path: PathBuf) -> Result<()> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_inner_size([1400.0, 860.0]).with_min_inner_size([960.0, 560.0]),
        ..Default::default()
    };
    eframe::run_native("Roblox Cache Explorer", options, Box::new(move |cc| Ok(Box::new(App::new(cc, dir, idmap_path))))).map_err(|e| anyhow!("{e}"))
}

/// Which list the table shows.
#[derive(Clone, Copy, PartialEq)]
enum Tab {
    /// Everything in the cache.
    All,
    /// Only what Roblox has written since the window opened or the list was cleared.
    Live,
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

#[derive(Clone, Copy)]
enum Action {
    Reload,
    OpenFolder,
    Export,
    ExportMany,
    Replace,
    OpenExternally,
    AskDelete,
    Delete,
    ApplyConfig,
    ConfirmConfig,
    Tag,
    CopyId,
    SortBy(SortKey),
    ShowTab(Tab),
    ClearLive,
    SortByArrival,
    SetSource,
    ReplaceFromSource,
    StopConfig,
    PlayAudio,
    ForcePreview,
    ExportConverted,
    About,
    Quit,
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

/// A config that has been checked against the cache and awaits confirmation.
struct PendingConfig {
    path: PathBuf,
    rules: Vec<config::Rule>,
    summary: String,
    notes: Vec<String>,
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
    /// The sort order of the tab that is not showing, restored when switching back.
    other_sort: (SortKey, bool),
    /// Entries Roblox wrote while watching, oldest first.
    arrivals: Vec<Id>,

    selected: BTreeSet<Id>,
    focus: Option<Id>,
    anchor: Option<usize>,
    preview: Option<Preview>,
    confirm_delete: bool,
    delete_prompt: String,
    pending_config: Option<PendingConfig>,
    confirm_config: bool,
    /// A config that is re-applied to whatever Roblox writes while watching.
    active_config: Option<(String, Vec<config::Rule>)>,
    /// Follow the cache as Roblox changes it.
    live: bool,
    /// Changes arriving from the watcher thread, and the flag that stops it.
    watch: Option<(Receiver<Change>, Arc<AtomicBool>)>,
    tracker: Tracker,
    protections: Option<Protections>,
    /// When each recently written entry arrived, for the marker beside it.
    fresh: HashMap<Id, f64>,
    /// The entry "Replace with source" copies from.
    source: Option<Id>,
    show_about: bool,
    status: String,

    /// Animations on. Off draws every change instantly.
    motion: bool,
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
    fn new(cc: &eframe::CreationContext<'_>, dir: PathBuf, idmap_path: PathBuf) -> Self {
        theme::apply(&cc.egui_ctx);
        let (idmap, idmap_error) = match IdMap::load(&idmap_path) {
            Ok(map) => (map, None),
            Err(e) => (IdMap::default(), Some(format!("{e:#}"))),
        };
        // Copies made by "Open" in earlier sessions.
        let _ = std::fs::remove_dir_all(open_dir());
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
            other_sort: (SortKey::Arrival, false),
            arrivals: Vec::new(),
            selected: BTreeSet::new(),
            focus: None,
            anchor: None,
            preview: None,
            confirm_delete: false,
            delete_prompt: String::new(),
            pending_config: None,
            confirm_config: false,
            active_config: None,
            live: true,
            watch: None,
            tracker: Tracker::default(),
            protections: Protections::load_default().ok(),
            fresh: HashMap::new(),
            source: None,
            show_about: false,
            motion: true,
            blob: Blob::default(),
            panel_time: 0.0,
            toast: None,
            scroll_to: None,
            focus_search: false,
            audio: None,
            force_preview: false,
        };
        app.start_load(&cc.egui_ctx);
        app
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
                if self.live {
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
            let Ok(mut watcher) = Watcher::new(&dir, stamps.into_iter()) else { return };
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

    /// Folds in what the watcher saw, and re-applies the active config to it.
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
        if let Some((name, rules)) = &self.active_config {
            let targets: HashSet<Id> = written.iter().filter(|id| self.tracker.may_replace(id)).copied().collect();
            if !targets.is_empty() {
                let reports = Store::open(&self.dir, true).map(|store| config::run(&store, &self.entries, &self.idmap, rules, true, Some(&targets)));
                let rewritten: Vec<Entry> = reports.into_iter().flatten().flat_map(|r| r.written).collect();
                if !rewritten.is_empty() {
                    self.status = format!("{name}: re-applied to {}", format::count(rewritten.len(), "entry", "entries"));
                    for entry in &rewritten {
                        self.tracker.record(entry);
                    }
                    watch::merge(&mut self.entries, &Change { written: rewritten, ..Default::default() });
                }
            }
        }
        if self.focus.is_some_and(|id| written.contains(&id)) {
            self.preview = None;
        }
        self.reindex();
        self.selected.retain(|id| self.index.contains_key(id));
        if self.focus.is_some_and(|id| !self.index.contains_key(&id)) {
            self.focus = None;
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
        self.kinds = kinds.into_iter().collect();
        self.categories = categories.into_iter().collect();
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
            Tab::All => HashMap::new(),
        };
        let mut view: Vec<usize> = (0..entries.len())
            .filter(|&i| self.tab == Tab::All || arrived.contains_key(&entries[i].id))
            .filter(|&i| {
                let e = &entries[i];
                (redirects || e.kind != "redirect")
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
        // While the cache is being read the list is stale, so nothing may act on it.
        if self.loading.is_some() && !matches!(action, Action::About | Action::Quit | Action::OpenFolder | Action::ShowTab(_)) {
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
                if self.tab != tab {
                    self.tab = tab;
                    // Each tab keeps its own sort order.
                    let current = (self.sort, self.ascending);
                    (self.sort, self.ascending) = std::mem::replace(&mut self.other_sort, current);
                    self.rebuild_view();
                }
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
            Action::About => {
                self.show_about = true;
                Ok(String::new())
            }
            Action::Quit => {
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
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
            Action::StopConfig => match self.active_config.take() {
                Some((name, _)) => Ok(format!("{name} is no longer re-applied")),
                None => Ok(String::new()),
            },
            Action::ApplyConfig => self.check_config(),
            Action::ConfirmConfig => self.apply_config(),
            Action::Tag => self.tag_focused(),
            Action::PlayAudio => self.play_audio(),
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
        let updated = Store::open(&self.dir, true)?.replace_body(&id, &body)?;
        if let Some(protections) = &mut self.protections {
            protections.remember(&id, &body)?;
        }
        let msg = format!("Replaced {} with {} ({})", updated.id_hex, path.display(), human_size(updated.size));
        self.replaced(updated);
        Ok(msg)
    }

    /// Copies the source entry's content over the focused entry.
    fn replace_from_source(&mut self) -> Result<String> {
        let (Some(e), Some(source)) = (self.focused(), self.source) else { return Ok(String::new()) };
        let id = e.id;
        if id == source {
            return Err(anyhow!("This entry is the replacement source itself"));
        }
        let store = Store::open(&self.dir, true)?;
        let body = store.read_body(&source)?;
        let updated = store.replace_body(&id, &body)?;
        if let Some(protections) = &mut self.protections {
            protections.remember(&id, &body)?;
        }
        let msg = format!("Replaced {} with the content of {}", &updated.id_hex[..12], &store::hex(&source)[..12]);
        self.replaced(updated);
        Ok(msg)
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

    /// Picks a config and works out what it would change, then asks before applying it.
    fn check_config(&mut self) -> Result<String> {
        let Some(path) = rfd::FileDialog::new().set_title("Choose a replacement config").add_filter("Config", &["json"]).pick_file() else {
            return Ok(String::new());
        };
        let rules = config::load(&path)?;
        let reports = config::run(&Store::open(&self.dir, false)?, &self.entries, &self.idmap, &rules, false, None);
        let notes = reports.iter().flat_map(|r| r.notes.iter().map(move |n| format!("{}: {n}", r.name))).collect();
        self.pending_config = Some(PendingConfig { path, rules, summary: config::summarize(&reports, false), notes });
        self.confirm_config = true;
        Ok(String::new())
    }

    fn apply_config(&mut self) -> Result<String> {
        let Some(pending) = self.pending_config.take() else { return Ok(String::new()) };
        let store = Store::open(&self.dir, true)?;
        let reports = config::run(&store, &self.entries, &self.idmap, &pending.rules, true, None);
        let summary = config::summarize(&reports, true);
        let rewritten: Vec<Entry> = reports.into_iter().flat_map(|r| r.written).collect();
        for entry in &rewritten {
            self.tracker.record(entry);
        }
        watch::merge(&mut self.entries, &Change { written: rewritten, ..Default::default() });
        self.preview = None;
        self.reindex();
        // From here on the config follows the cache: whatever Roblox writes is checked against it.
        let name = pending.path.file_stem().map_or_else(|| "config".to_string(), |n| n.to_string_lossy().into_owned());
        self.active_config = Some((name, pending.rules));
        Ok(summary)
    }

    fn play_audio(&mut self) -> Result<String> {
        let Some(e) = self.focused() else { return Ok(String::new()) };
        if !format::is_audio(e.kind) {
            return Ok(String::new());
        }
        let id = e.id;
        if let Some(audio) = self.audio.as_mut().filter(|a| a.current == Some(id) && a.active()) {
            audio.toggle_pause();
            return Ok(String::new());
        }
        let body = Store::open(&self.dir, false)?.read_body(&id)?;
        if self.audio.is_none() {
            self.audio = Some(Audio::open()?);
        }
        self.audio.as_mut().expect("audio was just opened").play(id, body)?;
        Ok(String::new())
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
                ui.separator();
                item(ui, actions, true, "Apply config...", "Ctrl+O", Action::ApplyConfig);
                item(ui, actions, self.active_config.is_some(), "Stop re-applying config", "", Action::StopConfig);
                ui.separator();
                item(ui, actions, true, "Quit", "Alt+F4", Action::Quit);
            });
            ui.menu_button("Entry", |ui| {
                ui.set_min_width(230.0);
                entry_items(ui, actions, one, any, self.source.is_some());
            });
            ui.menu_button("View", |ui| {
                ui.set_min_width(210.0);
                if ui.checkbox(&mut self.show_redirects, "Show redirect records").changed() {
                    self.rebuild_view();
                }
                ui.checkbox(&mut self.motion, "Animations");
                if ui.checkbox(&mut self.live, "Watch cache live").changed() {
                    match self.live {
                        true => self.start_watch(ui.ctx()),
                        false => self.stop_watch(),
                    }
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
                item(ui, actions, true, "About and shortcuts", "F1", Action::About);
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
            for (tab, label) in [(Tab::All, format!("All  {}", self.entries.len())), (Tab::Live, format!("Live  {}", self.arrivals.len()))] {
                if ui.selectable_label(self.tab == tab, label).clicked() {
                    actions.push(Action::ShowTab(tab));
                }
            }
            if self.tab == Tab::Live
                && ui.add_enabled(!self.arrivals.is_empty(), egui::Button::new("Clear")).on_hover_text("Empty the Live list").clicked()
            {
                actions.push(Action::ClearLive);
            }
            if self.tab == Tab::Live && self.sort != SortKey::Arrival && ui.button("Newest first").on_hover_text("Go back to arrival order").clicked()
            {
                actions.push(Action::SortByArrival);
            }
            ui.separator();
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
                for &(kind, n) in &self.kinds {
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
        let t = if self.motion { theme::ease_out(((now - self.panel_time) as f32) / PANEL_TIME) } else { 1.0 };
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
        let (motion, audio) = (self.motion, &mut self.audio);
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
            blob.moving = self.motion && blob.visible;
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

    /// Shown on the Live tab until Roblox writes something.
    fn live_placeholder(&self, ui: &mut egui::Ui) {
        ui.add_space(ui.available_height() * 0.36);
        ui.vertical_centered(|ui| {
            if self.watch.is_none() {
                ui.label(egui::RichText::new("Live watching is off").size(20.0));
                ui.label(egui::RichText::new("Turn on View > Watch cache live to fill this list.").color(theme::MUTED));
                return;
            }
            // A dot that pulses while waiting.
            let (dot, _) = ui.allocate_exact_size(Vec2::splat(16.0), egui::Sense::hover());
            let pulse = if self.motion { (ui.input(|i| i.time) * 2.2).sin() as f32 * 0.5 + 0.5 } else { 1.0 };
            ui.painter().circle_filled(dot.center(), 4.0 + 2.0 * pulse, alpha(theme::OK, 0.45 + 0.55 * pulse));
            ui.add_space(4.0);
            ui.label(egui::RichText::new("Waiting for Roblox").size(20.0));
            let state = if self.roblox_running {
                "Assets appear here as Roblox downloads them."
            } else {
                "Roblox is not running. Assets appear here as it downloads them."
            };
            ui.label(egui::RichText::new(state).color(theme::MUTED));
            if self.motion {
                ui.ctx().request_repaint();
            }
        });
    }

    fn loading_card(&self, ui: &mut egui::Ui) {
        let (done, total) = self.progress;
        let fraction = if total > 0 { done as f32 / total as f32 } else { 0.0 };
        let shown = ui.ctx().animate_value_with_time(egui::Id::new("load_progress"), fraction, 0.25);
        ui.add_space(ui.available_height() * 0.36);
        ui.vertical_centered(|ui| {
            ui.label(egui::RichText::new("Reading the Roblox cache").size(20.0));
            ui.add_space(2.0);
            let detail = if total > 0 { format!("Scanning files  {done} / {total}") } else { "Opening the index...".to_string() };
            ui.label(egui::RichText::new(detail).color(theme::MUTED));
            ui.add_space(12.0);
            let (track, _) = ui.allocate_exact_size(Vec2::new(360.0, 8.0), egui::Sense::hover());
            let radius = CornerRadius::same(4);
            let painter = ui.painter();
            painter.rect_filled(track, radius, theme::RAISED);
            let fill = Rect::from_min_size(track.min, Vec2::new(track.width() * shown.max(0.03), track.height()));
            painter.rect_filled(fill, radius, theme::ACCENT);
            if self.motion {
                // A soft highlight sweeping along the filled part.
                let phase = (ui.input(|i| i.time) * 0.9).fract() as f32;
                let x = track.left() - 80.0 + phase * (track.width() + 160.0);
                let sheen = Rect::from_min_size(egui::pos2(x, track.top()), Vec2::new(80.0, track.height()));
                painter.with_clip_rect(fill).rect_filled(sheen, radius, Color32::from_white_alpha(70));
            }
        });
        ui.ctx().request_repaint();
    }

    /// A centred dialog that fades and slides in over a dimmed window. Returns
    /// true when the user dismissed it (Escape or a click outside).
    fn modal(&self, ctx: &egui::Context, name: &str, open: bool, add: impl FnOnce(&mut egui::Ui)) -> bool {
        let time = if self.motion { 0.22 } else { 0.0 };
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

    fn config_dialog(&mut self, ctx: &egui::Context, actions: &mut Vec<Action>) {
        let Some(pending) = &self.pending_config else { return };
        let mut close = false;
        let dismissed = self.modal(ctx, "config", self.confirm_config, |ui| {
            ui.label(egui::RichText::new("Apply config").size(18.0));
            ui.add_space(4.0);
            ui.label(egui::RichText::new(pending.path.display().to_string()).color(theme::MUTED).small());
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
                    && self.confirm_config
                {
                    actions.push(Action::ConfirmConfig);
                    close = true;
                }
                if ui.button("Cancel").clicked() {
                    close = true;
                }
            });
        });
        if close || dismissed {
            self.confirm_config = false;
        }
    }

    fn about_dialog(&mut self, ctx: &egui::Context) {
        let mut close = false;
        let dismissed = self.modal(ctx, "about", self.show_about, |ui| {
            ui.label(egui::RichText::new("Roblox Cache Explorer").size(20.0));
            ui.label(egui::RichText::new(format!("rbxcache {}", env!("CARGO_PKG_VERSION"))).color(theme::MUTED));
            ui.add_space(10.0);
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
                    ("Ctrl+O", "Apply config"),
                    ("Delete", "Delete the selection"),
                    ("F5", "Reload the cache"),
                    ("F1", "This window"),
                ] {
                    ui.label(egui::RichText::new(keys).monospace().color(theme::ACCENT));
                    ui.label(what);
                    ui.end_row();
                }
            });
            ui.add_space(12.0);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button("Close").clicked() {
                    close = true;
                }
            });
        });
        if close || dismissed {
            self.show_about = false;
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
        let (opacity, travel, speed) = if !self.motion {
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
        if self.confirm_delete || self.confirm_config || self.show_about {
            return;
        }
        let typing = ctx.wants_keyboard_input();
        ctx.input_mut(|i| {
            use egui::{Key, KeyboardShortcut, Modifiers};
            if i.consume_shortcut(&KeyboardShortcut::new(Modifiers::COMMAND, Key::F)) {
                self.focus_search = true;
            }
            if i.consume_shortcut(&KeyboardShortcut::new(Modifiers::COMMAND, Key::O)) {
                actions.push(Action::ApplyConfig);
            }
            if i.consume_shortcut(&KeyboardShortcut::new(Modifiers::COMMAND, Key::E)) {
                actions.push(if self.selected.len() > 1 { Action::ExportMany } else { Action::Export });
            }
            if i.key_pressed(Key::F5) {
                actions.push(Action::Reload);
            }
            if i.key_pressed(Key::F1) {
                actions.push(Action::About);
            }
            if typing || self.loading.is_some() {
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
        self.poll_load(ctx);
        self.poll_watch(ctx);
        let now = ctx.input(|i| i.time);
        self.fresh.retain(|_, at| ((now - *at) as f32) < FRESH_SECONDS);
        if !self.fresh.is_empty() {
            ctx.request_repaint();
        }
        self.roblox_running = self.roblox.load(Ordering::Relaxed);
        // A sound belongs to the entry it was started from.
        if let Some(audio) = self.audio.as_mut().filter(|a| a.current.is_some() && a.current != self.focus) {
            audio.stop();
        }
        self.ensure_preview(ctx);
        let mut actions = Vec::new();
        self.shortcuts(ctx, &mut actions);

        let bar = egui::Frame::new().fill(theme::PANEL).inner_margin(Margin::symmetric(10, 6));
        egui::TopBottomPanel::top("menu").frame(bar).show(ctx, |ui| self.menu_bar(ui, &mut actions));
        egui::TopBottomPanel::top("toolbar").frame(bar.inner_margin(Margin::symmetric(10, 8))).show(ctx, |ui| self.toolbar(ui, &mut actions));
        egui::TopBottomPanel::bottom("status").frame(bar).show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new(&self.status).color(theme::MUTED).small());
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if self.watch.is_some() {
                        theme::pill(ui, "Live", theme::OK);
                    }
                    if let Some((name, _)) = &self.active_config {
                        theme::pill(ui, &format!("Applying {name}"), theme::ACCENT);
                    }
                    if !self.selected.is_empty() {
                        let text = format!("{} selected  -  {}", self.selected.len(), human_size(self.selected_bytes()));
                        ui.label(egui::RichText::new(text).color(theme::ACCENT).small());
                    }
                });
            });
        });
        egui::SidePanel::right("details")
            .resizable(true)
            .default_width(540.0)
            .width_range(340.0..=820.0)
            .frame(egui::Frame::new().fill(theme::PANEL).inner_margin(Margin::same(14)))
            .show(ctx, |ui| self.details(ui, &mut actions));
        egui::CentralPanel::default().frame(egui::Frame::new().fill(theme::BG).inner_margin(Margin::symmetric(10, 6))).show(ctx, |ui| {
            if let Some(error) = &self.load_error {
                ui.add_space(ui.available_height() * 0.36);
                ui.vertical_centered(|ui| {
                    ui.label(egui::RichText::new("Could not load the cache").size(20.0));
                    ui.colored_label(theme::DANGER, error);
                    if ui.button("Try again").clicked() {
                        actions.push(Action::Reload);
                    }
                });
            } else if self.loading.is_some() {
                self.loading_card(ui);
            } else if self.tab == Tab::Live && self.arrivals.is_empty() {
                self.live_placeholder(ui);
            } else {
                self.table(ui, &mut actions);
            }
        });
        self.delete_dialog(ctx, &mut actions);
        self.config_dialog(ctx, &mut actions);
        self.about_dialog(ctx);
        self.show_toast(ctx);

        for action in actions {
            self.run_action(action, ctx);
            // Actions run after this frame was laid out, so draw another to show the result.
            ctx.request_repaint();
        }
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
            Ok(image) => show_image(&mut preview, image),
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
