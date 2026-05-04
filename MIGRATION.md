# richspace Migration to xfce-panel-rs

## Summary

This plugin currently implements its own minimal XFCE panel FFI bindings (~150 lines total across ffi.rs + plugin.rs). Worker-1 is creating `~/Workspace/Lib/xfce-panel-rs/` as a shared crate to replace this duplication across all panel plugins.

**Migration Complexity:** LOW - This is the simplest plugin with minimal FFI usage.

## Current FFI Location

```
src/xfce/
├── ffi.rs     (~67 lines) - Raw FFI declarations
├── plugin.rs  (~150 lines) - Safe wrapper around XfcePanelPlugin
└── mod.rs     (~8 lines) - Module exports
```

## FFI Inventory

### Types Used (from ffi.rs)

```rust
// Opaque pointer type
pub type XfcePanelPluginPointer = *mut gtk_sys::GtkWidget;

// Enums
pub enum XfceScreenPosition { ... }   // 16 variants (unused in app.rs)
pub enum XfcePanelPluginMode { ... }  // 3 variants (unused in app.rs)
```

### FFI Functions Used (from ffi.rs)

```rust
// Plugin identity
xfce_panel_plugin_get_name(plugin) -> *const c_char
xfce_panel_plugin_get_unique_id(plugin) -> c_int

// Panel geometry
xfce_panel_plugin_get_orientation(plugin) -> GtkOrientation
xfce_panel_plugin_get_screen_position(plugin) -> XfceScreenPosition
xfce_panel_plugin_get_size(plugin) -> c_int
xfce_panel_plugin_get_mode(plugin) -> XfcePanelPluginMode
xfce_panel_plugin_get_icon_size(plugin) -> c_int

// Menu integration
xfce_panel_plugin_menu_show_configure(plugin)
xfce_panel_plugin_add_action_widget(plugin, widget)

// Configuration file paths
xfce_panel_plugin_save_location(plugin, create: gboolean) -> *mut c_char
```

### Safe Wrapper Methods (from plugin.rs)

```rust
impl XfcePanelPlugin {
    // Construction
    pub fn from_raw(pointer: XfcePanelPluginPointer) -> Self

    // Identity
    pub fn id(&self) -> String                  // Constructed: name + "-" + unique_id
    pub fn name(&self) -> String

    // Geometry
    pub fn orientation(&self) -> gtk::Orientation
    pub fn size(&self) -> i32
    pub fn icon_size(&self) -> i32
    pub fn screen_position(&self) -> XfceScreenPosition
    pub fn mode(&self) -> XfcePanelPluginMode

    // Menu integration
    pub fn add_action_widget(&self, widget: &impl IsA<gtk::Widget>)
    pub fn menu_show_configure(&self)

    // Configuration
    pub fn config_path(&self) -> Option<PathBuf>  // save_location + .rc -> .json conversion

    // Signal handlers (manual trampolines via container.connect_local)
    pub fn connect_orientation_changed<F>(&self, f: F)
    pub fn connect_size_changed<F>(&self, f: F)
    pub fn connect_free_data<F>(&self, f: F)
    pub fn connect_save<F>(&self, f: F)
    pub fn connect_configure_plugin<F>(&self, f: F)
}
```

## Usage in App

From `src/lib.rs` (constructor):
```rust
let plugin = XfcePanelPlugin::from_raw(pointer);
let plugin_id = plugin.id();
let plugin_name = plugin.name();
```

From `src/app.rs` (App::start):
```rust
// Initialization
let orientation = plugin.orientation();
let size = plugin.size();
let config_path = plugin.config_path();

// Widget setup
plugin.container.add(widget.widget());
plugin.add_action_widget(widget.widget());
plugin.menu_show_configure();

// Signal handlers
plugin.connect_orientation_changed(move |orientation| { ... });
plugin.connect_size_changed(move |size| { ... });
plugin.connect_configure_plugin(move || { ... });
plugin.connect_save(move || { ... });
plugin.connect_free_data(move || { ... });
```

## Migration Steps

### 1. Add xfce-panel-rs Dependency

**File:** `Cargo.toml`

```diff
 [dependencies]
+# XFCE panel plugin bindings (shared across plugins)
+xfce-panel-rs = { path = "../../Lib/xfce-panel-rs" }
+
 # GTK3 bindings
 gtk = { version = "0.18", features = ["v3_24"] }
 glib = "0.18"

-# Low-level GTK sys crates for FFI
-gtk-sys = "0.18"
-glib-sys = "0.18"
-gobject-sys = "0.18"
-gdk-sys = "0.18"

-# FFI utilities
-libc = "0.2"
```

**Note:** Keep `gdk`, `pango`, `cairo-rs` - those are still used by the drawing logic.

### 2. Update Imports

**File:** `src/lib.rs`

```diff
-mod xfce;
-use xfce::{XfcePanelPluginPointer, XfcePanelPlugin};
+use xfce_panel_rs::{XfcePanelPluginPointer, XfcePanelPlugin};
```

**File:** `src/app.rs`

```diff
-use crate::xfce::XfcePanelPlugin;
+use xfce_panel_rs::XfcePanelPlugin;
```

### 3. Delete Local FFI

```bash
rm -rf src/xfce/
```

### 4. Verify Build

```bash
just build
```

### 5. Test

```bash
just install
# Restart panel: xfce4-panel -r
# Check logs: journalctl -t richspace -f
```

## Expected API Compatibility

All methods currently used by richspace should map 1:1 to xfce-panel-rs.

### Potential Gaps

1. **config_path() .rc → .json conversion**
   - Current wrapper converts `.rc` to `.json` suffix
   - xfce-panel-rs may return raw `.rc` path
   - **Fix:** Add `.replace(".rc", ".json")` in app.rs if needed

2. **Signal handlers use container.connect_local()**
   - Current implementation uses manual trampolines
   - xfce-panel-rs should provide higher-level wrappers
   - **Verify:** Signal API matches exactly

3. **icon_size(), mode(), screen_position() methods**
   - These are defined but NEVER used in app.rs
   - xfce-panel-rs may skip them initially
   - **Action:** Document as "unused in richspace, safe to omit from shared crate"

## Files Modified

```
Cargo.toml              # Add xfce-panel-rs dependency, remove sys crates
src/lib.rs              # Update import from crate::xfce to xfce_panel_rs
src/app.rs              # Update import from crate::xfce to xfce_panel_rs
src/xfce/               # DELETE entire directory
```

## Testing Checklist

- [ ] Plugin loads without errors
- [ ] Workspace switching works
- [ ] Orientation changes apply correctly
- [ ] Panel size changes apply correctly
- [ ] Right-click menu shows "Configure" (even if dialog is TODO)
- [ ] Config file path resolves to .json correctly
- [ ] File watchers don't crash on hot-reload
- [ ] Provider IPC updates still render

## Notes

- This is the **smallest and simplest** plugin FFI usage
- Good candidate for **first migration** after tasklist
- **No custom FFI extensions** - pure standard usage
- The unused methods (icon_size, mode, screen_position) suggest this plugin was copy-pasted from a more complete template, then simplified

## Comparison with Other Plugins

| Plugin | FFI Lines | Complexity | Custom Extensions |
|--------|-----------|------------|-------------------|
| tasklist | ~350 | HIGH | Tooltip hooks, extended API |
| windowck | ~283 | MEDIUM | Custom window tracking |
| **richspace** | **~67** | **LOW** | **None** |
| richmon | ~83 | LOW | None |
| treasures | ~113 | LOW | None |

Richspace is tied with richmon for **simplest FFI usage**.
