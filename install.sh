#!/usr/bin/env sh
# Builds Faust and installs it into the current user's home. No root needed.
#
#   ./install.sh            build and install
#   ./install.sh uninstall  remove everything this script installed
set -eu

PREFIX="${PREFIX:-$HOME/.local}"
BIN="$PREFIX/bin/faust"
DESKTOP="$PREFIX/share/applications/faust.desktop"
ICON="$PREFIX/share/icons/hicolor/scalable/apps/faust.svg"
HERE=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)

refresh_caches() {
    [ -x "$(command -v update-desktop-database)" ] &&
        update-desktop-database "$PREFIX/share/applications" 2>/dev/null || true
    [ -x "$(command -v gtk-update-icon-cache)" ] &&
        gtk-update-icon-cache -qtf "$PREFIX/share/icons/hicolor" 2>/dev/null || true
}

if [ "${1:-install}" = "uninstall" ]; then
    rm -f "$BIN" "$DESKTOP" "$ICON"
    refresh_caches
    echo "faust: removed. Settings in ${XDG_CONFIG_HOME:-$HOME/.config}/faust were kept."
    exit 0
fi

command -v cargo >/dev/null 2>&1 || {
    echo "faust: cargo not found. Install Rust first: https://rustup.rs" >&2
    exit 1
}

echo "faust: building (release)…"
cargo build --release --manifest-path "$HERE/Cargo.toml"

install -Dm755 "$HERE/target/release/faust" "$BIN"
install -Dm644 "$HERE/assets/faust.desktop" "$DESKTOP"
install -Dm644 "$HERE/assets/faust.svg" "$ICON"
refresh_caches

echo "faust: installed $BIN"
case ":$PATH:" in
    *":$PREFIX/bin:"*) ;;
    *) echo "faust: note — $PREFIX/bin is not on your PATH" ;;
esac

# Faust draws a translucent window; the blur behind it belongs to the
# compositor. On GNOME that is the Blur My Shell extension.
BMS="$HOME/.local/share/gnome-shell/extensions/blur-my-shell@aunetx"
if [ -d "$BMS/schemas" ]; then
    SIGMA=$(gsettings --schemadir "$BMS/schemas" \
        get org.gnome.shell.extensions.blur-my-shell.applications sigma 2>/dev/null || echo "?")
    echo
    echo "faust: Blur My Shell is installed and blurs app windows at sigma $SIGMA."
    echo "       To use the 8px radius Faust is designed around (this is a global"
    echo "       setting and affects every blurred app):"
    echo
    echo "         gsettings --schemadir $BMS/schemas \\"
    echo "           set org.gnome.shell.extensions.blur-my-shell.applications sigma 8"
fi
