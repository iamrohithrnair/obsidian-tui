//! Application state and the actions that change it.
//!
//! Everything the user or the agent can do is an [`Action`], and every action
//! flows through [`App::dispatch`]. Keys, the command palette, mouse clicks and
//! agent tools all funnel into that one place, which is why a command works
//! identically however it was invoked — and why the agent gets the same
//! capabilities the user has, not a parallel set.

use std::path::PathBuf;

use ratatui::layout::Rect;

use otui_core::graph::{Graph, GraphOptions, Simulation, Vec2};
use otui_core::index::{NoteId, VaultIndex};
use otui_core::vault::{ScanOptions, Vault};
use otui_theme::{presets, ActiveTheme};

use crate::agent::Chat;
use crate::config::Config;
use crate::editor::Editor;
use crate::explorer::Explorer;

/// Which top-level surface is showing in the main pane.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum View {
    Notes,
    Graph,
}

/// Whether the open note is being read or edited. Obsidian's `Ctrl+E` toggle.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Reading,
    Editing,
}

/// Which pane takes keyboard input.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Focus {
    Explorer,
    Note,
    Sidebar,
    Chat,
    Graph,
}

/// The right sidebar's active tab.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SidePanel {
    Outline,
    Backlinks,
    Tags,
}

impl SidePanel {
    #[must_use]
    pub fn title(self) -> &'static str {
        match self {
            Self::Outline => "Outline",
            Self::Backlinks => "Backlinks",
            Self::Tags => "Tags",
        }
    }

    #[must_use]
    pub fn next(self) -> Self {
        match self {
            Self::Outline => Self::Backlinks,
            Self::Backlinks => Self::Tags,
            Self::Tags => Self::Outline,
        }
    }
}

/// An open note.
pub struct Tab {
    pub note: NoteId,
    pub mode: Mode,
    /// Scroll offset in reading mode.
    pub scroll: usize,
    /// Built lazily, the first time the tab is edited.
    pub editor: Option<Editor>,
}

impl Tab {
    #[must_use]
    pub fn is_modified(&self) -> bool {
        self.editor.as_ref().is_some_and(Editor::is_modified)
    }
}

/// A transient message in the status bar.
#[derive(Debug, Clone, Default)]
pub struct Status {
    pub text: String,
    pub is_error: bool,
}

/// The graph view's simulation and viewport.
pub struct GraphView {
    pub simulation: Simulation,
    /// Node under the cursor.
    pub selected: Option<usize>,
    /// Center of the viewport in graph space.
    pub center: Vec2,
    pub zoom: f32,
    /// When set, the graph shows only this note's neighborhood.
    pub local_root: Option<NoteId>,
    /// World-space extent the view frames at zoom 1.
    ///
    /// Held rather than measured each frame: the layout grows while it settles,
    /// so recomputing the fit from live bounds rescales the whole picture on
    /// every tick and the graph appears to breathe. It is refreshed when the
    /// layout stops moving and whenever the user asks to refit.
    pub span: f32,
    /// Node currently held by the mouse.
    pub dragging: Option<usize>,
}

/// Zoom limits. Below the lower bound the graph is a smudge; above the upper
/// one a single node fills the pane.
pub const MIN_ZOOM: f32 = 0.2;
pub const MAX_ZOOM: f32 = 20.0;

/// Padding around the laid-out graph when the view is fitted to it.
const FIT_PADDING: f32 = 1.15;

impl GraphView {
    /// Frames the whole layout.
    ///
    /// The origin is not where the nodes end up — the layout drifts as it
    /// settles — so resetting the view has to recentre on the graph's actual
    /// bounds rather than on `(0, 0)`.
    pub fn fit(&mut self) {
        let (min_x, min_y, max_x, max_y) = self.simulation.graph.bounds();
        self.center = Vec2::new((min_x + max_x) / 2.0, (min_y + max_y) / 2.0);
        self.zoom = 1.0;
        self.refit_span();
    }

    /// Recomputes the framed extent from the layout's current bounds.
    pub fn refit_span(&mut self) {
        let (min_x, min_y, max_x, max_y) = self.simulation.graph.bounds();
        self.span = (max_x - min_x).max(max_y - min_y).max(1.0) * FIT_PADDING;
    }

    /// Centres on a node and selects it.
    pub fn focus_node(&mut self, index: usize) {
        if let Some(node) = self.simulation.graph.nodes.get(index) {
            self.center = node.pos;
            self.selected = Some(index);
        }
    }

    /// Steps the selection through the graph, centring as it goes so the tour
    /// stays on screen at any zoom level.
    pub fn cycle_selection(&mut self, delta: isize) {
        let count = self.simulation.graph.nodes.len();
        if count == 0 {
            return;
        }
        let next = match self.selected {
            Some(current) => (current as isize + delta).rem_euclid(count as isize) as usize,
            // Starting from the best-connected node makes the first Tab land
            // somewhere worth looking at.
            None => (0..count)
                .max_by_key(|&i| self.simulation.graph.nodes[i].degree)
                .unwrap_or(0),
        };
        self.focus_node(next);
    }

    /// Moves the selection to the nearest node in a direction.
    ///
    /// Nearest *overall* is the wrong answer: pressing right should reach the
    /// note to the right even when a closer one sits just above, or the arrow
    /// keys stop feeling like movement through the picture. Candidates behind
    /// the cursor are rejected outright, and the rest are scored by distance
    /// plus how far off-axis they sit, so alignment beats a small head start.
    pub fn select_in_direction(&mut self, dx: f32, dy: f32) {
        let nodes = &self.simulation.graph.nodes;
        if nodes.is_empty() {
            return;
        }
        let Some(current) = self.selected.and_then(|i| nodes.get(i)).map(|n| n.pos) else {
            // Nothing selected yet, so an arrow key means "start somewhere
            // worth looking at" — the same entry point Tab uses.
            self.cycle_selection(0);
            return;
        };

        let mut best: Option<(usize, f32)> = None;
        for (index, node) in nodes.iter().enumerate() {
            if Some(index) == self.selected {
                continue;
            }
            let ox = node.pos.x - current.x;
            let oy = node.pos.y - current.y;
            if ox * dx + oy * dy <= 0.0 {
                continue;
            }
            let off_axis = (ox * dy - oy * dx).abs();
            let score = ox.hypot(oy) + off_axis * 2.0;
            if best.is_none_or(|(_, b)| score < b) {
                best = Some((index, score));
            }
        }

        if let Some((index, _)) = best {
            self.selected = Some(index);
        }
    }

    pub fn zoom_by(&mut self, factor: f32) {
        self.zoom = (self.zoom * factor).clamp(MIN_ZOOM, MAX_ZOOM);
    }
}

/// Everything the user can ask the app to do.
///
/// Kept as data rather than closures so the command palette can list them, the
/// key map can name them, and the agent can invoke them by name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    // Navigation
    OpenNote(NoteId),
    FollowLink(String),
    RevealInExplorer,
    Back,

    // Notes
    NewNote,
    InsertWikiLink,
    NewFolder,
    DailyNote,
    Save,
    SaveAll,
    RenameNote,
    DeleteNote,

    // Tabs
    CloseTab,
    NextTab,
    PreviousTab,

    // Views
    ToggleMode,
    ToggleLeftSidebar,
    ToggleRightSidebar,
    ToggleChat,
    ToggleRibbon,
    ToggleHints,
    /// Steps the explorer through the sort orders and remembers the choice.
    CycleSortOrder,
    CycleSidePanel,
    OpenGraph,
    OpenLocalGraph,
    OpenNotesView,

    // Modals
    OpenPalette,
    OpenSwitcher,
    OpenSearch,
    OpenThemePicker,
    OpenVaultPicker,
    OpenHelp,

    // Settings
    SetTheme(String),
    OpenVault(PathBuf),
    ToggleLineNumbers,
    ToggleGraphLabels,
    ToggleGraphUnresolved,
    ToggleGraphTags,
    ToggleGraphAttachments,
    ToggleGraphOrphans,

    // Vault
    /// Hand the open note to the Obsidian desktop app, via its CLI.
    OpenInObsidian,
    Refresh,
    SaveSettings,
    /// Asks for confirmation first; `ForceQuit` is what actually exits.
    Quit,
    ForceQuit,
}

/// Where each interactive element was drawn, recorded every frame.
///
/// The renderer knows the geometry; the mouse handler needs it. Rather than
/// duplicating the layout arithmetic in two places and letting them drift, the
/// draw pass writes down what it put where and clicks are resolved against
/// that.
#[derive(Debug, Clone, Default)]
pub struct Regions {
    /// Ribbon icons and the action each one runs.
    pub ribbon: Vec<(Rect, Action)>,
    /// The explorer's inner area, and the row index its first line shows.
    pub explorer: Option<(Rect, usize)>,
    /// Document tabs, with the index each one selects.
    pub tabs: Vec<(Rect, usize)>,
    /// The outline/backlinks/tags selector.
    pub side_tabs: Vec<(Rect, SidePanel)>,
    /// The sidebar's list area and the row index its first line shows.
    pub sidebar: Option<(Rect, usize)>,
    pub main: Option<Rect>,
    pub chat: Option<Rect>,
    /// The graph canvas with its coordinate bounds, for hit-testing nodes.
    pub graph: Option<(Rect, [f64; 2], [f64; 2])>,
}

pub struct App {
    pub index: VaultIndex,
    pub config: Config,
    pub theme: ActiveTheme,
    /// Every theme available in the picker, built-ins plus user files.
    pub themes: Vec<otui_theme::Theme>,

    pub explorer: Explorer,
    pub tabs: Vec<Tab>,
    pub active_tab: Option<usize>,
    /// Recently visited notes, newest last — powers `Back`.
    pub history: Vec<NoteId>,

    pub view: View,
    pub focus: Focus,
    pub side_panel: SidePanel,
    pub side_selected: usize,
    pub graph: Option<GraphView>,

    /// Hit regions from the last frame, for mouse input.
    pub regions: Regions,
    /// The overlay on top of everything, if any.
    pub modal: Option<crate::modal::Modal>,
    pub chat: Chat,
    pub status: Status,
    pub quit: bool,
}

impl App {
    /// Builds the app over a vault.
    pub fn new(vault: Vault, config: Config) -> Result<Self, std::io::Error> {
        let scan = ScanOptions {
            include_hidden: config.ui.show_hidden,
            ..Default::default()
        };
        let index = VaultIndex::build(vault, scan)?;

        let mut themes = presets::builtin();
        if let Some(dir) = Config::themes_dir() {
            let (custom, _errors) = presets::load_custom(&dir);
            themes.extend(custom);
        }
        let theme = themes
            .iter()
            .find(|t| t.name == config.theme)
            .cloned()
            .unwrap_or_else(presets::default_theme);

        let chat = Chat::new(&config.agent);
        let mut explorer = Explorer::default();
        explorer.set_sort(config.ui.sort_order());
        let mut app = Self {
            explorer,
            tabs: Vec::new(),
            active_tab: None,
            history: Vec::new(),
            view: View::Notes,
            focus: Focus::Explorer,
            side_panel: SidePanel::Outline,
            side_selected: 0,
            graph: None,
            regions: Regions::default(),
            modal: None,
            chat,
            status: Status::default(),
            quit: false,
            theme: ActiveTheme::new(theme),
            themes,
            config,
            index,
        };

        app.explorer.rebuild(&app.index);
        Ok(app)
    }

    // ---- accessors -------------------------------------------------------

    #[must_use]
    pub fn active(&self) -> Option<&Tab> {
        self.active_tab.and_then(|i| self.tabs.get(i))
    }

    #[must_use]
    pub fn active_mut(&mut self) -> Option<&mut Tab> {
        match self.active_tab {
            Some(index) => self.tabs.get_mut(index),
            None => None,
        }
    }

    #[must_use]
    pub fn active_note(&self) -> Option<NoteId> {
        self.active().map(|t| t.note)
    }

    #[must_use]
    pub fn note_title(&self, id: NoteId) -> String {
        self.index
            .note(id)
            .map(|n| n.meta.title.clone())
            .unwrap_or_else(|| "(missing)".into())
    }

    pub fn info(&mut self, message: impl Into<String>) {
        self.status = Status {
            text: message.into(),
            is_error: false,
        };
    }

    pub fn error(&mut self, message: impl Into<String>) {
        self.status = Status {
            text: message.into(),
            is_error: true,
        };
    }

    // ---- tabs ------------------------------------------------------------

    /// Opens a note, reusing an existing tab if it's already open.
    pub fn open_note(&mut self, id: NoteId) {
        if self.index.note(id).is_none() {
            self.error("that note no longer exists");
            return;
        }

        if let Some(current) = self.active_note() {
            if current != id {
                self.history.push(current);
                // A long session shouldn't accumulate unbounded history.
                if self.history.len() > 100 {
                    self.history.remove(0);
                }
            }
        }

        if let Some(existing) = self.tabs.iter().position(|t| t.note == id) {
            self.active_tab = Some(existing);
        } else {
            self.tabs.push(Tab {
                note: id,
                mode: if self.config.ui.reading_mode {
                    Mode::Reading
                } else {
                    Mode::Editing
                },
                scroll: 0,
                editor: None,
            });
            self.active_tab = Some(self.tabs.len() - 1);
        }

        self.view = View::Notes;
        self.focus = Focus::Note;
        self.side_selected = 0;
    }

    /// Opens a note by name or vault-relative path, creating it if missing.
    ///
    /// Following a link to a note that doesn't exist yet is how notes get
    /// written in Obsidian, so an unresolved target creates rather than errors.
    pub fn open_or_create(&mut self, target: &str) {
        if let Some(id) = self.index.resolve(target) {
            self.open_note(id);
            return;
        }
        // The heading is the note's name, not its path — `Daily/2026-07-27`
        // should open with `# 2026-07-27`.
        let title = target.rsplit('/').next().unwrap_or(target);
        match self.index.create_note(target, &format!("# {title}\n\n")) {
            Ok(id) => {
                self.explorer.rebuild(&self.index);
                self.open_note(id);
                self.info(format!("created {target}"));
            }
            Err(err) => self.error(format!("could not create {target}: {err}")),
        }
    }

    pub fn close_tab(&mut self) {
        let Some(index) = self.active_tab else {
            return;
        };
        if self.tabs[index].is_modified() && self.config.editor.auto_save {
            self.save_tab(index);
        }
        self.tabs.remove(index);
        self.active_tab = if self.tabs.is_empty() {
            None
        } else {
            Some(index.min(self.tabs.len() - 1))
        };
        if self.tabs.is_empty() {
            self.focus = Focus::Explorer;
        }
    }

    pub fn cycle_tab(&mut self, delta: isize) {
        if self.tabs.is_empty() {
            return;
        }
        let count = self.tabs.len() as isize;
        let current = self.active_tab.unwrap_or(0) as isize;
        let next = (current + delta).rem_euclid(count);
        self.active_tab = Some(next as usize);
        self.side_selected = 0;
    }

    /// The editor for the active tab, created on first use.
    pub fn editor_mut(&mut self) -> Option<&mut Editor> {
        let index = self.active_tab?;
        let note = self.tabs[index].note;
        if self.tabs[index].editor.is_none() {
            let text = self.index.read(note).unwrap_or_default();
            self.tabs[index].editor = Some(Editor::new(
                &text,
                self.config.editor.tab_width,
                self.config.editor.expand_tabs,
            ));
        }
        self.tabs[index].editor.as_mut()
    }

    pub fn save_tab(&mut self, index: usize) {
        let Some(tab) = self.tabs.get(index) else {
            return;
        };
        let note = tab.note;
        let Some(text) = tab.editor.as_ref().map(Editor::text) else {
            return;
        };
        match self.index.write_note(note, &text) {
            Ok(()) => {
                if let Some(editor) = self.tabs[index].editor.as_mut() {
                    editor.mark_saved();
                }
                let title = self.note_title(note);
                self.info(format!("saved {title}"));
                self.explorer.rebuild(&self.index);
            }
            Err(err) => self.error(format!("save failed: {err}")),
        }
    }

    // ---- graph -----------------------------------------------------------

    fn graph_options(&self) -> GraphOptions {
        GraphOptions {
            show_unresolved: self.config.graph.show_unresolved,
            show_tags: self.config.graph.show_tags,
            show_attachments: self.config.graph.show_attachments,
            show_orphans: self.config.graph.show_orphans,
            tag_filter: None,
        }
    }

    /// Opens the graph, optionally restricted to one note's neighborhood.
    pub fn open_graph(&mut self, local_root: Option<NoteId>) {
        let mut graph = Graph::build(&self.index, &self.graph_options());

        // The local graph is cut down to a graph of its own before anything is
        // laid out, so the neighbourhood settles into the pane it will be shown
        // in. Filtering at draw time instead leaves it laid out for a picture
        // it never gets, and drawn with edges running off to nodes that aren't
        // there.
        if let Some(root) = local_root.and_then(|id| graph.node_of_note(id)) {
            let neighborhood = graph.neighborhood(root, self.config.graph.local_depth);
            graph = graph.subgraph(&neighborhood);
        }

        let mut simulation = Simulation::new(graph);
        // Settle before the first frame so the graph opens readable rather
        // than as an exploding cloud.
        simulation.run(600);

        let selected = local_root.and_then(|id| simulation.graph.node_of_note(id));
        // Centre on the focused note if there is one, else on the middle of the
        // laid-out graph — the origin is not where the nodes end up.
        let center = selected
            .and_then(|n| simulation.graph.nodes.get(n))
            .map(|n| n.pos)
            .unwrap_or_else(|| {
                let (min_x, min_y, max_x, max_y) = simulation.graph.bounds();
                Vec2::new((min_x + max_x) / 2.0, (min_y + max_y) / 2.0)
            });

        let mut view = GraphView {
            center,
            simulation,
            selected,
            zoom: 1.0,
            local_root,
            span: 1.0,
            dragging: None,
        };
        view.refit_span();

        self.graph = Some(view);
        self.view = View::Graph;
        self.focus = Focus::Graph;
    }

    /// Rebuilds the graph in place, keeping the viewport.
    pub fn refresh_graph(&mut self) {
        if self.graph.is_none() {
            return;
        }
        let local_root = self.graph.as_ref().and_then(|g| g.local_root);
        let (center, zoom) = self
            .graph
            .as_ref()
            .map_or((Vec2::default(), 1.0), |g| (g.center, g.zoom));
        self.open_graph(local_root);
        if let Some(graph) = self.graph.as_mut() {
            graph.center = center;
            graph.zoom = zoom;
        }
    }

    // ---- vault -----------------------------------------------------------

    /// Re-reads the whole vault from disk.
    pub fn refresh(&mut self) {
        // Note ids are positional, so anything holding one must be re-resolved
        // against the rebuilt index or it will point at the wrong note.
        let open: Vec<String> = self
            .tabs
            .iter()
            .filter_map(|t| self.index.note(t.note).map(|n| n.meta.rel.clone()))
            .collect();
        let active_rel = self
            .active_note()
            .and_then(|id| self.index.note(id).map(|n| n.meta.rel.clone()));

        if let Err(err) = self.index.rebuild() {
            self.error(format!("refresh failed: {err}"));
            return;
        }

        self.tabs.retain(|_| true);
        let mut rebuilt = Vec::new();
        for (rel, tab) in open.iter().zip(std::mem::take(&mut self.tabs)) {
            if let Some(id) = self.index.id_of_rel(rel) {
                rebuilt.push(Tab { note: id, ..tab });
            }
        }
        self.tabs = rebuilt;
        self.active_tab = active_rel
            .and_then(|rel| self.index.id_of_rel(&rel))
            .and_then(|id| self.tabs.iter().position(|t| t.note == id))
            .or(if self.tabs.is_empty() { None } else { Some(0) });

        self.history.clear();
        self.explorer.rebuild(&self.index);
        if self.graph.is_some() {
            self.refresh_graph();
        }
    }

    /// Switches to another vault, keeping settings and theme.
    pub fn open_vault(&mut self, path: PathBuf) {
        let scan = ScanOptions {
            include_hidden: self.config.ui.show_hidden,
            ..Default::default()
        };
        match VaultIndex::build(Vault::from_path(&path), scan) {
            Ok(index) => {
                self.index = index;
                self.tabs.clear();
                self.active_tab = None;
                self.history.clear();
                self.graph = None;
                self.view = View::Notes;
                self.focus = Focus::Explorer;
                // Keep the chosen order across a vault switch; it's a
                // preference about the app, not about the vault.
                let sort = self.explorer.sort();
                self.explorer = Explorer::default();
                self.explorer.set_sort(sort);
                self.explorer.rebuild(&self.index);
                let name = self.index.vault.name.clone();
                self.info(format!("opened vault {name}"));
            }
            Err(err) => self.error(format!("could not open vault: {err}")),
        }
    }

    pub fn set_theme(&mut self, name: &str) {
        if let Some(theme) = self.themes.iter().find(|t| t.name == name).cloned() {
            self.theme = ActiveTheme::new(theme);
            self.config.theme = name.to_string();
            self.info(format!("theme: {name}"));
        } else {
            self.error(format!("no theme named {name}"));
        }
    }

    /// Today's daily note, created if needed.
    pub fn daily_note_name(&self) -> String {
        let name = format_date(&self.config.editor.daily_format);
        let folder = self.config.editor.daily_folder.trim_matches('/');
        if folder.is_empty() {
            name
        } else {
            format!("{folder}/{name}")
        }
    }
}

/// Formats today's date, supporting the `%Y`/`%m`/`%d` placeholders that cover
/// every daily-note format in practice.
///
/// Computed from the system clock with a plain civil-date conversion rather
/// than pulling in a date library for three substitutions.
fn format_date(format: &str) -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let (year, month, day) = civil_from_days((secs / 86_400) as i64);
    format
        .replace("%Y", &format!("{year:04}"))
        .replace("%m", &format!("{month:02}"))
        .replace("%d", &format!("{day:02}"))
}

/// Days since the Unix epoch to a civil date (Howard Hinnant's algorithm).
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

#[cfg(test)]
mod tests {
    use super::*;
    use otui_core::test_support::TempVault;

    fn app(vault: &TempVault) -> App {
        App::new(vault.vault(), Config::default()).expect("build app")
    }

    fn sample() -> TempVault {
        let vault = TempVault::new("app");
        vault.write("A.md", "# A\n\nlinks to [[B]]\n");
        vault.write("B.md", "# B\n");
        vault.write("Folder/C.md", "# C\n");
        vault
    }

    #[test]
    fn the_local_graph_holds_only_the_neighborhood() {
        let vault = TempVault::new("local-graph");
        vault.write("Hub.md", "[[Near]]\n");
        vault.write("Near.md", "[[Far]]\n");
        vault.write("Far.md", "end\n");
        vault.write("Unrelated.md", "nothing\n");
        let mut app = app(&vault);

        let hub = app.index.id_of_rel("Hub.md").unwrap();
        app.open_graph(Some(hub));

        let graph = app.graph.as_ref().expect("graph");
        let labels: Vec<&str> = graph
            .simulation
            .graph
            .nodes
            .iter()
            .map(|n| n.label.as_str())
            .collect();
        assert_eq!(
            labels,
            vec!["Hub", "Near"],
            "depth 1 from Hub, and nothing else in the vault"
        );
        assert_eq!(
            graph.selected,
            Some(0),
            "the note the graph was opened for is selected"
        );
    }

    #[test]
    fn the_local_graph_is_framed_on_its_own_extent() {
        // The bug this guards: fitting the view to the whole vault's bounds
        // while drawing only a neighbourhood, which leaves the local graph a
        // speck in the corner of an empty pane.
        let vault = TempVault::new("local-frame");
        let mut wide = String::new();
        for i in 0..40 {
            wide.push_str(&format!("[[N{i}]]\n"));
            vault.write(&format!("N{i}.md"), "spread out\n");
        }
        vault.write("Wide.md", &wide);
        vault.write("Pair.md", "[[Mate]]\n");
        vault.write("Mate.md", "back\n");
        let mut app = app(&vault);

        app.open_graph(None);
        let whole = app.graph.as_ref().expect("graph").span;

        let pair = app.index.id_of_rel("Pair.md").unwrap();
        app.open_graph(Some(pair));
        let local = app.graph.as_ref().expect("graph").span;

        assert!(
            local < whole,
            "two linked notes must frame tighter than a 40-spoke vault: {local} vs {whole}"
        );
    }

    #[test]
    fn framing_holds_still_while_the_layout_settles() {
        let vault = sample();
        let mut app = app(&vault);
        app.open_graph(None);

        let graph = app.graph.as_mut().expect("graph");
        let before = graph.span;
        graph.simulation.reheat();
        graph.simulation.step();
        assert_eq!(
            graph.span, before,
            "a step must not rescale the view, or the graph appears to breathe"
        );
    }

    #[test]
    fn arrows_walk_to_the_node_they_point_at() {
        let vault = sample();
        let mut app = app(&vault);
        app.open_graph(None);
        let graph = app.graph.as_mut().expect("graph");

        // A closer node off to the side must lose to an aligned one further
        // away, or arrow keys stop tracking the direction pressed.
        graph.simulation.drag(0, Vec2::new(0.0, 0.0));
        graph.simulation.drag(1, Vec2::new(40.0, 0.0));
        graph.simulation.drag(2, Vec2::new(4.0, 30.0));
        graph.selected = Some(0);

        graph.select_in_direction(1.0, 0.0);
        assert_eq!(
            graph.selected,
            Some(1),
            "right reaches the node to the right"
        );

        graph.selected = Some(0);
        graph.select_in_direction(0.0, 1.0);
        assert_eq!(graph.selected, Some(2), "up reaches the node above");

        // Nothing lies left of the origin node, so the selection stays put
        // rather than wrapping to the far side of the graph.
        graph.selected = Some(0);
        graph.select_in_direction(-1.0, 0.0);
        assert_eq!(graph.selected, Some(0));
    }

    #[test]
    fn an_arrow_with_nothing_selected_starts_at_a_hub() {
        let vault = sample();
        let mut app = app(&vault);
        app.open_graph(None);
        let graph = app.graph.as_mut().expect("graph");
        graph.selected = None;

        graph.select_in_direction(1.0, 0.0);
        assert!(
            graph.selected.is_some(),
            "an arrow key always lands somewhere"
        );
    }

    #[test]
    fn opening_a_note_creates_one_tab_and_reuses_it() {
        let vault = sample();
        let mut app = app(&vault);
        let a = app.index.id_of_rel("A.md").unwrap();

        app.open_note(a);
        assert_eq!(app.tabs.len(), 1);
        assert_eq!(app.active_note(), Some(a));
        assert_eq!(app.focus, Focus::Note);

        app.open_note(a);
        assert_eq!(app.tabs.len(), 1, "reopening focuses the existing tab");
    }

    #[test]
    fn following_a_link_to_a_missing_note_creates_it() {
        let vault = sample();
        let mut app = app(&vault);

        app.open_or_create("Brand New");
        assert!(vault.exists("Brand New.md"));
        assert_eq!(
            app.note_title(app.active_note().unwrap()),
            "Brand New",
            "the new note is opened, not just created"
        );
    }

    #[test]
    fn a_note_created_in_a_folder_gets_a_bare_heading() {
        let vault = sample();
        let mut app = app(&vault);

        app.open_or_create("Daily/2026-07-27");
        assert_eq!(
            vault.read("Daily/2026-07-27.md"),
            "# 2026-07-27\n\n",
            "the heading is the note name, not its path"
        );
    }

    #[test]
    fn following_a_link_to_an_existing_note_opens_it() {
        let vault = sample();
        let mut app = app(&vault);
        let b = app.index.id_of_rel("B.md").unwrap();

        app.open_or_create("B");
        assert_eq!(app.active_note(), Some(b));
        assert_eq!(app.tabs.len(), 1, "nothing was created");
    }

    #[test]
    fn editing_and_saving_writes_through_to_disk() {
        let vault = sample();
        let mut app = app(&vault);
        let b = app.index.id_of_rel("B.md").unwrap();
        app.open_note(b);

        let editor = app.editor_mut().expect("editor");
        editor.move_document_end(false);
        editor.insert_str("appended");
        assert!(app.active().unwrap().is_modified());

        app.save_tab(app.active_tab.unwrap());
        assert!(vault.read("B.md").contains("appended"));
        assert!(!app.active().unwrap().is_modified());
    }

    #[test]
    fn saving_reindexes_new_links() {
        let vault = sample();
        let mut app = app(&vault);
        let b = app.index.id_of_rel("B.md").unwrap();
        let a = app.index.id_of_rel("A.md").unwrap();
        app.open_note(b);

        app.editor_mut().unwrap().insert_str("[[A]]\n");
        app.save_tab(app.active_tab.unwrap());

        assert!(
            app.index.backlinks(a).iter().any(|bl| bl.source == b),
            "the new link must be in the index immediately"
        );
    }

    #[test]
    fn closing_a_tab_selects_a_neighbor() {
        let vault = sample();
        let mut app = app(&vault);
        let a = app.index.id_of_rel("A.md").unwrap();
        let b = app.index.id_of_rel("B.md").unwrap();

        app.open_note(a);
        app.open_note(b);
        app.close_tab();

        assert_eq!(app.tabs.len(), 1);
        assert_eq!(app.active_note(), Some(a));
    }

    #[test]
    fn closing_the_last_tab_returns_focus_to_the_explorer() {
        let vault = sample();
        let mut app = app(&vault);
        let a = app.index.id_of_rel("A.md").unwrap();

        app.open_note(a);
        app.close_tab();

        assert!(app.tabs.is_empty());
        assert_eq!(app.active_tab, None);
        assert_eq!(app.focus, Focus::Explorer);
    }

    #[test]
    fn tab_cycling_wraps_in_both_directions() {
        let vault = sample();
        let mut app = app(&vault);
        for rel in ["A.md", "B.md", "Folder/C.md"] {
            let id = app.index.id_of_rel(rel).unwrap();
            app.open_note(id);
        }

        app.cycle_tab(1);
        assert_eq!(app.active_tab, Some(0), "wraps past the end");
        app.cycle_tab(-1);
        assert_eq!(app.active_tab, Some(2), "wraps past the start");
    }

    #[test]
    fn refresh_keeps_open_tabs_pointing_at_the_right_notes() {
        let vault = sample();
        let mut app = app(&vault);
        let c = app.index.id_of_rel("Folder/C.md").unwrap();
        app.open_note(c);

        // Adding a note earlier in sort order shifts every id.
        vault.write("AAA.md", "first\n");
        app.refresh();

        let title = app.note_title(app.active_note().expect("still open"));
        assert_eq!(title, "C", "the tab must follow the note, not the id");
    }

    #[test]
    fn refresh_drops_tabs_for_deleted_notes() {
        let vault = sample();
        let mut app = app(&vault);
        let b = app.index.id_of_rel("B.md").unwrap();
        app.open_note(b);

        std::fs::remove_file(vault.path().join("B.md")).expect("delete");
        app.refresh();

        assert!(app.tabs.is_empty(), "a tab for a deleted note is closed");
    }

    #[test]
    fn opening_the_graph_places_the_focused_note() {
        let vault = sample();
        let mut app = app(&vault);
        let a = app.index.id_of_rel("A.md").unwrap();

        app.open_graph(Some(a));
        let graph = app.graph.as_ref().expect("graph built");

        assert_eq!(app.view, View::Graph);
        assert!(
            graph.selected.is_some(),
            "the note is selected in the graph"
        );
        assert!(graph.simulation.is_settled(), "opens already laid out");
    }

    #[test]
    fn theme_switching_updates_the_palette_and_config() {
        let vault = sample();
        let mut app = app(&vault);

        app.set_theme("nord");
        assert_eq!(app.theme.name(), "nord");
        assert_eq!(app.config.theme, "nord");

        app.set_theme("not-a-theme");
        assert_eq!(
            app.theme.name(),
            "nord",
            "a bad name leaves the theme alone"
        );
        assert!(app.status.is_error);
    }

    #[test]
    fn history_records_where_you_came_from() {
        let vault = sample();
        let mut app = app(&vault);
        let a = app.index.id_of_rel("A.md").unwrap();
        let b = app.index.id_of_rel("B.md").unwrap();

        app.open_note(a);
        app.open_note(b);
        assert_eq!(app.history, vec![a]);

        app.open_note(b);
        assert_eq!(app.history, vec![a], "reopening the same note adds nothing");
    }

    #[test]
    fn daily_note_name_uses_the_configured_folder_and_format() {
        let vault = sample();
        let app = app(&vault);
        let name = app.daily_note_name();

        assert!(name.starts_with("Daily/"));
        // `Daily/YYYY-MM-DD`
        assert_eq!(name.len(), "Daily/".len() + 10, "got {name}");
    }

    #[test]
    fn civil_dates_match_known_values() {
        assert_eq!(civil_from_days(0), (1970, 1, 1));
        assert_eq!(civil_from_days(19_723), (2024, 1, 1));
        // A leap day, which is where naive date math usually breaks.
        assert_eq!(civil_from_days(19_782), (2024, 2, 29));
    }
}
