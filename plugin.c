// Minimal C shim for XFCE4 panel plugin registration
// All logic lives in Rust - this only exists because
// XFCE_PANEL_PLUGIN_REGISTER is a C macro

#include <libxfce4panel/libxfce4panel.h>
#include "plugin.h"

XFCE_PANEL_PLUGIN_REGISTER(constructor);
