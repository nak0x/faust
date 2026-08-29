# Faust

A live Markdown preview for Obsidian vaults, with the look of a TUI and the
rendering of a GPU app.

Faust never writes to your vault. You edit in nvim; Faust watches the folder and
re-renders the moment a file hits disk.

```
faust                 # reopen the last vault
faust ~/notes         # open a vault
faust ~/notes/todo.md # open one note, inferring its vault
```

## Install

```sh
./install.sh              # → ~/.local/bin/faust, plus a desktop entry and icon
./install.sh uninstall
```

Needs a Rust toolchain (<https://rustup.rs>) and the JetBrains Mono Nerd Font.
`PREFIX=/usr/local sudo -E ./install.sh` installs system-wide instead.

## Keys

Everything else lives in the command palette; nothing is on a menu bar.

| key | |
|---|---|
| `alt+space` | command palette |
| `ctrl+p` | find a file or folder by name |
| `ctrl+shift+f` | search inside notes |
| `ctrl+w` | close tab |
| `ctrl+tab` / `ctrl+shift+tab` | cycle tabs |
| `ctrl+1`…`ctrl+9` | jump to tab |
| `ctrl+b` | toggle the file tree |
| `ctrl+r` | reload the current file |
| `ctrl+=` / `ctrl+-` | font size |
| `ctrl+q` | quit |

In the palette: type to filter, `↑`/`↓` (or `ctrl+p`/`ctrl+n`) to move, `enter`
to take the highlighted row — which is the first one until you move.

## The file tree

Dock it left, right or bottom from the palette. It follows the same ignore rules
as ripgrep: `.gitignore`, `.ignore`, `.git/info/exclude`, your global gitignore,
and dotfiles are hidden. Unlike git, Faust honours `.gitignore` even when the
vault is not a repository, because most vaults aren't.

## Settings

`~/.config/faust/faust.conf`, written on exit and read at launch.

| key | default | |
|---|---|---|
| `vault` | — | reopened at next launch |
| `theme` | `light` | `light` is the `#e1e1e1` surface; `dark` suits dark wallpapers |
| `opacity` | `0.82` | alpha of the window surface |
| `blur` | `8` | radius Faust is designed around — applied by the compositor, not by Faust |
| `font_size` | `14` | |
| `decorations` | `false` | title bar; drag the tab strip to move, drag an edge to resize |
| `tree_side` | `left` | `left`, `right`, `bottom` |
| `tree_size` | `260` | |

### Blur

Faust paints a translucent window and stops there — an application cannot blur
what is behind its own window; only the compositor can. On GNOME that is the
[Blur My Shell](https://extensions.gnome.org/extension/3193/blur-my-shell/)
extension, whose *Applications* pipeline blurs any transparent window:

```sh
BMS=~/.local/share/gnome-shell/extensions/blur-my-shell@aunetx
gsettings --schemadir $BMS/schemas \
  set org.gnome.shell.extensions.blur-my-shell.applications sigma 8
```

That radius is global to every blurred window, not just Faust's.

## What it renders

Headings, emphasis, inline and fenced code, block quotes, tables, task lists,
nested lists, rules, footnotes, YAML front matter, `[[wikilinks]]` (resolved
against the vault index) and `#tags`. Images are shown as labelled placeholders:
Faust ships without an image decoder, which is most of why it stays small.

Non-Markdown files open as read-only code. Binary and very large files are
reported rather than rendered.

## Footprint

Faust adds about 12 MB of private memory over an empty `eframe` window on the
same machine, roughly 10 MB of which is the four Nerd Font faces. The rest of a
GPU app's resident size is the OpenGL driver, and is shared with every other
GL application on the system.

The vault is indexed on a single background thread that only ever holds the
newest request, so typing in the palette cannot queue up stale directory walks.
Directories in the tree are read when you expand them and dropped when you
collapse them.

## Licence

MIT
