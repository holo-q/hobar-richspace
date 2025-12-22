# GTK3 Widget Rebuild Performance Issue - Reproduction Case

## Problem Statement

An XFCE panel workspace indicator widget takes **40-80ms** to render 12 simple buttons. This is unacceptably slow for a UI element that should update at 60fps.

## CRITICAL FINDING: Standalone vs Panel Plugin

| Context | Render Time |
|---------|-------------|
| **Standalone GTK app** (this repro) | 5-15ms |
| **XFCE panel plugin** (real widget) | 40-80ms |

**The slowness is NOT inherent to GTK3 widget creation.**
Something in the XFCE panel plugin environment adds 30-65ms of overhead.

### Possible Environmental Factors

1. **XFCE panel process** - Plugin runs inside xfce4-panel via dlopen()
2. **Shared GTK main loop** - Multiple plugins share the same event loop
3. **Screen-wide CSS provider** - Real widget calls `add_provider_for_screen()`
4. **libwnck integration** - Real widget queries workspace state via wnck
5. **Provider IPC** - Tokio runtime + Unix socket + JSON parsing
6. **tracing → journald** - Logging overhead in hot path
7. **DrawingArea closures** - May capture complex state from babel provider

## Environment

| Component | Version |
|-----------|---------|
| OS | Arch Linux (CachyOS kernel 6.12.49) |
| Desktop | XFCE 4.20.5 with xfwm4 |
| GTK | 3.24.x |
| Display | X11, 2560x1440 @ 144Hz |
| CPU | Modern multi-core (not the bottleneck) |

The actual widget runs as an **XFCE panel plugin** (Rust staticlib linked with C shim, dlopen'd by xfce4-panel).

## Widget Architecture

```
┌─────────────────────────────────────────────────────────────────┐
│ EventBox (for scroll events)                                    │
│ ┌───────────────────────────────────────────────────────────────┐
│ │ GtkBox (container)                                            │
│ ├───────────────────────────────────────────────────────────────┤
│ │ ┌─────────┐ ┌─────────┐ ┌─────────┐     ┌─────────┐          │
│ │ │ Button  │ │ Button  │ │ Button  │ ... │ Button  │ (12)     │
│ │ │ ┌─────┐ │ │ ┌─────┐ │ │ ┌─────┐ │     │ ┌─────┐ │          │
│ │ │ │Icon │ │ │ │Icon │ │ │ │Icon │ │     │ │Icon │ │          │
│ │ │ │Label│ │ │ │Label│ │ │ │Label│ │     │ │Label│ │          │
│ │ │ │Badge│ │ │ │     │ │ │ │Badge│ │     │ │     │ │          │
│ │ │ │Dots?│ │ │ │     │ │ │ │Dots │ │     │ │     │ │          │
│ │ │ └─────┘ │ │ └─────┘ │ │ └─────┘ │     │ └─────┘ │          │
│ │ └─────────┘ └─────────┘ └─────────┘     └─────────┘          │
│ └───────────────────────────────────────────────────────────────┘
└─────────────────────────────────────────────────────────────────┘
```

Each button contains:
- **Icon**: GtkLabel with emoji/nerd font icon (`set_use_markup(true)`)
- **Label**: GtkLabel with workspace number (`set_use_markup(true)`)
- **Badge** (optional): Window count badge label
- **Dots** (optional): GtkDrawingArea for provider status indicators

Each button also has:
- **Tooltip**: `set_tooltip_text()`
- **Click handler**: `connect_clicked()` with closure capturing `glib::Sender`
- **Right-click handler**: `connect_button_press_event()` with closure for context menu
- **CSS classes**: "richspace-button", "active", "has-windows"/"empty", "urgent"
- **CSS provider**: `add_provider()` on each widget's StyleContext

## Render Trigger Sources

1. **Workspace changes** (wnck signals): ~1-5 per second during active use
2. **Provider IPC** (external daemon): Sends render state at 60fps, throttled to 5fps (200ms)
3. **Config file changes**: Rare, on user edit

## Current Render Implementation (Pseudocode)

```rust
fn render(&self, workspaces: &[WorkspaceInfo]) {
    // 1. Destroy all existing buttons
    for child in container.children() {
        container.remove(&child);
    }

    // 2. Create new buttons from scratch
    for ws in workspaces {
        let button = Button::new();
        button.style_context().add_provider(&css_provider, PRIORITY_USER);
        button.style_context().add_class("workspace-button");
        if ws.is_active { button.style_context().add_class("active"); }

        let content = Box::new(...);
        let icon = Label::new("●");
        icon.style_context().add_provider(&css_provider, PRIORITY_USER);
        icon.style_context().add_class("workspace-icon");

        let label = Label::new(&ws.number.to_string());
        // ... more style context operations

        if has_dots {
            let drawing_area = DrawingArea::new();
            drawing_area.connect_draw(|_, ctx| { ... });
            content.pack_end(&drawing_area, ...);
        }

        button.add(&content);
        container.pack_start(&button, ...);
    }

    // 3. Show all
    container.show_all();
}
```

## Timing Breakdown (Typical)

| Phase | Time |
|-------|------|
| Clear (remove children) | 1-5ms |
| Create (12 buttons) | 30-60ms |
| Show all | 5-15ms |
| **Total** | **40-80ms** |

## What We've Tried

### 1. Removed CSS regeneration from hot path
- Previously: `apply_default_css()` was called every render
- Now: CSS only regenerated on config change
- **Result**: Minor improvement (~5ms saved)

### 2. Throttled provider updates to 5fps
- Provider sends 60fps, but render limited to 200ms intervals
- **Result**: Fewer renders, but each still takes 40-80ms

### 3. Added tracing instrumentation
- Confirmed the slowness is in widget creation, not our logic
- Style context operations appear frequently in traces

### 4. Profiled with perf (briefly)
- GTK internals dominate: `gtk_style_context_*`, `gtk_widget_realize`
- Suggests widget creation overhead, not our code

## Hypotheses

1. **StyleContext operations are expensive**
   - Each `add_class()` and `add_provider()` may trigger CSS recalculation
   - 12 buttons × 3 widgets × 2 operations = 72 style operations

2. **Widget realization has overhead**
   - Each new widget goes through realize/map/size-allocate
   - DrawingArea closures may have setup cost

3. **GObject/GType overhead**
   - Each `Widget::new()` involves GObject construction
   - Reference counting, signal setup, etc.

4. **The screen-wide CSS provider**
   - `add_provider_for_screen()` might cause global recalculation

## Questions for the Researcher

Given that standalone GTK runs at 5-15ms but panel plugin runs at 40-80ms:

1. **What's causing the 30-65ms environmental overhead?**
   - Is it xfce4-panel's main loop processing?
   - CSS cascade recalculation with screen-wide providers?
   - libwnck signal handling interleaved with our render?
   - Tokio runtime contention?

2. **How to diagnose the overhead?**
   - Can we run perf/sysprof on the panel plugin specifically?
   - What GTK_DEBUG flags would reveal the bottleneck?
   - How to profile inside a dlopen'd .so?

3. **Should we avoid screen-wide CSS?**
   - Real widget calls `add_provider_for_screen()` + per-widget `add_provider()`
   - Could this cause O(n) cascade recalculation across all panel plugins?

4. **Is there panel plugin best practice we're violating?**
   - Are we blocking the panel's main loop inappropriately?
   - Should widget updates be deferred/batched differently?

## Suggested Diagnostics

```bash
# 1. Run panel with GTK debugging
GTK_DEBUG=actions,geometry xfce4-panel --replace 2>&1 | tee panel-debug.log

# 2. Profile the panel process
perf record -g -p $(pgrep xfce4-panel) -- sleep 10
perf report

# 3. Check for excessive style recalculation
GTK_DEBUG=css xfce4-panel --replace 2>&1 | grep -i richspace

# 4. Measure without tracing overhead (rebuild with tracing disabled)
# In Cargo.toml: tracing = { version = "...", features = [] }

# 5. Measure without provider IPC (disconnect babel)
systemctl --user stop richspace-babel

# 6. Measure without libwnck (comment out wnck signal handlers)
```

## Target Performance

- **Goal**: <5ms per render (enables smooth 60fps if needed)
- **Acceptable**: <16ms per render (60fps capable)
- **Current**: 40-80ms per render (12-25fps max, visible lag)

## Running the Reproduction

```bash
cd repro
cargo run --release
```

The window shows:
- The workspace widget (12 buttons)
- Timing breakdown for each render
- Auto-render every 200ms (simulating provider throttle)
- Manual "Trigger Render" button

Watch the console for per-render timing output.
