//! Context menu for workspace button customization
//!
//! Provides right-click menu on workspace buttons to:
//! - Set custom label/icon (emoji, Nerd Font, Unicode)
//! - Clear customizations (revert to defaults)
//!
//! Design notes:
//! - Menu items trigger simple text input dialogs
//! - Callbacks use Rc<RefCell<>> pattern for state mutation
//! - Parent window reference allows modal dialog centering

use gtk::prelude::*;
use gtk::{Dialog, Entry, Menu, MenuItem, ResponseType, SeparatorMenuItem};

/// Build context menu for a workspace button
///
/// # Arguments
/// * `workspace_number` - Zero-indexed workspace number
/// * `current_label` - Current custom label (if any)
/// * `current_icon` - Current custom icon (if any)
/// * `on_label_change` - Callback when label is changed (None = clear)
/// * `on_icon_change` - Callback when icon is changed (None = clear)
/// * `on_clear` - Callback when all customizations are cleared
///
/// # Usage
/// ```no_run
/// let menu = build_workspace_menu(
///     0,
///     Some("Work"),
///     Some(""),
///     |label| { /* update label */ },
///     |icon| { /* update icon */ },
///     || { /* clear all */ },
/// );
/// menu.popup_at_pointer(None);
/// ```
pub fn build_workspace_menu(
    _workspace_number: i32,
    current_label: Option<String>,
    current_icon: Option<String>,
    on_label_change: impl Fn(Option<String>) + 'static,
    on_icon_change: impl Fn(Option<String>) + 'static,
    on_clear: impl Fn() + 'static,
) -> Menu {
    let menu = Menu::new();

    // "Set Label..." - opens text input dialog for custom workspace name
    let set_label = MenuItem::with_label("Set Label...");
    {
        let current = current_label.clone();
        set_label.connect_activate(move |_| {
            if let Some(text) = show_text_input_dialog(
                None, // No parent window (menu is transient)
                "Set Workspace Label",
                "Enter custom label (leave empty to clear):",
                current.as_deref(),
            ) {
                if text.trim().is_empty() {
                    on_label_change(None); // Clear label
                } else {
                    on_label_change(Some(text));
                }
            }
        });
    }

    // "Set Icon..." - opens text input dialog for emoji/nerd font/unicode
    let set_icon = MenuItem::with_label("Set Icon...");
    {
        let current = current_icon.clone();
        set_icon.connect_activate(move |_| {
            if let Some(text) = show_text_input_dialog(
                None,
                "Set Workspace Icon",
                "Enter icon (emoji, Nerd Font, Unicode):",
                current.as_deref(),
            ) {
                if text.trim().is_empty() {
                    on_icon_change(None); // Clear icon
                } else {
                    on_icon_change(Some(text));
                }
            }
        });
    }

    // Separator
    let sep = SeparatorMenuItem::new();

    // "Clear Customizations" - reset to defaults (no confirmation)
    let clear = MenuItem::with_label("Clear Customizations");
    clear.connect_activate(move |_| {
        on_clear();
    });

    // Assemble menu
    menu.append(&set_label);
    menu.append(&set_icon);
    menu.append(&sep);
    menu.append(&clear);
    menu.show_all();

    menu
}

/// Show a simple text input dialog
///
/// # Arguments
/// * `parent` - Parent window for modal positioning (can be None)
/// * `title` - Dialog title
/// * `prompt` - Prompt text displayed above input
/// * `initial_value` - Pre-filled text (if any)
///
/// # Returns
/// * `Some(String)` - User entered text and clicked OK
/// * `None` - User cancelled or closed dialog
///
/// # Design notes
/// - Modal dialog blocks until dismissed
/// - Entry widget pre-populated with initial_value
/// - Returns None if cancelled, Some("") if cleared by user
fn show_text_input_dialog(
    parent: Option<&gtk::Window>,
    title: &str,
    prompt: &str,
    initial_value: Option<&str>,
) -> Option<String> {
    // Create modal dialog with OK/Cancel buttons
    let dialog = Dialog::with_buttons(
        Some(title),
        parent,
        gtk::DialogFlags::MODAL | gtk::DialogFlags::DESTROY_WITH_PARENT,
        &[("Cancel", ResponseType::Cancel), ("OK", ResponseType::Ok)],
    );

    // Content area setup
    let content = dialog.content_area();
    content.set_spacing(12);
    content.set_margin_start(12);
    content.set_margin_end(12);
    content.set_margin_top(12);
    content.set_margin_bottom(12);

    // Prompt label
    let label = gtk::Label::new(Some(prompt));
    label.set_halign(gtk::Align::Start);
    content.pack_start(&label, false, false, 0);

    // Text entry widget
    let entry = Entry::new();
    entry.set_activates_default(true); // Enter key triggers OK
    if let Some(value) = initial_value {
        entry.set_text(value);
        // Select all text for easy replacement
        entry.select_region(0, -1);
    }
    content.pack_start(&entry, false, false, 0);

    // OK button as default (activated by Enter key)
    if let Some(ok_button) = dialog.widget_for_response(ResponseType::Ok) {
        ok_button.set_can_default(true);
        ok_button.grab_default();
    }

    // Show dialog
    content.show_all();
    entry.grab_focus(); // Focus entry immediately

    // Run modal dialog
    let response = dialog.run();
    let result = if response == ResponseType::Ok {
        Some(entry.text().to_string())
    } else {
        None
    };

    // Clean up
    dialog.close();

    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_menu_creation() {
        // GTK not initialized in tests, just verify compilation
        // Actual menu behavior tested via integration tests
    }
}
