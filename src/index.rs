//! Background vault index and content search.
//!
//! One worker thread serves both. Only the newest request is ever pending, so
//! typing quickly in the palette cannot build a backlog of stale walks, and the
//! worker checks the generation counter between files to abandon superseded
//! work instead of finishing it.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::thread::JoinHandle;

use ignore::WalkBuilder;

/// Entries streamed to the UI in batches this large.
const BATCH: usize = 512;
/// Upper bound on indexed entries, so a stray vault cannot exhaust memory.
const MAX_ENTRIES: usize = 200_000;
/// Content search stops here; nobody scrolls past it.
const MAX_HITS: usize = 400;
/// Files larger than this are not searched for content.
const MAX_SEARCH_BYTES: u64 = 2 << 20;
/// Matching lines are elided beyond this many bytes.
const MAX_LINE: usize = 240;

/// A file or folder in the vault.
#[derive(Clone)]
pub struct FileEntry {
    pub path: PathBuf,
    /// Path relative to the vault root, used as the fuzzy haystack.
    pub display: String,
    pub is_dir: bool,
}

/// One matching line.
#[derive(Clone)]
pub struct Hit {
    pub path: PathBuf,
    pub display: String,
    pub line: u32,
    pub text: String,
}

pub enum Message {
    Files {
        generation: u64,
        entries: Vec<FileEntry>,
        done: bool,
    },
    Hits {
        generation: u64,
        hits: Vec<Hit>,
        done: bool,
    },
}

enum Kind {
    Index,
    Content(String),
}

struct Job {
    root: PathBuf,
    generation: u64,
    kind: Kind,
}

struct Shared {
    pending: Mutex<Option<Job>>,
    wake: Condvar,
    /// Newest request; the worker abandons anything older.
    generation: AtomicU64,
    stop: AtomicBool,
}

impl Shared {
    fn lock(&self) -> MutexGuard<'_, Option<Job>> {
        // A panicking worker must not wedge the UI thread, and the protected
        // value is a plain Option, so the poison flag carries no information.
        self.pending.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn current(&self, generation: u64) -> bool {
        self.generation.load(Ordering::Relaxed) == generation && !self.stop.load(Ordering::Relaxed)
    }
}

pub struct Search {
    shared: Arc<Shared>,
    rx: Receiver<Message>,
    worker: Option<JoinHandle<()>>,
}

impl Search {
    pub fn new(ctx: egui::Context) -> Self {
        let shared = Arc::new(Shared {
            pending: Mutex::new(None),
            wake: Condvar::new(),
            generation: AtomicU64::new(0),
            stop: AtomicBool::new(false),
        });
        let (tx, rx) = mpsc::channel();
        let worker = {
            let shared = Arc::clone(&shared);
            std::thread::Builder::new()
                .name("faust-search".into())
                .spawn(move || run(&shared, &tx, &ctx))
                .ok()
        };
        Self { shared, rx, worker }
    }

    /// Rebuilds the file list. Returns the generation the results will carry.
    pub fn index(&self, root: &Path) -> u64 {
        self.submit(root, Kind::Index)
    }

    /// Searches file contents for `query`.
    pub fn content(&self, root: &Path, query: &str) -> u64 {
        self.submit(root, Kind::Content(query.to_owned()))
    }

    fn submit(&self, root: &Path, kind: Kind) -> u64 {
        let generation = self.shared.generation.fetch_add(1, Ordering::Relaxed) + 1;
        *self.shared.lock() = Some(Job {
            root: root.to_owned(),
            generation,
            kind,
        });
        self.shared.wake.notify_one();
        generation
    }

    /// Everything the worker has produced since the last call.
    pub fn poll(&self) -> Vec<Message> {
        let mut out = Vec::new();
        while let Ok(message) = self.rx.try_recv() {
            out.push(message);
        }
        out
    }
}

impl Drop for Search {
    fn drop(&mut self) {
        self.shared.stop.store(true, Ordering::Relaxed);
        self.shared.generation.fetch_add(1, Ordering::Relaxed);
        self.shared.wake.notify_all();
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

fn run(shared: &Shared, tx: &Sender<Message>, ctx: &egui::Context) {
    loop {
        let job = {
            let mut pending = shared.lock();
            while pending.is_none() && !shared.stop.load(Ordering::Relaxed) {
                let (guard, _) = shared
                    .wake
                    .wait_timeout(pending, std::time::Duration::from_millis(250))
                    .unwrap_or_else(|e| e.into_inner());
                pending = guard;
            }
            if shared.stop.load(Ordering::Relaxed) {
                return;
            }
            pending.take()
        };
        let Some(job) = job else { continue };

        match job.kind {
            Kind::Index => index(shared, tx, ctx, &job.root, job.generation),
            Kind::Content(query) => content(shared, tx, ctx, &job.root, job.generation, &query),
        }
    }
}

/// Builds a walker with the ignore rules the tree also uses, so what you can
/// search is exactly what you can see.
fn walker(root: &Path) -> ignore::Walk {
    WalkBuilder::new(root)
        .hidden(true)
        .parents(true)
        .git_ignore(true)
        .git_global(true)
        .git_exclude(true)
        .require_git(false)
        .follow_links(false)
        .build()
}

fn index(shared: &Shared, tx: &Sender<Message>, ctx: &egui::Context, root: &Path, generation: u64) {
    let mut batch = Vec::with_capacity(BATCH);
    let mut total = 0usize;

    for entry in walker(root).filter_map(Result::ok) {
        if !shared.current(generation) {
            return;
        }
        if entry.depth() == 0 {
            continue;
        }
        let is_dir = entry.file_type().is_some_and(|t| t.is_dir());
        let path = entry.into_path();
        let Some(display) = relative(root, &path) else {
            continue;
        };
        batch.push(FileEntry {
            path,
            display,
            is_dir,
        });
        total += 1;

        if batch.len() >= BATCH {
            send(
                tx,
                ctx,
                Message::Files {
                    generation,
                    entries: std::mem::take(&mut batch),
                    done: false,
                },
            );
            batch.reserve(BATCH);
        }
        if total >= MAX_ENTRIES {
            break;
        }
    }

    send(
        tx,
        ctx,
        Message::Files {
            generation,
            entries: batch,
            done: true,
        },
    );
}

fn content(
    shared: &Shared,
    tx: &Sender<Message>,
    ctx: &egui::Context,
    root: &Path,
    generation: u64,
    query: &str,
) {
    let terms: Vec<String> = query
        .split_whitespace()
        .map(str::to_lowercase)
        .filter(|t| !t.is_empty())
        .collect();
    if terms.is_empty() {
        send(
            tx,
            ctx,
            Message::Hits {
                generation,
                hits: Vec::new(),
                done: true,
            },
        );
        return;
    }

    let mut hits = Vec::new();
    let mut sent = 0usize;

    for entry in walker(root).filter_map(Result::ok) {
        if !shared.current(generation) {
            return;
        }
        if entry.file_type().is_some_and(|t| t.is_dir()) {
            continue;
        }
        let too_big = entry.metadata().is_ok_and(|m| m.len() > MAX_SEARCH_BYTES);
        if too_big {
            continue;
        }
        let path = entry.into_path();
        let Ok(bytes) = std::fs::read(&path) else {
            continue;
        };
        if bytes[..bytes.len().min(1024)].contains(&0) {
            continue;
        }
        let Some(display) = relative(root, &path) else {
            continue;
        };
        let text = String::from_utf8_lossy(&bytes);

        for (number, line) in text.lines().enumerate() {
            let lowered = line.to_lowercase();
            if !terms.iter().all(|term| lowered.contains(term)) {
                continue;
            }
            hits.push(Hit {
                path: path.clone(),
                display: display.clone(),
                line: number as u32 + 1,
                text: elide(line),
            });
            sent += 1;
            if hits.len() >= 64 {
                send(
                    tx,
                    ctx,
                    Message::Hits {
                        generation,
                        hits: std::mem::take(&mut hits),
                        done: false,
                    },
                );
            }
            if sent >= MAX_HITS {
                send(
                    tx,
                    ctx,
                    Message::Hits {
                        generation,
                        hits,
                        done: true,
                    },
                );
                return;
            }
        }
    }

    send(
        tx,
        ctx,
        Message::Hits {
            generation,
            hits,
            done: true,
        },
    );
}

fn send(tx: &Sender<Message>, ctx: &egui::Context, message: Message) {
    if tx.send(message).is_ok() {
        ctx.request_repaint();
    }
}

fn relative(root: &Path, path: &Path) -> Option<String> {
    Some(
        path.strip_prefix(root)
            .unwrap_or(path)
            .to_string_lossy()
            .into_owned(),
    )
}

/// Trims a matching line to something that fits on one row.
fn elide(line: &str) -> String {
    let line = line.trim();
    if line.len() <= MAX_LINE {
        return line.to_owned();
    }
    let mut end = MAX_LINE;
    while end > 0 && !line.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}…", &line[..end])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn long_lines_are_elided_on_a_char_boundary() {
        let line = "é".repeat(400);
        let out = elide(&line);
        assert!(out.ends_with('…'));
        assert!(out.len() <= MAX_LINE + 4);
    }

    #[test]
    fn short_lines_are_untouched() {
        assert_eq!(elide("  hello  "), "hello");
    }
}
