//! Application state and the effects commands have on it.
//!
//! Painting lives in the [`ui`] submodule, which can reach this module's
//! private state because it is a child of it.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use egui::{Key, Modifiers};

use crate::config::{Config, TreeSide};
use crate::document::Document;
use crate::index::{FileEntry, Hit, Message, Search};
use crate::layout::{Axis, Dir, PaneId, Panes};
use crate::palette::{Command, Mode, Palette};
use crate::theme::{self, Theme};
use crate::tree::FileTree;
use crate::watcher::VaultWatcher;

mod ui;

/// Filesystem events are coalesced for this long. Editors write a note in
/// several syscalls; reloading on the first one shows a half-written file.
const FS_DEBOUNCE: Duration = Duration::from_millis(60);
/// Keystrokes in the content search are coalesced for this long.
const QUERY_DEBOUNCE: Duration = Duration::from_millis(140);
/// How long `ctrl+w` waits for the rest of its chord before giving up. Vim
/// waits forever; a preview window that swallows the next key indefinitely
/// would be a trap, so this matches a comfortable `timeoutlen`.
const CHORD_TIMEOUT: Duration = Duration::from_millis(1200);
/// `ctrl+h/j/k/l`, the vim motions, and where each one goes.
const MOTION_KEYS: [Key; 4] = [Key::H, Key::J, Key::K, Key::L];
const MOTIONS: [Dir; 4] = [Dir::Left, Dir::Down, Dir::Up, Dir::Right];

/// `ctrl+1` .. `ctrl+9`, in tab order.
const NUMBER_KEYS: [Key; 9] = [
    Key::Num1,
    Key::Num2,
    Key::Num3,
    Key::Num4,
    Key::Num5,
    Key::Num6,
    Key::Num7,
    Key::Num8,
    Key::Num9,
];

/// Width of the invisible window-edge resize handles.
const RESIZE_GRIP: f32 = 6.0;
/// Settings are written this long after the last change, so that dragging a
/// splitter does not write the file on every frame.
const SAVE_DEBOUNCE: Duration = Duration::from_secs(2);

struct Vault {
    root: PathBuf,
    tree: FileTree,
    /// `None` if the watch could not be installed; the preview then only
    /// refreshes on an explicit reload.
    watcher: Option<VaultWatcher>,
}

pub struct App {
    cfg: Config,
    theme: Theme,
    vault: Option<Vault>,
    panes: Panes,
    palette: Palette,

    search: Search,
    files: Vec<FileEntry>,
    files_generation: u64,
    indexing: bool,
    hits: Vec<Hit>,
    hits_generation: u64,
    searching: bool,

    /// Paths reported by the watcher, waiting out `FS_DEBOUNCE`.
    pending_paths: Vec<PathBuf>,
    pending_since: Option<Instant>,
    /// Content query waiting out `QUERY_DEBOUNCE`.
    pending_query: Option<(String, Instant)>,
    /// Content query most recently handed to the worker.
    content_query: String,
    /// Inputs the palette's current row list was built from.
    ranked: Option<RankKey>,

    /// Settings as last written to disk, to detect real changes.
    saved: Config,
    dirty_since: Option<Instant>,

    /// When `ctrl+w` was pressed and is waiting for the key that completes it.
    chord: Option<Instant>,
    /// The tab currently riding the cursor, if any.
    drag: Option<TabDrag>,

    notice: Option<String>,
    quit: bool,
}

/// A tab in flight between (or within) tab strips.
#[derive(Clone)]
struct TabDrag {
    pane: PaneId,
    index: usize,
    /// Kept here so the ghost under the cursor survives the tab being moved.
    title: String,
}

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>, cfg: Config, target: Option<PathBuf>) -> Self {
        let missing = theme::install_fonts(&cc.egui_ctx);
        let notice = (!missing.is_empty()).then(|| {
            format!(
                "JetBrainsMono Nerd Font {} not found — falling back",
                missing.join(", ")
            )
        });

        let theme = Theme::new(&cfg);
        theme.apply(&cc.egui_ctx);

        let mut app = Self {
            saved: cfg.clone(),
            dirty_since: None,
            cfg,
            theme,
            vault: None,
            panes: Panes::new(),
            palette: Palette::new(),
            search: Search::new(cc.egui_ctx.clone()),
            files: Vec::new(),
            files_generation: 0,
            indexing: false,
            hits: Vec::new(),
            hits_generation: 0,
            searching: false,
            pending_paths: Vec::new(),
            pending_since: None,
            pending_query: None,
            content_query: String::new(),
            ranked: None,
            chord: None,
            drag: None,
            notice,
            quit: false,
        };

        // An explicit argument wins over the remembered vault, except that a
        // note inside the remembered vault keeps that vault open.
        let remembered = app.cfg.vault.clone();
        match target.map(|path| resolve_target(path, remembered.as_deref())) {
            Some(Ok(Target::Vault(root))) => app.open_vault(root, &cc.egui_ctx),
            Some(Ok(Target::Note { vault, note })) => {
                app.open_vault(vault, &cc.egui_ctx);
                app.open_file(note);
            }
            Some(Err(message)) => app.notice = Some(message),
            None => {
                if let Some(root) = app.cfg.vault.clone().filter(|root| root.is_dir()) {
                    app.open_vault(root, &cc.egui_ctx);
                }
            }
        }
        app
    }

    // ---------------------------------------------------------------- vault

    fn open_vault(&mut self, root: PathBuf, ctx: &egui::Context) {
        let root = root.canonicalize().unwrap_or(root);
        self.panes = Panes::new();
        self.files.clear();
        self.hits.clear();

        let watcher = match VaultWatcher::new(&root, ctx.clone()) {
            Ok(watcher) => Some(watcher),
            Err(err) => {
                self.notice = Some(format!("not watching {}: {err}", root.display()));
                None
            }
        };
        self.files_generation = self.search.index(&root);
        self.indexing = true;

        self.cfg.vault = Some(root.clone());
        self.vault = Some(Vault {
            tree: FileTree::new(root.clone()),
            root,
            watcher,
        });
        ctx.send_viewport_cmd(egui::ViewportCommand::Title(self.window_title()));
    }

    fn window_title(&self) -> String {
        match &self.vault {
            Some(vault) => format!("{} — Faust", name_of(&vault.root)),
            None => "Faust".to_owned(),
        }
    }

    fn open_file(&mut self, path: PathBuf) {
        if path.is_dir() {
            if let Some(vault) = &mut self.vault {
                vault.tree.reveal(&path);
                vault.tree.toggle(&path);
            }
            return;
        }
        // Files always open in the focused pane, which is what makes the tree
        // and the palette useful once the window is split.
        let pane = self.panes.active_mut();
        if let Some(index) = pane.tabs.iter().position(|tab| tab.path == path) {
            pane.active = index;
            return;
        }
        pane.tabs.push(Document::open(path));
        pane.active = pane.tabs.len() - 1;
    }

    /// Resolves an Obsidian `[[wikilink]]` against the index.
    fn open_note(&mut self, name: &str) {
        let target = name.split('#').next().unwrap_or(name).trim();
        if target.is_empty() {
            return;
        }
        let wanted = target.to_lowercase();
        let hit = self
            .files
            .iter()
            .filter(|entry| !entry.is_dir)
            .find(|entry| {
                let stem = entry
                    .path
                    .file_stem()
                    .map(|s| s.to_string_lossy().to_lowercase());
                stem.as_deref() == Some(wanted.as_str())
                    || entry.display.to_lowercase() == wanted
                    || entry.display.to_lowercase() == format!("{wanted}.md")
            })
            .map(|entry| entry.path.clone());

        match hit {
            Some(path) => self.open_file(path),
            None => self.notice = Some(format!("no note named “{target}”")),
        }
    }

    fn active_path(&self) -> Option<&Path> {
        self.panes.active().doc().map(|tab| tab.path.as_path())
    }

    fn close_tab(&mut self) {
        let pane = self.panes.active_mut();
        pane.take(pane.active);
    }

    fn cycle_tab(&mut self, forward: bool) {
        let pane = self.panes.active_mut();
        let len = pane.tabs.len();
        if len < 2 {
            return;
        }
        pane.active = if forward {
            (pane.active + 1) % len
        } else {
            (pane.active + len - 1) % len
        };
    }

    // -------------------------------------------------------------- effects

    fn run(&mut self, command: Command, ctx: &egui::Context) {
        self.cfg.touch_command(command.id());
        match command {
            Command::OpenVault => {
                self.palette.show(Mode::Vault);
                self.palette.browse = self
                    .vault
                    .as_ref()
                    .and_then(|vault| vault.root.parent().map(Path::to_path_buf))
                    .or_else(|| std::env::var_os("HOME").map(PathBuf::from))
                    .unwrap_or_else(|| PathBuf::from("/"));
                self.palette.rank_folders();
            }
            Command::FindFile => self.palette.show(Mode::File),
            Command::SearchContent => {
                self.palette.show(Mode::Content);
                self.hits.clear();
                self.content_query.clear();
            }
            Command::ReloadFile => {
                if let Some(tab) = self.panes.active_mut().doc_mut() {
                    tab.reload();
                }
            }
            Command::RevealInTree => {
                if let (Some(path), Some(vault)) = (
                    self.panes.active().doc().map(|tab| tab.path.clone()),
                    self.vault.as_mut(),
                ) {
                    vault.tree.reveal(&path);
                    self.cfg.tree_visible = true;
                }
            }
            Command::CloseTab => self.close_tab(),
            Command::CloseOthers => {
                let pane = self.panes.active_mut();
                let at = pane.active;
                if at < pane.tabs.len() {
                    let keep = pane.tabs.remove(at);
                    pane.tabs.clear();
                    pane.tabs.push(keep);
                    pane.active = 0;
                }
            }
            Command::CloseAll => {
                // Every pane, but the splits themselves stay: `Close Other
                // Splits` is the command for undoing a layout.
                for pane in self.panes.iter_mut() {
                    pane.tabs.clear();
                    pane.active = 0;
                }
            }
            Command::NextTab => self.cycle_tab(true),
            Command::PrevTab => self.cycle_tab(false),
            Command::SplitRight => {
                self.panes.split(Axis::Row);
            }
            Command::SplitDown => {
                self.panes.split(Axis::Column);
            }
            Command::NextPane => self.panes.cycle(true),
            Command::ClosePane => self.panes.close(),
            Command::OnlyPane => self.panes.only(),
            Command::EqualizePanes => self.panes.equalize(),
            Command::ToggleTree => self.cfg.tree_visible = !self.cfg.tree_visible,
            Command::TreeLeft => self.dock(TreeSide::Left),
            Command::TreeRight => self.dock(TreeSide::Right),
            Command::TreeBottom => self.dock(TreeSide::Bottom),
            Command::ToggleTheme => {
                self.cfg.theme = self.cfg.theme.toggled();
                self.restyle(ctx);
            }
            Command::IncreaseFont => {
                self.cfg.font_size = (self.cfg.font_size + 1.0).min(32.0);
                self.restyle(ctx);
            }
            Command::DecreaseFont => {
                self.cfg.font_size = (self.cfg.font_size - 1.0).max(8.0);
                self.restyle(ctx);
            }
            Command::ToggleDecorations => {
                self.cfg.decorations = !self.cfg.decorations;
                ctx.send_viewport_cmd(egui::ViewportCommand::Decorations(self.cfg.decorations));
            }
            Command::CopyPath => {
                if let Some(path) = self.active_path() {
                    ctx.copy_text(path.display().to_string());
                    self.notice = Some("path copied".to_owned());
                }
            }
            Command::OpenExternally => {
                if let Some(path) = self.active_path() {
                    open_externally(path);
                }
            }
            Command::Quit => self.quit = true,
        }
    }

    fn dock(&mut self, side: TreeSide) {
        // Width and height are not interchangeable, so reset the split when the
        // tree moves between a side and the bottom.
        let was_vertical = self.cfg.tree_side == TreeSide::Bottom;
        let is_vertical = side == TreeSide::Bottom;
        if was_vertical != is_vertical {
            self.cfg.tree_size = if is_vertical { 220.0 } else { 260.0 };
        }
        self.cfg.tree_side = side;
        self.cfg.tree_visible = true;
    }

    fn restyle(&mut self, ctx: &egui::Context) {
        self.theme = Theme::new(&self.cfg);
        self.theme.apply(ctx);
    }

    // --------------------------------------------------------------- inputs

    fn shortcuts(&mut self, ctx: &egui::Context) {
        if self.chord(ctx) {
            return;
        }
        let palette_open = self.palette.open;
        // Most specific first: `consume_key` ignores extra Shift and Alt.
        let hits = ctx.input_mut(|input| {
            let cmd = Modifiers::COMMAND;
            let cmd_shift = Modifiers::COMMAND.plus(Modifiers::SHIFT);
            Shortcuts {
                palette: input.consume_key(Modifiers::ALT, Key::Space)
                    || input.consume_key(cmd_shift, Key::P),
                prev_tab: input.consume_key(cmd_shift, Key::Tab),
                content: input.consume_key(cmd_shift, Key::F),
                next_tab: input.consume_key(cmd, Key::Tab),
                find_file: input.consume_key(cmd, Key::P),
                close_tab: input.consume_key(cmd_shift, Key::W),
                window: input.consume_key(cmd, Key::W),
                toggle_tree: input.consume_key(cmd, Key::B),
                reload: input.consume_key(cmd, Key::R),
                quit: input.consume_key(cmd, Key::Q),
                bigger: input.consume_key(cmd, Key::Plus) || input.consume_key(cmd, Key::Equals),
                smaller: input.consume_key(cmd, Key::Minus),
                escape: !palette_open && input.consume_key(Modifiers::NONE, Key::Escape),
                tab_index: NUMBER_KEYS
                    .iter()
                    .position(|key| input.consume_key(cmd, *key)),
                motion: MOTION_KEYS
                    .iter()
                    .position(|key| input.consume_key(cmd, *key)),
            }
        });

        if hits.palette {
            if self.palette.open && self.palette.mode == Mode::Command {
                self.palette.close();
            } else {
                self.palette.show(Mode::Command);
            }
        }
        if hits.find_file {
            self.run(Command::FindFile, ctx);
        }
        if hits.content {
            self.run(Command::SearchContent, ctx);
        }
        if hits.next_tab {
            self.cycle_tab(true);
        }
        if hits.prev_tab {
            self.cycle_tab(false);
        }
        if hits.close_tab {
            self.close_tab();
        }
        if let Some(index) = hits.motion {
            self.panes.focus_dir(MOTIONS[index]);
        }
        // The palette owns the keyboard while it is open, `ctrl+w` included.
        if hits.window && !palette_open {
            self.chord = Some(Instant::now());
            // The rest of the chord may already be in this frame's events if
            // it was typed as fast as a chord usually is.
            if let Some((key, modifiers)) = ctx.input_mut(take_key) {
                self.chord = None;
                self.window_key(key, modifiers, ctx);
            }
        }
        if hits.toggle_tree {
            self.cfg.tree_visible = !self.cfg.tree_visible;
        }
        if hits.reload {
            self.run(Command::ReloadFile, ctx);
        }
        if hits.bigger {
            self.run(Command::IncreaseFont, ctx);
        }
        if hits.smaller {
            self.run(Command::DecreaseFont, ctx);
        }
        if hits.quit {
            self.quit = true;
        }
        if hits.escape {
            self.notice = None;
        }
        if let Some(index) = hits
            .tab_index
            .filter(|index| *index < self.panes.active().tabs.len())
        {
            self.panes.active_mut().active = index;
        }
    }

    /// Waits out a `ctrl+w` chord, and returns whether it owns this frame.
    ///
    /// While the prefix is armed nothing else sees the keyboard: the next key
    /// belongs to the chord, wherever it would otherwise have gone.
    fn chord(&mut self, ctx: &egui::Context) -> bool {
        let Some(since) = self.chord else {
            return false;
        };
        let waited = since.elapsed();
        if waited >= CHORD_TIMEOUT {
            self.chord = None;
            return false;
        }
        let Some((key, modifiers)) = ctx.input_mut(take_key) else {
            ctx.request_repaint_after(CHORD_TIMEOUT - waited);
            return true;
        };
        self.chord = None;
        self.window_key(key, modifiers, ctx);
        true
    }

    /// The second half of a `ctrl+w` chord, spelled as vim spells it.
    ///
    /// Anything else — `esc` included — just cancels, which is why there is no
    /// fallback arm doing something surprising.
    fn window_key(&mut self, key: Key, modifiers: Modifiers, ctx: &egui::Context) {
        match key {
            Key::V => self.run(Command::SplitRight, ctx),
            Key::S => self.run(Command::SplitDown, ctx),
            Key::H => self.panes.focus_dir(Dir::Left),
            Key::J => self.panes.focus_dir(Dir::Down),
            Key::K => self.panes.focus_dir(Dir::Up),
            Key::L => self.panes.focus_dir(Dir::Right),
            Key::W => self.panes.cycle(!modifiers.shift),
            Key::C | Key::Q => self.run(Command::ClosePane, ctx),
            Key::O => self.run(Command::OnlyPane, ctx),
            Key::D => self.run(Command::CloseTab, ctx),
            Key::Equals | Key::Plus => self.run(Command::EqualizePanes, ctx),
            _ => {}
        }
    }

    // ------------------------------------------------------- external state

    /// Applies watcher events once they have settled.
    fn drain_filesystem(&mut self, ctx: &egui::Context) {
        if let Some(vault) = &self.vault {
            if let Some(watcher) = &vault.watcher {
                let paths = watcher.drain();
                if !paths.is_empty() {
                    self.pending_paths.extend(paths);
                    self.pending_since = Some(Instant::now());
                }
            }
        }

        let Some(since) = self.pending_since else {
            return;
        };
        let waited = since.elapsed();
        if waited < FS_DEBOUNCE {
            ctx.request_repaint_after(FS_DEBOUNCE - waited);
            return;
        }
        self.pending_since = None;
        let paths = std::mem::take(&mut self.pending_paths);

        let mut structural = false;
        for path in &paths {
            for tab in self.panes.docs_mut() {
                if tab.path == *path {
                    tab.reload();
                }
            }
            // A path Faust has never indexed means the vault gained or lost
            // something, so both the tree and the index need refreshing.
            structural |= !self.files.iter().any(|entry| entry.path == *path);
            if let Some(vault) = &mut self.vault {
                vault.tree.invalidate(path);
            }
        }

        if structural {
            if let Some(vault) = &self.vault {
                self.files_generation = self.search.index(&vault.root);
                self.indexing = true;
            }
        }
    }

    fn drain_search(&mut self) {
        for message in self.search.poll() {
            match message {
                Message::Files {
                    generation,
                    entries,
                    done,
                } => {
                    if generation != self.files_generation {
                        continue;
                    }
                    self.files.extend(entries);
                    self.indexing = !done;
                }
                Message::Hits {
                    generation,
                    hits,
                    done,
                } => {
                    if generation != self.hits_generation {
                        continue;
                    }
                    self.hits.extend(hits);
                    self.searching = !done;
                }
            }
        }
    }

    /// Writes the settings file once changes have settled.
    ///
    /// Faust would otherwise only save on a clean exit, and a session that ends
    /// with a logout or a `kill` would silently forget the open vault.
    fn autosave(&mut self, ctx: &egui::Context) {
        if self.cfg == self.saved {
            self.dirty_since = None;
            return;
        }
        let since = *self.dirty_since.get_or_insert_with(Instant::now);
        let waited = since.elapsed();
        if waited < SAVE_DEBOUNCE {
            ctx.request_repaint_after(SAVE_DEBOUNCE - waited);
            return;
        }
        self.dirty_since = None;
        match self.cfg.save() {
            Ok(()) => self.saved = self.cfg.clone(),
            Err(err) => eprintln!("faust: cannot save settings: {err}"),
        }
    }

    /// Submits a content search once typing has paused.
    fn drain_query(&mut self, ctx: &egui::Context) {
        let Some((query, since)) = self.pending_query.clone() else {
            return;
        };
        let waited = since.elapsed();
        if waited < QUERY_DEBOUNCE {
            ctx.request_repaint_after(QUERY_DEBOUNCE - waited);
            return;
        }
        self.pending_query = None;
        let Some(vault) = &self.vault else { return };

        self.hits.clear();
        if query.trim().is_empty() {
            self.searching = false;
            return;
        }
        self.hits_generation = self.search.content(&vault.root, &query);
        self.searching = true;
    }
}

/// Everything the palette's row list is derived from.
///
/// Comparing this is far cheaper than rebuilding the rows it produces.
#[derive(PartialEq, Eq)]
struct RankKey {
    epoch: u64,
    mode: Mode,
    query: String,
    files: usize,
    hits: usize,
    searching: bool,
    indexing: bool,
    browse: PathBuf,
}

/// What a command-line argument turned out to mean.
enum Target {
    Vault(PathBuf),
    Note { vault: PathBuf, note: PathBuf },
}

/// Interprets the command-line argument.
///
/// Passing a note rather than a folder is the common case when Faust is invoked
/// from an editor, so the vault is inferred from the note's ancestry.
fn resolve_target(path: PathBuf, remembered: Option<&Path>) -> Result<Target, String> {
    let path = path.canonicalize().unwrap_or(path);
    if path.is_dir() {
        return Ok(Target::Vault(path));
    }
    if !path.exists() {
        return Err(format!("{} does not exist", path.display()));
    }
    // `faust some/deep/note.md` from an editor should keep the vault you were
    // already in, rather than treating the note's own folder as a new vault.
    let vault = remembered
        .filter(|root| path.starts_with(root) && root.is_dir())
        .map_or_else(|| vault_root_for(&path), Path::to_path_buf);
    Ok(Target::Note { vault, note: path })
}

/// Walks up from a note looking for a vault marker, stopping at `$HOME`.
///
/// `.obsidian` is the vault's own marker; `.git` is the next best signal that a
/// directory is the root of something. Without either, the note's own folder is
/// the vault, which keeps `faust scratch.md` cheap.
fn vault_root_for(note: &Path) -> PathBuf {
    let home = std::env::var_os("HOME").map(PathBuf::from);
    let fallback = note.parent().unwrap_or(Path::new(".")).to_path_buf();

    let mut dir = note.parent();
    while let Some(current) = dir {
        if current.join(".obsidian").is_dir() || current.join(".git").exists() {
            return current.to_owned();
        }
        if home.as_deref() == Some(current) {
            break;
        }
        dir = current.parent();
    }
    fallback
}

/// Which shortcuts fired this frame.
struct Shortcuts {
    palette: bool,
    find_file: bool,
    content: bool,
    next_tab: bool,
    prev_tab: bool,
    close_tab: bool,
    /// The `ctrl+w` prefix, which arms a chord rather than doing anything.
    window: bool,
    toggle_tree: bool,
    reload: bool,
    quit: bool,
    bigger: bool,
    smaller: bool,
    escape: bool,
    tab_index: Option<usize>,
    /// Index into [`MOTIONS`] of the `ctrl+h/j/k/l` that fired.
    motion: Option<usize>,
}

/// Takes the first key press of the frame, and the text it produced with it.
///
/// A chord's second key must not also reach whatever it is normally bound to,
/// and `consume_key` cannot help here because the key is not known in advance.
fn take_key(input: &mut egui::InputState) -> Option<(Key, Modifiers)> {
    let mut found = None;
    input.events.retain(|event| match event {
        egui::Event::Key {
            key,
            modifiers,
            pressed: true,
            ..
        } if found.is_none() => {
            found = Some((*key, *modifiers));
            false
        }
        egui::Event::Text(_) => false,
        _ => true,
    });
    found
}

fn name_of(path: &Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string())
}

/// Hands a path to the desktop's default handler.
fn open_externally(path: &Path) {
    let _ = std::process::Command::new("xdg-open")
        .arg(path)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn();
}
