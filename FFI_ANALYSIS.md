# hobar-richspace FFI Analysis

## Executive Summary

**Plugin Purpose:** Workspace switcher with custom labels, icons, and IPC provider dots
**FFI Complexity:** MINIMAL (67 lines)
**Signal Usage:** Standard (orientation, size, configure, save, free-data)
**Custom Extensions:** NONE

This plugin represents the **minimal viable FFI** for an XFCE panel plugin.

## Complete FFI Function Inventory

### Actually Used in Runtime

```rust
// ✅ CRITICAL - Used in app.rs
xfce_panel_plugin_get_name()              // Called: lib.rs:35, plugin.rs:37
xfce_panel_plugin_get_unique_id()         // Called: plugin.rs:30
xfce_panel_plugin_get_orientation()       // Called: app.rs:147
xfce_panel_plugin_get_size()              // Called: app.rs:148
xfce_panel_plugin_add_action_widget()     // Called: app.rs:160
xfce_panel_plugin_menu_show_configure()   // Called: app.rs:164
xfce_panel_plugin_save_location()         // Called: plugin.rs:97

// ✅ SIGNALS - Connected in app.rs:252-287
orientation-changed    // app.rs:254
size-changed           // app.rs:261
configure-plugin       // app.rs:269
save                   // app.rs:276
free-data              // app.rs:283
```

### Declared But NEVER Used

```rust
// ❌ UNUSED - Defined in ffi.rs but no grep hits in app.rs
xfce_panel_plugin_get_screen_position()   // 0 uses
xfce_panel_plugin_get_mode()              // 0 uses
xfce_panel_plugin_get_icon_size()         // 0 uses

// Types for unused functions
XfceScreenPosition enum (16 variants)      // 0 uses
XfcePanelPluginMode enum (3 variants)      // 0 uses
```

## Signal Implementation Details

All signals use **manual trampolines** via `container.connect_local()`:

```rust
// Pattern: Direct closure connection to GTK container
self.container.connect_local("signal-name", false, move |values| {
    let arg = values[1].get::<Type>().unwrap_or(default);
    f(arg);
    Some(return_value.to_value())  // or None
});
```

### Signal Signatures

```rust
orientation-changed: fn(GtkOrientation) -> ()
size-changed: fn(i32) -> bool              // Must return gboolean
configure-plugin: fn() -> ()
save: fn() -> ()
free-data: fn() -> ()
```

### Notable Implementation

**size-changed** returns bool (must convert to GValue):
```rust
pub fn connect_size_changed<F: Fn(i32) -> bool + 'static>(&self, f: F) {
    self.container.connect_local("size-changed", false, move |values| {
        let size = values[1].get::<i32>().unwrap_or(24);
        Some(f(size).to_value())  // ← Return wrapped in Some()
    });
}
```

All other signals return `None`.

## XfcePanelPlugin Struct

### Fields
```rust
pub struct XfcePanelPlugin {
    pointer: XfcePanelPluginPointer,    // Raw *mut GtkWidget
    pub container: gtk::Container,      // PUBLIC - used in app.rs:159,160,302
}
```

### Container Usage

The `container` field is **publicly exposed** and directly used:
- `app.rs:159` - `plugin.container.add(widget.widget())`
- `app.rs:302` - `plugin.container.show_all()`
- All signal connections via `self.container.connect_local(...)`

This is **critical** - xfce-panel-rs MUST expose the container as public.

## config_path() Special Logic

```rust
pub fn config_path(&self) -> Option<PathBuf> {
    unsafe {
        let ptr = ffi::xfce_panel_plugin_save_location(self.pointer, 1);
        if ptr.is_null() {
            return None;
        }
        let path = CStr::from_ptr(ptr).to_string_lossy().into_owned();
        glib_sys::g_free(ptr as *mut _);  // ← MANUAL FREE

        // CUSTOM LOGIC: Convert .rc to .json
        let path = path.replace(".rc", ".json");
        Some(std::path::PathBuf::from(path))
    }
}
```

**Critical Details:**
1. Calls `xfce_panel_plugin_save_location(plugin, 1)` with `create=1`
2. Result is **owned pointer** - must be freed with `g_free()`
3. Converts `.rc` suffix to `.json` (richspace uses JSON not RC format)

**xfce-panel-rs API Suggestion:**
```rust
// Return raw .rc path, let caller decide format
pub fn config_path(&self) -> Option<PathBuf>

// Or provide conversion helper
pub fn config_path_with_ext(&self, ext: &str) -> Option<PathBuf>
```

## id() Construction

```rust
pub fn id(&self) -> String {
    let name = self.name();  // e.g., "richspace"
    let unique_id = unsafe { ffi::xfce_panel_plugin_get_unique_id(self.pointer) };
    format!("{}-{}", name, unique_id)  // e.g., "richspace-15"
}
```

**Why this exists:**
XFCE doesn't provide `xfce_panel_plugin_get_id()`. Plugins must construct it from name + unique_id.

**Note in ffi.rs:**
```rust
// NOTE: There is no xfce_panel_plugin_get_id - use get_name + get_unique_id
// The "plugin-15" style ID must be constructed from name + unique_id
```

## Memory Management

### Safe Wrapper Pattern

All FFI calls wrapped with:
1. Null pointer checks
2. Default fallbacks
3. Manual memory management (g_free for returned strings)
4. CStr → String conversion

```rust
pub fn name(&self) -> String {
    unsafe {
        let ptr = ffi::xfce_panel_plugin_get_name(self.pointer);
        if ptr.is_null() {
            return String::from("richspace");  // ← Fallback
        }
        CStr::from_ptr(ptr).to_string_lossy().into_owned()
    }
}
```

**xfce_panel_plugin_get_name()** returns **borrowed string** (no free needed).
**xfce_panel_plugin_save_location()** returns **owned string** (must free).

## Build System Integration

### C Shim (plugin.c)
```c
#include <libxfce4panel/libxfce4panel.h>
#include "plugin.h"

XFCE_PANEL_PLUGIN_REGISTER(constructor);
```

Minimal. Just registers the Rust `constructor()` function.

### Linking (Cargo.toml)
```toml
[lib]
crate-type = ["staticlib", "cdylib"]

[profile.release]
lto = false      # ← CRITICAL: LTO strips unreferenced C symbols
strip = false
```

**Why staticlib:** GCC links the Rust staticlib with plugin.c shim.
**Why lto = false:** LTO removes symbols the C macro needs.

## Dependencies for FFI

### Current (to be removed)
```toml
gtk-sys = "0.18"        # Raw GtkWidget pointer, GtkOrientation
glib-sys = "0.18"       # g_free(), gboolean
gobject-sys = "0.18"    # (unused, can remove)
gdk-sys = "0.18"        # (unused, can remove)
libc = "0.2"            # c_char, c_int
```

### After Migration
```toml
xfce-panel-rs = { path = "../../Lib/xfce-panel-rs" }

# Keep these (used by drawing logic, not FFI)
gtk = { version = "0.18", features = ["v3_24"] }
glib = "0.18"
gdk = "0.18"
pango = "0.18"
cairo-rs = "0.18"
```

## API Requirements for xfce-panel-rs

### Minimum API (what richspace actually uses)

```rust
pub struct XfcePanelPlugin {
    pub container: gtk::Container,  // ← MUST be public
    // ... private fields
}

impl XfcePanelPlugin {
    // Construction
    pub fn from_raw(pointer: XfcePanelPluginPointer) -> Self;

    // Identity
    pub fn id(&self) -> String;
    pub fn name(&self) -> String;

    // Geometry
    pub fn orientation(&self) -> gtk::Orientation;
    pub fn size(&self) -> i32;

    // Menu
    pub fn add_action_widget(&self, widget: &impl IsA<gtk::Widget>);
    pub fn menu_show_configure(&self);

    // Config
    pub fn config_path(&self) -> Option<PathBuf>;

    // Signals
    pub fn connect_orientation_changed<F: Fn(gtk::Orientation) + 'static>(&self, f: F);
    pub fn connect_size_changed<F: Fn(i32) -> bool + 'static>(&self, f: F);
    pub fn connect_configure_plugin<F: Fn() + 'static>(&self, f: F);
    pub fn connect_save<F: Fn() + 'static>(&self, f: F);
    pub fn connect_free_data<F: Fn() + 'static>(&self, f: F);
}
```

### Optional API (declared but unused)

```rust
pub fn icon_size(&self) -> i32;
pub fn screen_position(&self) -> XfceScreenPosition;
pub fn mode(&self) -> XfcePanelPluginMode;
```

These can be **omitted** from xfce-panel-rs initially. Add them when a plugin actually needs them.

## Testing Requirements

### Behavioral Tests

1. **Construction:** `from_raw()` must not panic
2. **Identity:** `id()` format matches "name-uniqueid"
3. **Geometry:** `orientation()` and `size()` return panel values
4. **Menu:** `add_action_widget()` doesn't crash, `menu_show_configure()` shows entry
5. **Config:** `config_path()` returns valid path (or None)
6. **Signals:** All handlers fire when panel sends events

### Memory Tests

1. **No leaks:** `config_path()` frees g_malloc'd string
2. **No double-free:** `name()` doesn't free borrowed string
3. **No use-after-free:** Widget container remains valid after plugin setup

### Integration Tests

1. Panel loads plugin without crash
2. Log shows "richspace plugin constructor BEGIN/END"
3. Workspace buttons render
4. Clicking buttons switches workspaces
5. Panel resize triggers size-changed
6. Panel rotate triggers orientation-changed
7. Right-click shows "Configure"

## Migration Risk Assessment

**Risk Level: LOW**

**Reasons:**
1. Minimal FFI surface (7 functions, 5 signals)
2. No custom extensions or hacks
3. Standard memory management patterns
4. Well-isolated FFI module (easy to delete)
5. Comprehensive logging for debugging

**Potential Issues:**
1. `container` field must be public in xfce-panel-rs
2. `config_path()` .rc → .json conversion is custom (handle in app.rs if needed)
3. Signal trampolines must match exactly (bool return for size-changed)

**Mitigation:**
- Test with `just install && xfce4-panel -r`
- Monitor `journalctl -t richspace -f` for errors
- Verify each signal fires by adding trace logs

## Code Archaeology Notes

### Why are icon_size/mode/screen_position unused?

Likely copy-pasted from a more complete reference implementation (possibly windowck or tasklist), then simplified. The workspace switcher doesn't need:
- `icon_size` - draws custom workspace dots, not icons
- `mode` - only cares about orientation (horizontal/vertical)
- `screen_position` - no edge-specific logic

These were kept "just in case" but never needed.

### Why manual signal trampolines?

XFCE panel plugins are **not** GtkWidget subclasses. They're C extensions to GtkEventBox. The `container` is the actual GTK widget. All signals are emitted by the container, not the plugin pointer.

This is why:
```rust
self.container.connect_local("signal-name", ...)  // ✅ Works
// NOT:
glib::signal_handler_find(self.pointer, ...)      // ❌ Won't work
```

## Recommendations for xfce-panel-rs

### API Design

1. **Expose container as public field** (richspace requires direct access)
2. **Provide both raw and converted config paths** (let plugins choose .rc vs .json)
3. **Document memory management rules** (which strings need freeing)
4. **Test signal trampolines thoroughly** (easy to get wrong)

### Implementation Priority

1. Implement the **Minimum API** first (richspace uses these)
2. Add **Optional API** later (when another plugin needs it)
3. Consider **builder pattern** for signal connections (more ergonomic)

### Documentation

Document the gotchas:
- `id()` must be constructed (XFCE doesn't provide it)
- `size-changed` returns bool (others return void)
- `save_location()` returns owned string (must free)
- `get_name()` returns borrowed string (don't free)
- Signals emit from `container`, not `pointer`

## Appendix: Full grep Output

```bash
# FFI function usage
rg "xfce_panel_plugin_" src/

src/xfce/plugin.rs:30:        let unique_id = unsafe { ffi::xfce_panel_plugin_get_unique_id(self.pointer) };
src/xfce/plugin.rs:37:            let ptr = ffi::xfce_panel_plugin_get_name(self.pointer);
src/xfce/plugin.rs:48:            let raw = ffi::xfce_panel_plugin_get_orientation(self.pointer);
src/xfce/plugin.rs:59:        unsafe { ffi::xfce_panel_plugin_get_size(self.pointer) }
src/xfce/plugin.rs:64:        unsafe { ffi::xfce_panel_plugin_get_icon_size(self.pointer) }
src/xfce/plugin.rs:69:        unsafe { ffi::xfce_panel_plugin_get_screen_position(self.pointer) }
src/xfce/plugin.rs:74:        unsafe { ffi::xfce_panel_plugin_get_mode(self.pointer) }
src/xfce/plugin.rs:80:            ffi::xfce_panel_plugin_add_action_widget(
src/xfce/plugin.rs:89:            ffi::xfce_panel_plugin_menu_show_configure(self.pointer);
src/xfce/plugin.rs:97:            let ptr = ffi::xfce_panel_plugin_save_location(self.pointer, 1);
```

All other functions (screen_position, mode, icon_size) are called **only from plugin.rs wrappers**, never from app.rs.
