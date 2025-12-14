//! XFCE4 panel plugin bindings and wrappers

pub mod ffi;
pub mod plugin;

pub use ffi::XfcePanelPluginPointer;
pub use plugin::XfcePanelPlugin;
