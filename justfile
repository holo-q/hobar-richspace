# Richspace - XFCE4 Panel Plugin
# Custom workspace display with configurable labels and icons
#
# BUILD ARCHITECTURE:
# Rustc's cdylib hides C symbols, but XFCE needs xfce_panel_module_construct
# exposed via dlsym(). Solution: Build Rust staticlib, then GCC links everything.

set shell := ["bash", "-c"]

plugin_dir := "~/.local/lib/xfce4/panel/plugins"
desktop_dir := "~/.local/share/xfce4/panel/plugins"

# List recipes
default:
    @just --list

# Build debug (Rust staticlib only - use link-debug for full plugin)
build:
    cargo build

# Build release (Rust staticlib only - use link-release for full plugin)
release:
    cargo build --release

# Link final plugin .so (debug)
link-debug: build
    gcc -Wall -shared -fPIC -o target/debug/librichspace.so plugin.c \
        -Wl,--whole-archive target/debug/librichspace.a -Wl,--no-whole-archive \
        $(pkg-config --cflags --libs libxfce4panel-2.0 gtk+-3.0 libwnck-3.0)

# Link final plugin .so (release)
link-release: release
    gcc -Wall -shared -fPIC -O2 -o target/release/librichspace.so plugin.c \
        -Wl,--whole-archive target/release/librichspace.a -Wl,--no-whole-archive \
        $(pkg-config --cflags --libs libxfce4panel-2.0 gtk+-3.0 libwnck-3.0)

# Validate plugin .so has no unresolved xfce symbols
# CRITICAL: Catches missing symbol errors BEFORE panel restart crashes
validate-symbols:
    #!/usr/bin/env bash
    set -e
    echo "Checking for undefined xfce symbols..."
    plugin_syms=$(nm -D target/release/librichspace.so 2>/dev/null | grep "U xfce_panel" | awk '{print $2}')
    lib_syms=$(nm -D /usr/lib/libxfce4panel-2.0.so.4 2>/dev/null | grep " T " | awk '{print $3}')
    errors=""
    for sym in $plugin_syms; do
        if ! echo "$lib_syms" | grep -q "^${sym}$"; then
            errors="$errors\n  $sym"
        fi
    done
    if [ -n "$errors" ]; then
        echo -e "ERROR: Undefined symbols not in libxfce4panel:$errors"
        echo "The plugin will fail to load! Fix FFI bindings in src/xfce/ffi.rs"
        exit 1
    fi
    echo "All xfce symbols resolved"

# Full build (Rust + GCC link)
full-debug: link-debug

# Full release build (Rust + GCC link)
full-release: link-release

# Install plugin (validates symbols before installing to prevent crash loops)
install: link-release validate-symbols
    install -Dm755 target/release/librichspace.so {{plugin_dir}}/librichspace.so
    install -Dm644 richspace.desktop {{desktop_dir}}/richspace.desktop
    xfce4-panel -r
    @echo "Installed! Add 'Rich Workspaces' to your panel."

# Uninstall plugin
uninstall:
    rm -f {{plugin_dir}}/librichspace.so
    rm -f {{desktop_dir}}/richspace.desktop
    @echo "Uninstalled."

# Watch and rebuild on changes
watch:
    cargo watch -x build

# Run tests
test:
    cargo test

# Check types
check:
    cargo check

# Format code
fmt:
    cargo fmt

# Lint
lint:
    cargo clippy

# Clean build artifacts
clean:
    cargo clean

# View plugin logs
logs:
    journalctl -f -t xfce4-panel | grep -i richspace

# Restart panel
restart:
    xfce4-panel -r

# Show state file location
state-path:
    @echo "$XDG_RUNTIME_DIR/richspace/state.json"

# Clear ephemeral state (force reset)
clear-state:
    rm -f "$XDG_RUNTIME_DIR/richspace/state.json"
    @echo "State cleared. Will reset to defaults on next panel restart."
