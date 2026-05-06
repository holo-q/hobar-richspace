# hobar-richspace

Panel plugin for rich, configurable workspace/window visualization. A drop-in
replacement for the stock workspace switcher that adds:

- **Configurable per-workspace labels** — number, WM name, or custom string
- **Auto-icons via WM_CLASS rules** — emoji, FontAwesome, Nerd Font, any Unicode glyph
- **Macro-based icon rules** — group app classes (browsers, file managers, IDEs) into
  reusable named sets, reference them from icon rules
- **Window count badges** — hidden, tooltip, badge, or inline display modes
- **Drag-and-drop window moves** — drop tasklist windows onto any workspace
- **Keyboard reorder + true window swap** — Ctrl+Shift+Arrow to swap workspaces
- **Animated reorder transitions** — `gtk::Fixed` container with tweened motion
- **Live config reload** — edit TOML, see changes without restart (uses `notify` watcher)
- **Structured tracing → journald** — `journalctl -t richspace -f`

## Build

Requires Rust (edition 2021), GCC, pkg-config, and the GTK3 / libwnck-3 / libxfce4panel-2.0
development headers.

```sh
# Debug build (Rust staticlib + GCC final link)
just full-debug

# Release build
just full-release

# Or step by step
just release        # cargo build --release  (Rust staticlib only)
just link-release   # GCC links Rust .a + plugin.c → librichspace.so
```

The split exists because `rustc`'s `cdylib` hides C symbols, but XFCE needs
`xfce_panel_module_construct` exposed via `dlsym()`. Solution: build a Rust
`staticlib`, then let GCC do the final shared-object link with
`-Wl,--whole-archive`.

> **Note**: this crate currently has path dependencies on three sibling crates
> (`wnck-rs`, `spaceship-std`, `babel`) that live in the author's monorepo. To
> build standalone you'll need to vendor those or rewrite `Cargo.toml` to point
> at upstream/published versions.

## Install

```sh
just install        # builds release, validates symbols, installs to:
                    #   ~/.local/lib/xfce4/panel/plugins/librichspace.so
                    #   ~/.local/share/xfce4/panel/plugins/richspace.desktop
                    # then xfce4-panel -r
```

Then right-click the panel → *Add New Items…* → **Rich Workspaces**.

## Configure

Per-instance TOML at `~/.config/xfce4/panel/richspace-N.toml` (where `N` is the
panel plugin ID). Live-reloaded on save.

```toml
[macros]
browser = ["firefox", "brave-browser", "chromium"]
fm      = ["nemo", "nautilus", "thunar"]

[[icon_rules]]
macro      = "browser"
icon       = "󰖟"
match_mode = "all"

[[icon_rules]]
class_regex = "^code$"
icon        = "󰨞"
match_mode  = "any"
```

Display modes (`icon_only` / `label_only` / `icon_and_label`), label sources
(`number` / `wm_name` / `custom`), and window count display
(`hidden` / `tooltip` / `badge` / `inline`) are all configurable. See `src/config.rs`
for the full schema.

## Logs

```sh
journalctl -t richspace -f
```

Log level is centralized via `~/Workspace/logging.toml` and hot-reloaded on
`SIGHUP` (e.g. `pkill -HUP xfce4-panel`).

## License

GPL-2.0-or-later. See [LICENSE](./LICENSE).
