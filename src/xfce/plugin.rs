//! Safe wrapper around XfcePanelPlugin

use std::ffi::CStr;
use gtk::prelude::*;
use glib::translate::*;

use super::ffi::{self, XfcePanelPluginPointer, XfceScreenPosition, XfcePanelPluginMode};

/// Safe wrapper around XfcePanelPlugin
pub struct XfcePanelPlugin {
    pointer: XfcePanelPluginPointer,
    pub container: gtk::Container,
}

#[allow(dead_code)]
impl XfcePanelPlugin {
    /// Create from raw pointer (called from constructor)
    pub fn from_raw(pointer: XfcePanelPluginPointer) -> Self {
        let container: gtk::Container = unsafe {
            gtk::Widget::from_glib_none(pointer).downcast().unwrap()
        };

        XfcePanelPlugin { pointer, container }
    }

    /// Get plugin unique ID (e.g., "richspace-1")
    pub fn id(&self) -> String {
        unsafe {
            let ptr = ffi::xfce_panel_plugin_get_id(self.pointer);
            if ptr.is_null() {
                return String::from("unknown");
            }
            CStr::from_ptr(ptr).to_string_lossy().into_owned()
        }
    }

    /// Get plugin name
    pub fn name(&self) -> String {
        unsafe {
            let ptr = ffi::xfce_panel_plugin_get_name(self.pointer);
            if ptr.is_null() {
                return String::from("richspace");
            }
            CStr::from_ptr(ptr).to_string_lossy().into_owned()
        }
    }

    /// Get panel orientation
    pub fn orientation(&self) -> gtk::Orientation {
        unsafe {
            let raw = ffi::xfce_panel_plugin_get_orientation(self.pointer);
            if raw == gtk_sys::GTK_ORIENTATION_VERTICAL {
                gtk::Orientation::Vertical
            } else {
                gtk::Orientation::Horizontal
            }
        }
    }

    /// Get panel size in pixels
    pub fn size(&self) -> i32 {
        unsafe { ffi::xfce_panel_plugin_get_size(self.pointer) }
    }

    /// Get icon size for this panel
    pub fn icon_size(&self) -> i32 {
        unsafe { ffi::xfce_panel_plugin_get_icon_size(self.pointer) }
    }

    /// Get screen position
    pub fn screen_position(&self) -> XfceScreenPosition {
        unsafe { ffi::xfce_panel_plugin_get_screen_position(self.pointer) }
    }

    /// Get plugin mode
    pub fn mode(&self) -> XfcePanelPluginMode {
        unsafe { ffi::xfce_panel_plugin_get_mode(self.pointer) }
    }

    /// Register a widget for action menu (right-click)
    pub fn add_action_widget(&self, widget: &impl IsA<gtk::Widget>) {
        unsafe {
            ffi::xfce_panel_plugin_add_action_widget(
                self.pointer,
                widget.as_ref().to_glib_none().0,
            );
        }
    }

    /// Show the configure dialog entry in menu
    pub fn menu_show_configure(&self) {
        unsafe {
            ffi::xfce_panel_plugin_menu_show_configure(self.pointer);
        }
    }

    /// Get config file save location
    pub fn config_path(&self) -> Option<std::path::PathBuf> {
        unsafe {
            let ptr = ffi::xfce_panel_plugin_save_location(self.pointer, 1);
            if ptr.is_null() {
                return None;
            }
            let path = CStr::from_ptr(ptr).to_string_lossy().into_owned();
            glib_sys::g_free(ptr as *mut _);
            // Convert .rc path to .json
            let path = path.replace(".rc", ".json");
            Some(std::path::PathBuf::from(path))
        }
    }

    /// Connect to orientation-changed signal
    pub fn connect_orientation_changed<F: Fn(gtk::Orientation) + 'static>(&self, f: F) {
        self.container.connect_local("orientation-changed", false, move |values| {
            let orientation = values[1].get::<gtk::Orientation>().unwrap_or(gtk::Orientation::Horizontal);
            f(orientation);
            None
        });
    }

    /// Connect to size-changed signal
    pub fn connect_size_changed<F: Fn(i32) -> bool + 'static>(&self, f: F) {
        self.container.connect_local("size-changed", false, move |values| {
            let size = values[1].get::<i32>().unwrap_or(24);
            Some(f(size).to_value())
        });
    }

    /// Connect to free-data signal (cleanup)
    pub fn connect_free_data<F: Fn() + 'static>(&self, f: F) {
        self.container.connect_local("free-data", false, move |_| {
            f();
            None
        });
    }

    /// Connect to save signal
    pub fn connect_save<F: Fn() + 'static>(&self, f: F) {
        self.container.connect_local("save", false, move |_| {
            f();
            None
        });
    }

    /// Connect to configure-plugin signal
    pub fn connect_configure_plugin<F: Fn() + 'static>(&self, f: F) {
        self.container.connect_local("configure-plugin", false, move |_| {
            f();
            None
        });
    }
}
