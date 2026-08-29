//! Faust — a live Markdown preview for Obsidian vaults.
//!
//! Faust does not edit. You write in your editor of choice; Faust watches the
//! vault and re-renders the moment a file is saved.

#![forbid(unsafe_code)]

mod app;
mod config;
mod document;
mod fuzzy;
mod index;
mod markdown;
mod palette;
mod render;
mod theme;
mod tree;
mod watcher;

use std::path::PathBuf;
use std::process::ExitCode;

use config::Config;

const USAGE: &str = "\
faust — live Markdown preview for Obsidian vaults

USAGE:
    faust [PATH]

ARGS:
    PATH    A vault folder to open, or a note to open inside its vault.
            Defaults to the last vault used.

OPTIONS:
    -h, --help       Print this help
    -V, --version    Print the version

KEYS:
    alt+space        command palette (everything lives here)
    ctrl+p           find a file or folder by name
    ctrl+shift+f     search inside notes
    ctrl+w           close tab
    ctrl+tab         next tab            ctrl+shift+tab   previous tab
    ctrl+1..9        jump to tab
    ctrl+b           toggle the file tree
    ctrl+r           reload the current file
    ctrl+= / ctrl+-  font size
    ctrl+q           quit
";

fn main() -> ExitCode {
    let mut target = None;
    for arg in std::env::args().skip(1) {
        match arg.as_str() {
            "-h" | "--help" => {
                print!("{USAGE}");
                return ExitCode::SUCCESS;
            }
            "-V" | "--version" => {
                println!("faust {}", env!("CARGO_PKG_VERSION"));
                return ExitCode::SUCCESS;
            }
            other if other.starts_with('-') => {
                eprintln!("faust: unknown option `{other}`\n\n{USAGE}");
                return ExitCode::FAILURE;
            }
            other => target = Some(PathBuf::from(other)),
        }
    }

    let config = Config::load();
    let viewport = egui::ViewportBuilder::default()
        // The Wayland app id: window rules and the compositor's blur effect
        // both key off this.
        .with_app_id("faust")
        .with_title("Faust")
        .with_inner_size(config.window)
        .with_min_inner_size([480.0, 320.0])
        .with_transparent(true)
        .with_decorations(config.decorations);

    let options = eframe::NativeOptions {
        viewport,
        renderer: eframe::Renderer::Glow,
        // Faust draws flat rectangles and text; none of these buffers earn
        // their memory.
        multisampling: 0,
        depth_buffer: 0,
        stencil_buffer: 0,
        persist_window: false,
        persistence_path: None,
        ..Default::default()
    };

    let result = eframe::run_native(
        "faust",
        options,
        Box::new(move |cc| Ok(Box::new(app::App::new(cc, config, target)))),
    );

    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("faust: {err}");
            ExitCode::FAILURE
        }
    }
}
