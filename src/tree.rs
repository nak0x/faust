//! Gitignore-aware file tree.
//!
//! Directories are listed only when expanded, so opening a vault costs one
//! `read_dir` of the root no matter how large the vault is. Listings for
//! collapsed directories are dropped again to keep the resident set flat.

use std::collections::{BTreeSet, HashMap};
use std::path::{Path, PathBuf};

use ignore::WalkBuilder;

/// One visible line of the tree.
pub struct Row {
    pub path: PathBuf,
    pub name: String,
    pub depth: usize,
    pub is_dir: bool,
    pub expanded: bool,
}

struct Child {
    path: PathBuf,
    name: String,
    is_dir: bool,
}

pub struct FileTree {
    root: PathBuf,
    expanded: BTreeSet<PathBuf>,
    listings: HashMap<PathBuf, Vec<Child>>,
    rows: Vec<Row>,
    dirty: bool,
}

impl FileTree {
    pub fn new(root: PathBuf) -> Self {
        Self {
            root,
            expanded: BTreeSet::new(),
            listings: HashMap::new(),
            rows: Vec::new(),
            dirty: true,
        }
    }

    /// Visible rows, rebuilt only when something changed.
    pub fn rows(&mut self) -> &[Row] {
        if self.dirty {
            self.rebuild();
        }
        &self.rows
    }

    pub fn toggle(&mut self, path: &Path) {
        if self.expanded.remove(path) {
            // Collapsed directories keep no listing; re-reading on the next
            // expand is cheaper than holding every subtree ever visited.
            self.listings.remove(path);
        } else {
            self.expanded.insert(path.to_owned());
        }
        self.dirty = true;
    }

    /// Expands every ancestor of `path` so it becomes visible.
    pub fn reveal(&mut self, path: &Path) {
        let mut current = path.parent();
        while let Some(dir) = current {
            if !dir.starts_with(&self.root) && dir != self.root {
                break;
            }
            self.expanded.insert(dir.to_owned());
            if dir == self.root {
                break;
            }
            current = dir.parent();
        }
        self.dirty = true;
    }

    /// Drops the cached listing for whichever directory `path` lives in.
    pub fn invalidate(&mut self, path: &Path) {
        if let Some(parent) = path.parent() {
            if self.listings.remove(parent).is_some() {
                self.dirty = true;
            }
        }
        if self.listings.remove(path).is_some() {
            self.dirty = true;
        }
    }

    fn rebuild(&mut self) {
        self.dirty = false;
        let mut rows = Vec::with_capacity(self.rows.len().max(32));
        let root = self.root.clone();
        self.push_rows(&root, 0, &mut rows);
        self.rows = rows;
    }

    fn push_rows(&mut self, dir: &Path, depth: usize, rows: &mut Vec<Row>) {
        self.ensure_listing(dir);
        let Some(children) = self.listings.get(dir) else {
            return;
        };
        let snapshot: Vec<(PathBuf, String, bool)> = children
            .iter()
            .map(|c| (c.path.clone(), c.name.clone(), c.is_dir))
            .collect();

        for (path, name, is_dir) in snapshot {
            let expanded = is_dir && self.expanded.contains(&path);
            rows.push(Row {
                path: path.clone(),
                name,
                depth,
                is_dir,
                expanded,
            });
            if expanded {
                self.push_rows(&path, depth + 1, rows);
            }
        }
    }

    fn ensure_listing(&mut self, dir: &Path) {
        if self.listings.contains_key(dir) {
            return;
        }
        self.listings.insert(dir.to_owned(), read_dir(dir));
    }
}

/// Lists one directory level, honouring ignore files.
///
/// `require_git(false)` matters here: an Obsidian vault usually has a
/// `.gitignore` but no `.git`, and the default would then ignore the file.
fn read_dir(dir: &Path) -> Vec<Child> {
    let mut children: Vec<Child> = WalkBuilder::new(dir)
        .max_depth(Some(1))
        .hidden(true)
        .parents(true)
        .git_ignore(true)
        .git_global(true)
        .git_exclude(true)
        .require_git(false)
        .follow_links(false)
        .build()
        .filter_map(Result::ok)
        .filter(|entry| entry.depth() > 0)
        .filter_map(|entry| {
            let is_dir = entry.file_type().is_some_and(|t| t.is_dir());
            let path = entry.into_path();
            let name = path.file_name()?.to_string_lossy().into_owned();
            Some(Child { path, name, is_dir })
        })
        .collect();

    children.sort_by(|a, b| {
        b.is_dir
            .cmp(&a.is_dir)
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });
    children
}
