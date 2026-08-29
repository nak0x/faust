//! Filesystem watch over the open vault.
//!
//! The watcher owns no state beyond the paths it has seen since the last drain;
//! deciding what a change *means* is the app's job.

use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver};

use notify::{RecommendedWatcher, RecursiveMode, Watcher};

pub struct VaultWatcher {
    /// Dropped on close, which unregisters the inotify watches.
    _watcher: RecommendedWatcher,
    rx: Receiver<PathBuf>,
}

impl VaultWatcher {
    /// Watches `root` recursively, waking the UI on every event.
    pub fn new(root: &Path, ctx: egui::Context) -> notify::Result<Self> {
        let (tx, rx) = mpsc::channel();
        let mut watcher =
            notify::recommended_watcher(move |event: notify::Result<notify::Event>| {
                let Ok(event) = event else { return };
                if !matches!(
                    event.kind,
                    notify::EventKind::Create(_)
                        | notify::EventKind::Modify(_)
                        | notify::EventKind::Remove(_)
                ) {
                    return;
                }
                let mut any = false;
                for path in event.paths {
                    any |= tx.send(path).is_ok();
                }
                if any {
                    // Faust runs on demand rather than at a fixed frame rate, so a
                    // change on disk has to ask for the next frame explicitly.
                    ctx.request_repaint();
                }
            })?;
        watcher.watch(root, RecursiveMode::Recursive)?;
        Ok(Self {
            _watcher: watcher,
            rx,
        })
    }

    /// Takes every path reported since the last call.
    pub fn drain(&self) -> Vec<PathBuf> {
        let mut paths = Vec::new();
        while let Ok(path) = self.rx.try_recv() {
            paths.push(path);
        }
        paths
    }
}
