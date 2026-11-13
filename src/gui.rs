//! The graphical explorer.

use crate::cli::{describe, export_name, matches};
use crate::config::{self, IdMap};
use crate::format::{self, human_size};
use crate::store::{self, category_name, format_time_ms, Entry, Id, Store};
use anyhow::{anyhow, Context, Result};
use eframe::egui;
use egui_extras::{Column, TableBuilder};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::PathBuf;
use std::sync::mpsc::{channel, Receiver};

pub fn run(dir: PathBuf, idmap_path: PathBuf) -> Result<()> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_inner_size([1360.0, 840.0]).with_min_inner_size([900.0, 520.0]),
        ..Default::default()
    };
    eframe::run_native("Roblox Cache Explorer", options, Box::new(move |cc| Ok(Box::new(App::new(cc, dir, idmap_path)))))
        .map_err(|e| anyhow!("{e}"))
}

#[derive(Clone, Copy, PartialEq)]
enum SortKey {
    Kind,
    Category,
    Size,
    Hits,
    Used,
    Id,
}

enum LoadMsg {
    Progress(usize, usize),
    Done(Result<Vec<Entry>, String>),
}

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
    Tag,
    CopyId,
}

struct Preview {
    id: Id,
    facts: Vec<(&'static str, String)>,
    texture: Option<egui::TextureHandle>,
    text: Option<String>,
    hex: String,
    error: Option<String>,
}

struct App {
    dir: PathBuf,
    idmap_path: PathBuf,
    idmap: IdMap,
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
    roblox_running: bool,

    search: String,
    kind_filter: Option<&'static str>,
    category_filter: Option<i64>,
    show_redirects: bool,
    sort: SortKey,
    ascending: bool,

    selected: BTreeSet<Id>,
    focus: Option<Id>,
    anchor: Option<usize>,
    preview: Option<Preview>,
    confirm_delete: bool,
    status: String,
}

impl App {
    fn new(cc: &eframe::CreationContext<'_>, dir: PathBuf, idmap_path: PathBuf) -> Self {
        // Row clicks select entries, so labels must not swallow them for text selection.
        cc.egui_ctx.style_mut(|s| s.interaction.selectable_labels = false);
        let (idmap, idmap_error) = match IdMap::load(&idmap_path) {
            Ok(map) => (map, String::new()),
            Err(e) => (IdMap::default(), format!("Error: {e:#}")),
        };
        let mut app = Self {
            dir,
            idmap_path,
            idmap,
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
            roblox_running: false,
            search: String::new(),
            kind_filter: None,
            category_filter: None,
            show_redirects: false,
            sort: SortKey::Size,
            ascending: false,
            selected: BTreeSet::new(),
            focus: None,
            anchor: None,
            preview: None,
            confirm_delete: false,
            status: idmap_error,
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
        self.loading = Some(rx);
        self.progress = (0, 0);
        self.load_error = None;
        self.roblox_running = store::roblox_running();
    }

    fn poll_load(&mut self) {
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
                self.selected.clear();
                self.focus = None;
                self.preview = None;
                self.reindex();
                self.status = format!("Loaded {} entries", self.entries.len());
            }
            Some(Err(e)) => {
                self.loading = None;
                self.load_error = Some(e);
            }
            None => {}
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
        let mut view: Vec<usize> = (0..entries.len())
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
                if !self.selected.remove(&id) {
                    self.selected.insert(id);
                }
                self.anchor = Some(pos);
            }
            _ => {
                self.selected.clear();
                self.selected.insert(id);
                self.anchor = Some(pos);
            }
        }
        self.focus = Some(id);
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
        self.preview = Some(build_preview(&self.dir, entry, &self.idmap, ctx));
    }

    fn run_action(&mut self, action: Action, ctx: &egui::Context) {
        let result = match action {
            Action::Reload => {
                self.start_load(ctx);
                Ok(String::new())
            }
            Action::OpenFolder => open::that(self.dir.join("rbx-storage")).map(|()| String::new()).map_err(Into::into),
            Action::CopyId => {
                if let Some(e) = self.focused() {
                    ctx.copy_text(e.id_hex.clone());
                }
                Ok("ID copied".into())
            }
            Action::AskDelete => {
                if !self.selected.is_empty() {
                    self.roblox_running = store::roblox_running();
                    self.confirm_delete = true;
                }
                Ok(String::new())
            }
            Action::Export => self.export_one(),
            Action::ExportMany => self.export_many(),
            Action::OpenExternally => self.open_externally(),
            Action::Replace => self.replace(),
            Action::Delete => self.delete_selected(),
            Action::ApplyConfig => self.apply_config(ctx),
            Action::Tag => self.tag_focused(),
        };
        match result {
            Ok(msg) if msg.is_empty() => {}
            Ok(msg) => self.status = msg,
            Err(e) => self.status = format!("Error: {e:#}"),
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

    fn export_many(&mut self) -> Result<String> {
        let Some(folder) = rfd::FileDialog::new().pick_folder() else { return Ok(String::new()) };
        let store = Store::open(&self.dir, false)?;
        let mut n = 0;
        for id in &self.selected {
            let Some(&i) = self.index.get(id) else { continue };
            let e = &self.entries[i];
            std::fs::write(folder.join(export_name(e, false)), store.read_body(&e.id)?)?;
            n += 1;
        }
        Ok(format!("Exported {n} entries to {}", folder.display()))
    }

    fn open_externally(&mut self) -> Result<String> {
        let Some(e) = self.focused() else { return Ok(String::new()) };
        let folder = std::env::temp_dir().join("rbxcache-preview");
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
        let msg = format!("Replaced {} with {} ({})", updated.id_hex, path.display(), human_size(updated.size));
        if let Some(&i) = self.index.get(&id) {
            self.entries[i] = updated;
        }
        self.preview = None;
        self.reindex();
        Ok(msg)
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
            Some(e) => Err(anyhow!("deleted {}, {failed} failed: {e:#}", deleted.len())),
            None => Ok(format!("Deleted {} entries", deleted.len())),
        }
    }

    fn apply_config(&mut self, ctx: &egui::Context) -> Result<String> {
        let Some(path) = rfd::FileDialog::new().set_title("Choose a replacement config").add_filter("Config", &["json"]).pick_file() else {
            return Ok(String::new());
        };
        let rules = config::load(&path)?;
        let store = Store::open(&self.dir, true)?;
        let reports = config::run(&store, &self.entries, &self.idmap, &rules, true);
        let mut msg = config::summarize(&reports, true);
        if let Some(note) = reports.iter().flat_map(|r| r.notes.iter().map(move |n| format!("{}: {n}", r.name))).next() {
            msg.push_str(&format!(" (first: {note})"));
        }
        self.start_load(ctx);
        Ok(msg)
    }

    fn tag_focused(&mut self) -> Result<String> {
        let Some(e) = self.focused() else { return Ok(String::new()) };
        let asset: u64 = self.tag_input.trim().parse().ok().filter(|&n| n > 0).context("enter the numeric Roblox asset ID first")?;
        let fp = config::fingerprint(&Store::open(&self.dir, false)?.read_body(&e.id)?);
        let msg = format!("Asset {asset} now points at {}", e.id_hex);
        if self.idmap.add(asset, fp) {
            self.idmap.save(&self.idmap_path)?;
        }
        self.tag_input.clear();
        self.preview = None;
        Ok(msg)
    }

    fn top_bar(&mut self, ui: &mut egui::Ui, actions: &mut Vec<Action>) {
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            ui.label(format!("Cache: {}", self.dir.display()));
            if ui.button("Reload").clicked() {
                actions.push(Action::Reload);
            }
            if ui.button("Open folder").clicked() {
                actions.push(Action::OpenFolder);
            }
            if ui.button("Apply config...").on_hover_text("Apply a Fleasion-style replacement config to the cache").clicked() {
                actions.push(Action::ApplyConfig);
            }
            if self.roblox_running {
                ui.colored_label(egui::Color32::from_rgb(230, 160, 40), "Roblox is running - it may overwrite or ignore changes");
            }
        });
        ui.horizontal(|ui| {
            let mut changed = ui
                .add(egui::TextEdit::singleline(&mut self.search).hint_text("Search id, type or URL").desired_width(260.0))
                .changed();
            let kind_text = self.kind_filter.unwrap_or("All types");
            egui::ComboBox::from_id_salt("kind").selected_text(kind_text).height(400.0).show_ui(ui, |ui| {
                changed |= ui.selectable_value(&mut self.kind_filter, None, "All types").changed();
                for &(kind, n) in &self.kinds {
                    changed |= ui.selectable_value(&mut self.kind_filter, Some(kind), format!("{kind} ({n})")).changed();
                }
            });
            let category_text = self.category_filter.map_or("All categories".to_string(), |c| format!("{c} {}", category_name(c)));
            egui::ComboBox::from_id_salt("category").selected_text(category_text).show_ui(ui, |ui| {
                changed |= ui.selectable_value(&mut self.category_filter, None, "All categories").changed();
                for &(c, n) in &self.categories {
                    changed |= ui.selectable_value(&mut self.category_filter, Some(c), format!("{c} {} ({n})", category_name(c))).changed();
                }
            });
            changed |= ui.checkbox(&mut self.show_redirects, "Show redirects").changed();
            if changed {
                self.rebuild_view();
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.label(format!("{} of {} shown, {}", self.view.len(), self.entries.len(), human_size(self.view_bytes)));
            });
        });
        ui.add_space(4.0);
    }

    fn details(&mut self, ui: &mut egui::Ui, actions: &mut Vec<Action>) {
        // Keep the panel at its dragged width instead of shrinking to the content.
        ui.set_min_width(ui.available_width());
        if self.selected.len() > 1 {
            ui.heading(format!("{} entries selected", self.selected.len()));
            ui.label(format!("{} in total", human_size(self.selected_bytes())));
            ui.horizontal(|ui| {
                if ui.button("Export all to folder...").clicked() {
                    actions.push(Action::ExportMany);
                }
                if ui.button("Delete").clicked() {
                    actions.push(Action::AskDelete);
                }
            });
            ui.separator();
        }
        let Some(preview) = &self.preview else {
            if self.selected.is_empty() {
                ui.weak("Select an entry to see its details.\n\nCtrl-click and Shift-click select several.");
            }
            return;
        };
        if self.selected.len() <= 1 {
            ui.horizontal_wrapped(|ui| {
                if ui.button("Export...").clicked() {
                    actions.push(Action::Export);
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
                if ui.button("Delete").clicked() {
                    actions.push(Action::AskDelete);
                }
            });
            ui.horizontal(|ui| {
                ui.label("Asset ID");
                ui.add(egui::TextEdit::singleline(&mut self.tag_input).hint_text("e.g. 14147882149").desired_width(150.0));
                if ui.button("Tag").on_hover_text("Remember that this entry is that Roblox asset, so configs can refer to it by ID").clicked() {
                    actions.push(Action::Tag);
                }
            });
            ui.separator();
        }
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            if let Some(error) = &preview.error {
                ui.colored_label(ui.visuals().error_fg_color, error);
            }
            if let Some(texture) = &preview.texture {
                let size = texture.size_vec2();
                let scale = (ui.available_width() / size.x).min(380.0 / size.y).min(8.0);
                ui.add(egui::Image::new(egui::load::SizedTexture::new(texture.id(), size * scale)));
                ui.add_space(6.0);
            }
            let value_width = (ui.available_width() - 110.0).max(120.0);
            egui::Grid::new("facts").num_columns(2).striped(true).max_col_width(value_width).show(ui, |ui| {
                for (key, value) in &preview.facts {
                    ui.label(*key);
                    ui.add(egui::Label::new(value).wrap().selectable(true));
                    ui.end_row();
                }
            });
            if let Some(text) = &preview.text {
                ui.add_space(8.0);
                ui.strong("Text preview");
                ui.add(egui::TextEdit::multiline(&mut text.as_str()).font(egui::TextStyle::Monospace).desired_width(f32::INFINITY));
            }
            if !preview.hex.is_empty() {
                ui.add_space(8.0);
                ui.strong("First bytes of the payload");
                egui::ScrollArea::horizontal().id_salt("hex").show(ui, |ui| {
                    ui.add(egui::Label::new(egui::RichText::new(&preview.hex).monospace()).extend().selectable(true));
                });
            }
        });
    }

    fn table(&mut self, ui: &mut egui::Ui, actions: &mut Vec<Action>) {
        let mut clicked: Option<usize> = None;
        let mut sort_clicked: Option<SortKey> = None;
        let (sort, ascending) = (self.sort, self.ascending);
        let columns = [
            ("Type", SortKey::Kind),
            ("Category", SortKey::Category),
            ("Size", SortKey::Size),
            ("Hits", SortKey::Hits),
            ("Last used", SortKey::Used),
            ("ID", SortKey::Id),
        ];
        TableBuilder::new(ui)
            .striped(true)
            .resizable(true)
            .sense(egui::Sense::click())
            .cell_layout(egui::Layout::left_to_right(egui::Align::Center))
            .column(Column::initial(100.0).at_least(60.0))
            .column(Column::initial(110.0).at_least(60.0))
            .column(Column::initial(90.0).at_least(60.0))
            .column(Column::initial(60.0).at_least(40.0))
            .column(Column::initial(150.0).at_least(80.0))
            .column(Column::remainder().at_least(120.0))
            .min_scrolled_height(0.0)
            .header(24.0, |mut header| {
                for (title, key) in columns {
                    header.col(|ui| {
                        let arrow = if sort != key { "" } else if ascending { " ^" } else { " v" };
                        let label = egui::Label::new(egui::RichText::new(format!("{title}{arrow}")).strong()).sense(egui::Sense::click());
                        if ui.add(label).on_hover_text("Click to sort").clicked() {
                            sort_clicked = Some(key);
                        }
                    });
                }
            })
            .body(|body| {
                body.rows(20.0, self.view.len(), |mut row| {
                    let pos = row.index();
                    let e = &self.entries[self.view[pos]];
                    row.set_selected(self.selected.contains(&e.id));
                    row.col(|ui| {
                        ui.label(e.kind);
                    });
                    row.col(|ui| {
                        ui.label(format!("{} {}", e.category, category_name(e.category)));
                    });
                    row.col(|ui| {
                        ui.label(human_size(e.size));
                    });
                    row.col(|ui| {
                        ui.label(e.hits.to_string());
                    });
                    row.col(|ui| {
                        ui.label(format_time_ms(e.atime));
                    });
                    row.col(|ui| {
                        ui.label(egui::RichText::new(&e.id_hex).monospace());
                    });
                    let response = row.response();
                    if response.double_clicked() {
                        actions.push(Action::OpenExternally);
                    }
                    if response.clicked() {
                        clicked = Some(pos);
                    }
                });
            });
        if let Some(pos) = clicked {
            let modifiers = ui.input(|i| i.modifiers);
            self.click_row(pos, modifiers);
        }
        if let Some(key) = sort_clicked {
            if self.sort == key {
                self.ascending = !self.ascending;
            } else {
                self.sort = key;
                self.ascending = matches!(key, SortKey::Kind | SortKey::Category | SortKey::Id);
            }
            self.rebuild_view();
        }
    }

    fn delete_dialog(&mut self, ctx: &egui::Context, actions: &mut Vec<Action>) {
        if !self.confirm_delete {
            return;
        }
        egui::Window::new("Delete entries?")
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                ui.label(format!(
                    "Delete {} cache entr{} ({})?",
                    self.selected.len(),
                    if self.selected.len() == 1 { "y" } else { "ies" },
                    human_size(self.selected_bytes())
                ));
                ui.weak("Roblox downloads deleted assets again the next time it needs them.");
                if self.roblox_running {
                    ui.colored_label(egui::Color32::from_rgb(230, 160, 40), "Roblox is running and may still be using these.");
                }
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    if ui.button("Delete").clicked() {
                        actions.push(Action::Delete);
                        self.confirm_delete = false;
                    }
                    if ui.button("Cancel").clicked() || ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                        self.confirm_delete = false;
                    }
                });
            });
    }
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.poll_load();
        self.ensure_preview(ctx);
        let mut actions = Vec::new();

        egui::TopBottomPanel::top("top").show(ctx, |ui| self.top_bar(ui, &mut actions));
        egui::TopBottomPanel::bottom("status").show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.label(&self.status);
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if !self.selected.is_empty() {
                        ui.label(format!("{} selected", self.selected.len()));
                    }
                });
            });
        });
        egui::SidePanel::right("details")
            .resizable(true)
            .default_width(540.0)
            .width_range(320.0..=800.0)
            .show(ctx, |ui| self.details(ui, &mut actions));
        egui::CentralPanel::default().show(ctx, |ui| {
            if let Some(error) = &self.load_error {
                ui.colored_label(ui.visuals().error_fg_color, format!("Could not load the cache: {error}"));
            } else if self.loading.is_some() {
                ui.horizontal(|ui| {
                    ui.spinner();
                    let (done, total) = self.progress;
                    if total > 0 {
                        ui.label(format!("Scanning cache files... {done} / {total}"));
                    } else {
                        ui.label("Reading the cache index...");
                    }
                });
            } else {
                self.table(ui, &mut actions);
            }
        });
        self.delete_dialog(ctx, &mut actions);

        if ctx.input(|i| i.key_pressed(egui::Key::Delete)) && !ctx.wants_keyboard_input() {
            actions.push(Action::AskDelete);
        }
        for action in actions {
            self.run_action(action, ctx);
            // Actions run after this frame was laid out, so draw another to show the result.
            ctx.request_repaint();
        }
    }
}

fn build_preview(dir: &std::path::Path, entry: &Entry, idmap: &IdMap, ctx: &egui::Context) -> Preview {
    let mut preview = Preview { id: entry.id, facts: Vec::new(), texture: None, text: None, hex: String::new(), error: None };
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
    let tagged = idmap.assets_for(&config::fingerprint(body));
    if !tagged.is_empty() {
        preview.facts.insert(1, ("Asset ID", tagged.join(", ")));
    }
    if format::is_image(entry.kind) {
        match image::load_from_memory(body) {
            Ok(img) => {
                let img = if img.width().max(img.height()) > 2048 { img.thumbnail(2048, 2048) } else { img };
                let rgba = img.to_rgba8();
                let size = [rgba.width() as usize, rgba.height() as usize];
                let pixels = egui::ColorImage::from_rgba_unmultiplied(size, rgba.as_raw());
                preview.texture = Some(ctx.load_texture("preview", pixels, egui::TextureOptions::LINEAR));
            }
            Err(e) => preview.error = Some(format!("Could not decode the image: {e}")),
        }
    } else if format::is_text(entry.kind) {
        preview.text = Some(String::from_utf8_lossy(&body[..body.len().min(8000)]).into_owned());
    }
    preview.hex = format::hex_dump(body, 256);
    preview
}

