# richspace Migration Patch

This document contains the exact changes needed to migrate from local FFI to xfce-panel-rs.

**Prerequisites:** xfce-panel-rs must exist at `~/Workspace/Lib/xfce-panel-rs/`

## Step 1: Update Cargo.toml

```diff
--- a/Cargo.toml
+++ b/Cargo.toml
@@ -12,6 +12,9 @@ name = "richspace"
 crate-type = ["staticlib", "cdylib"]

 [dependencies]
+# XFCE panel plugin bindings (shared across all panel plugins)
+xfce-panel-rs = { path = "../../Lib/xfce-panel-rs" }
+
 # GTK3 bindings
 gtk = { version = "0.18", features = ["v3_24"] }
 glib = "0.18"
@@ -19,12 +22,6 @@ gdk = "0.18"
 pango = "0.18"
 cairo-rs = "0.18"

-# Low-level GTK sys crates for FFI
-gtk-sys = "0.18"
-glib-sys = "0.18"
-gobject-sys = "0.18"
-gdk-sys = "0.18"
-
 # libwnck bindings (shared crate)
 wnck-rs = { path = "../../Lib/wnck-rs" }

@@ -35,9 +32,6 @@ notify = "6"
 # Async runtime for provider IPC (unix socket listener)
 tokio = { version = "1", features = ["rt", "net", "io-util", "sync", "fs"] }

-# FFI utilities
-libc = "0.2"
-
 # Error handling
 thiserror = "1"
 anyhow = "1"
```

## Step 2: Update src/lib.rs

```diff
--- a/src/lib.rs
+++ b/src/lib.rs
@@ -19,15 +19,14 @@
 //! - INFO: Lifecycle events (start, stop, reload)
 //! - DEBUG: Event flow, state changes
 //! - TRACE: Hot path details (render, animation ticks)

-mod xfce;
 mod wnck;
 mod config;
 mod state;
 mod app;
 mod ui;
 mod providers;

-use xfce::{XfcePanelPluginPointer, XfcePanelPlugin};
+use xfce_panel_rs::{XfcePanelPluginPointer, XfcePanelPlugin};

 /// Entry point called by XFCE panel via C shim
 ///
```

## Step 3: Update src/app.rs

```diff
--- a/src/app.rs
+++ b/src/app.rs
@@ -7,7 +7,7 @@ use gtk::prelude::*;
 use std::cell::RefCell;
 use std::rc::Rc;

-use crate::xfce::XfcePanelPlugin;
+use xfce_panel_rs::XfcePanelPlugin;
 use crate::config::Config;
 use crate::state::State;
 use crate::wnck::{self, WorkspaceInfo};
```

## Step 4: Delete Local FFI

```bash
rm -rf src/xfce/
```

Files to delete:
- `src/xfce/ffi.rs` (67 lines)
- `src/xfce/plugin.rs` (150 lines)
- `src/xfce/mod.rs` (8 lines)

## Step 5: Handle Potential API Differences

### If xfce-panel-rs doesn't convert .rc to .json

The current `config_path()` implementation converts `.rc` to `.json`:

```rust
// Old code in src/xfce/plugin.rs:104
let path = path.replace(".rc", ".json");
```

If xfce-panel-rs returns the raw `.rc` path, add this in `src/app.rs`:

```diff
--- a/src/app.rs
+++ b/src/app.rs
@@ -102,7 +102,10 @@ impl App {

         // Load persistent config
         tracing::debug!("loading config");
-        let config_path = plugin.config_path();
+        let config_path = plugin.config_path().map(|p| {
+            // richspace uses JSON format, XFCE returns .rc path
+            PathBuf::from(p.to_string_lossy().replace(".rc", ".json"))
+        });
         let config = config_path
             .as_ref()
             .and_then(|p| {
```

### If xfce-panel-rs has a different container field

The current code assumes `plugin.container` is public:

```rust
// src/app.rs:159
plugin.container.add(widget.widget());

// src/app.rs:302
plugin.container.show_all();
```

If xfce-panel-rs exposes this as `plugin.widget()` or `plugin.as_container()`, update accordingly.

## Verification Script

```bash
#!/bin/bash
set -e

echo "=== Verifying xfce-panel-rs exists ==="
test -d ../../Lib/xfce-panel-rs || {
    echo "ERROR: xfce-panel-rs not found"
    exit 1
}

echo "=== Building richspace ==="
cargo build --release 2>&1 | tee build.log

echo "=== Checking for FFI symbols ==="
if grep -q "xfce_panel_rs" build.log; then
    echo "✅ Using xfce-panel-rs"
else
    echo "⚠️  xfce-panel-rs not detected in build"
fi

echo "=== Installing ==="
just install

echo "=== Restarting panel ==="
xfce4-panel -r

echo "=== Checking logs (5 seconds) ==="
timeout 5 journalctl -t richspace -f || true

echo ""
echo "✅ Migration complete!"
echo "Monitor logs with: journalctl -t richspace -f"
```

Save as `verify-migration.sh` and run with `bash verify-migration.sh`.

## Rollback Plan

If migration fails:

```bash
# Restore from git
git checkout -- Cargo.toml src/lib.rs src/app.rs
git checkout src/xfce/

# Rebuild with original FFI
cargo build --release
just install
xfce4-panel -r
```

## Expected Build Output

### Before Migration
```
Compiling xfce4-panel-richspace v0.1.0
  - Using local FFI (src/xfce/ffi.rs)
  - Direct gtk-sys/glib-sys dependencies
```

### After Migration
```
Compiling xfce-panel-rs v0.1.0
Compiling xfce4-panel-richspace v0.1.0
  - Using shared xfce-panel-rs crate
  - No direct sys dependencies
```

## Success Criteria

- [ ] `cargo build --release` succeeds
- [ ] No warnings about missing symbols
- [ ] `just install` completes
- [ ] Panel restarts without crash
- [ ] `journalctl -t richspace -f` shows startup logs
- [ ] Workspace buttons render correctly
- [ ] Clicking workspace switches desktop
- [ ] Panel resize triggers re-render
- [ ] Right-click menu shows "Configure"
- [ ] Config hot-reload still works

## Regression Testing

```bash
# Test 1: Plugin loads
journalctl -t richspace --since "1 minute ago" | grep "constructor BEGIN"

# Test 2: Workspace switching
xdotool set_desktop 2
sleep 0.5
xdotool set_desktop 0

# Test 3: Panel resize
xfconf-query -c xfce4-panel -p /panels/panel-1/size -s 48

# Test 4: Config reload
touch ~/.config/xfce4/panel/richspace-*.json
sleep 0.5
journalctl -t richspace --since "5 seconds ago" | grep "config hot-reloaded"
```

## Estimated Time

- **Code changes:** 5 minutes (3 files, ~10 lines total)
- **Build/test:** 2 minutes
- **Verification:** 3 minutes
- **Total:** ~10 minutes

This is one of the fastest migrations due to minimal FFI usage.
