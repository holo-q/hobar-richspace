# xfce-panel-rs Requirements from richspace

This document specifies what xfce-panel-rs must provide for richspace to migrate successfully.

## Critical Requirements (blocking migration)

### 1. Public Container Field

**Current code:**
```rust
pub struct XfcePanelPlugin {
    pointer: XfcePanelPluginPointer,
    pub container: gtk::Container,  // ← MUST BE PUBLIC
}
```

**Usage in richspace:**
```rust
// src/app.rs:159 - Direct widget addition
plugin.container.add(widget.widget());

// src/app.rs:302 - Show all widgets
plugin.container.show_all();
```

**Why critical:** Richspace directly manipulates the container. If this is private, the code breaks.

**Alternative:** Provide wrapper methods:
```rust
impl XfcePanelPlugin {
    pub fn add_widget(&self, widget: &impl IsA<gtk::Widget>) {
        self.container.add(widget);
    }

    pub fn show_all(&self) {
        self.container.show_all();
    }
}
```

But this is worse ergonomics. **Prefer public field.**

---

### 2. Core Methods

All of these are **actually called** in app.rs:

```rust
impl XfcePanelPlugin {
    pub fn from_raw(pointer: XfcePanelPluginPointer) -> Self;
    pub fn id(&self) -> String;
    pub fn name(&self) -> String;
    pub fn orientation(&self) -> gtk::Orientation;
    pub fn size(&self) -> i32;
    pub fn add_action_widget(&self, widget: &impl IsA<gtk::Widget>);
    pub fn menu_show_configure(&self);
    pub fn config_path(&self) -> Option<PathBuf>;
}
```

**Details:**
- `id()` must return `format!("{}-{}", name, unique_id)` (XFCE doesn't provide this)
- `config_path()` calls `xfce_panel_plugin_save_location(ptr, 1)` with create=true
- `config_path()` must free the returned string with `g_free()`

---

### 3. Signal Handlers

All 5 signal handlers are **actually connected** in app.rs:

```rust
impl XfcePanelPlugin {
    pub fn connect_orientation_changed<F: Fn(gtk::Orientation) + 'static>(&self, f: F);
    pub fn connect_size_changed<F: Fn(i32) -> bool + 'static>(&self, f: F);
    pub fn connect_configure_plugin<F: Fn() + 'static>(&self, f: F);
    pub fn connect_save<F: Fn() + 'static>(&self, f: F);
    pub fn connect_free_data<F: Fn() + 'static>(&self, f: F);
}
```

**Critical detail:** `size-changed` returns `bool`, others return `()`.

**Implementation pattern:**
```rust
pub fn connect_size_changed<F: Fn(i32) -> bool + 'static>(&self, f: F) {
    self.container.connect_local("size-changed", false, move |values| {
        let size = values[1].get::<i32>().unwrap_or(24);
        Some(f(size).to_value())  // ← Must wrap in Some()
    });
}

pub fn connect_orientation_changed<F: Fn(gtk::Orientation) + 'static>(&self, f: F) {
    self.container.connect_local("orientation-changed", false, move |values| {
        let orientation = values[1].get::<gtk::Orientation>()
            .unwrap_or(gtk::Orientation::Horizontal);
        f(orientation);
        None  // ← Returns None for void signals
    });
}
```

---

## Optional Requirements (nice to have)

### Methods Declared But Unused

These are defined in richspace's FFI but **never called**:

```rust
pub fn icon_size(&self) -> i32;
pub fn screen_position(&self) -> XfceScreenPosition;
pub fn mode(&self) -> XfcePanelPluginMode;
```

**Recommendation:** Omit these from xfce-panel-rs initially. Add them when a plugin actually needs them (likely tasklist or windowck).

---

## Behavioral Requirements

### Memory Management

1. **xfce_panel_plugin_get_name()** returns **borrowed string** (don't free)
2. **xfce_panel_plugin_save_location()** returns **owned string** (must free with g_free)

**Implementation:**
```rust
pub fn name(&self) -> String {
    unsafe {
        let ptr = ffi::xfce_panel_plugin_get_name(self.pointer);
        if ptr.is_null() {
            return String::from("richspace");  // Fallback
        }
        CStr::from_ptr(ptr).to_string_lossy().into_owned()
        // NO g_free() - borrowed pointer
    }
}

pub fn config_path(&self) -> Option<PathBuf> {
    unsafe {
        let ptr = ffi::xfce_panel_plugin_save_location(self.pointer, 1);
        if ptr.is_null() {
            return None;
        }
        let path = CStr::from_ptr(ptr).to_string_lossy().into_owned();
        glib_sys::g_free(ptr as *mut _);  // ← REQUIRED
        Some(PathBuf::from(path))
    }
}
```

### Thread Safety

All methods are called from **GTK main thread only**. No sync required.

Signal handlers are `Fn` (not `FnMut`), closures can't mutate captured state unless wrapped in `Rc<RefCell<>>`.

---

## Type Requirements

### XfcePanelPluginPointer

```rust
pub type XfcePanelPluginPointer = *mut gtk_sys::GtkWidget;
```

Must be **publicly exported** for use in plugin.c shim.

### Enums

```rust
// Only if other plugins need them (richspace doesn't)
#[repr(C)]
pub enum XfceScreenPosition { ... }  // 16 variants

#[repr(C)]
pub enum XfcePanelPluginMode {       // 3 variants
    Horizontal,
    Vertical,
    Deskbar,
}
```

---

## Dependencies

xfce-panel-rs must depend on:

```toml
[dependencies]
gtk = { version = "0.18", features = ["v3_24"] }
glib = "0.18"
gtk-sys = "0.18"
glib-sys = "0.18"
libc = "0.2"

[build-dependencies]
pkg-config = "0.3"  # To find libxfce4panel-2.0
```

---

## Build System Requirements

### pkg-config

xfce-panel-rs must link against `libxfce4panel-2.0`:

```rust
// build.rs
fn main() {
    pkg_config::Config::new()
        .atleast_version("4.16.0")
        .probe("libxfce4panel-2.0")
        .unwrap();
}
```

### Cargo.toml

```toml
[package]
name = "xfce-panel-rs"
version = "0.1.0"
edition = "2021"
license = "GPL-2.0-or-later"  # Match XFCE panel license

[lib]
name = "xfce_panel_rs"

[dependencies]
gtk = { version = "0.18", features = ["v3_24"] }
glib = "0.18"
gtk-sys = "0.18"
glib-sys = "0.18"
libc = "0.2"

[build-dependencies]
pkg-config = "0.3"
```

---

## Testing Requirements

### Minimal Test Plugin

Create a test that exercises all APIs:

```rust
#[test]
fn test_minimal_plugin() {
    gtk::init().unwrap();

    // Mock pointer (in real plugin, this comes from C macro)
    let mock_ptr: XfcePanelPluginPointer = std::ptr::null_mut();

    // This will panic if APIs are missing
    let plugin = XfcePanelPlugin::from_raw(mock_ptr);
    let _ = plugin.id();
    let _ = plugin.name();
    let _ = plugin.orientation();
    let _ = plugin.size();
    let _ = plugin.config_path();

    // Signal connections (won't fire without real panel)
    plugin.connect_orientation_changed(|_| {});
    plugin.connect_size_changed(|_| true);
    plugin.connect_configure_plugin(|| {});
    plugin.connect_save(|| {});
    plugin.connect_free_data(|| {});
}
```

---

## Migration Validation Checklist

When xfce-panel-rs is ready, verify richspace can:

- [ ] Import `use xfce_panel_rs::{XfcePanelPlugin, XfcePanelPluginPointer};`
- [ ] Compile with `cargo build --release`
- [ ] Load in panel without crash
- [ ] Log startup message to journald
- [ ] Render workspace buttons
- [ ] Switch workspaces on click
- [ ] Respond to panel resize (size-changed signal)
- [ ] Respond to panel rotation (orientation-changed signal)
- [ ] Show "Configure" in right-click menu
- [ ] Save config on panel shutdown

---

## Comparison with Other Plugins

This plugin has **minimal FFI requirements**. Other plugins may need:

| Plugin | Extra FFI | Notes |
|--------|-----------|-------|
| richspace | **None** | Simplest possible usage |
| richmon | **None** | Similar to richspace |
| treasures | Maybe popup APIs? | Need to check |
| tasklist | Tooltip hooks, extended window API | Most complex |
| windowck | Window control APIs | Medium complexity |

**Strategy:** Build xfce-panel-rs to satisfy richspace first (smallest API surface). Then extend for tasklist (largest API surface).

---

## Gotchas for xfce-panel-rs Implementer

### 1. No xfce_panel_plugin_get_id()

XFCE doesn't provide a combined ID getter. Must construct from name + unique_id:

```rust
pub fn id(&self) -> String {
    format!("{}-{}", self.name(), self.unique_id())
}

fn unique_id(&self) -> i32 {
    unsafe { ffi::xfce_panel_plugin_get_unique_id(self.pointer) }
}
```

### 2. Signal Trampolines Emit from Container

Signals are **not** emitted by the plugin pointer. They're emitted by the GTK container:

```rust
// ✅ Correct
self.container.connect_local("signal-name", ...)

// ❌ Wrong
glib::signal_connect(self.pointer, "signal-name", ...)
```

### 3. size-changed Returns bool

The `size-changed` signal expects a `gboolean` return value:

```c
gboolean (*size_changed) (XfcePanelPlugin *plugin, gint size);
```

All other signals return void. The wrapper must handle this:

```rust
// size-changed: Fn(i32) -> bool
Some(f(size).to_value())

// orientation-changed: Fn(Orientation) -> ()
f(orientation);
None
```

### 4. LTO Must Be Disabled

Plugins linking xfce-panel-rs must set:

```toml
[profile.release]
lto = false
strip = false
```

Otherwise the C shim's `XFCE_PANEL_PLUGIN_REGISTER` macro symbols get stripped.

**Document this in xfce-panel-rs README.**

---

## Summary

**Minimum viable xfce-panel-rs for richspace:**

```rust
pub type XfcePanelPluginPointer = *mut gtk_sys::GtkWidget;

pub struct XfcePanelPlugin {
    pointer: XfcePanelPluginPointer,
    pub container: gtk::Container,  // ← PUBLIC
}

impl XfcePanelPlugin {
    pub fn from_raw(pointer: XfcePanelPluginPointer) -> Self;
    pub fn id(&self) -> String;
    pub fn name(&self) -> String;
    pub fn orientation(&self) -> gtk::Orientation;
    pub fn size(&self) -> i32;
    pub fn add_action_widget(&self, widget: &impl IsA<gtk::Widget>);
    pub fn menu_show_configure(&self);
    pub fn config_path(&self) -> Option<PathBuf>;

    pub fn connect_orientation_changed<F: Fn(gtk::Orientation) + 'static>(&self, f: F);
    pub fn connect_size_changed<F: Fn(i32) -> bool + 'static>(&self, f: F);
    pub fn connect_configure_plugin<F: Fn() + 'static>(&self, f: F);
    pub fn connect_save<F: Fn() + 'static>(&self, f: F);
    pub fn connect_free_data<F: Fn() + 'static>(&self, f: F);
}
```

**Lines of code:** ~200 (ffi.rs ~100, plugin.rs ~100)

**Complexity:** LOW - No custom extensions, standard memory management, well-understood signal patterns.
