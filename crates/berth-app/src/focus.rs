//! Keyboard focus (P6-T4): what holds it, who may use bare keys while it
//! does, and where it goes when a dialog opens and closes.
//!
//! Dioxus `KeyboardData` carries no target element, so the page reports
//! focus itself: [`use_focus_tracking`] installs scripts that send what holds
//! focus on every change, and [`bare_keys_allowed`] is the one rule that turns
//! it into "may a bare key (Space, J, K, ⌫…) be a shortcut now". The root
//! handler (`keys.rs`) reads the answer through [`bare_keys_blocked`];
//! nothing else decides it.

use std::sync::atomic::{AtomicBool, Ordering};

use dioxus::prelude::*;

/// What the page says holds keyboard focus, in the five facts the rule below
/// needs. Parsed from the tab-separated line [`FOCUS_JS`] sends.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FocusInfo {
    /// Lower-case tag name of `document.activeElement` ("body", "button").
    pub tag: String,
    /// Its `role` attribute, empty when it has none.
    pub role: String,
    /// `isContentEditable`.
    pub editable: bool,
    /// Inside a CodeMirror editor (`.cm-editor`).
    pub in_editor: bool,
    /// Inside an xterm terminal (`.xterm`).
    pub in_terminal: bool,
}

impl FocusInfo {
    /// Focus on the page itself: nothing narrower holds it.
    #[must_use]
    pub fn body() -> Self {
        Self {
            tag: "body".to_string(),
            ..Self::default()
        }
    }

    /// Read the line [`FOCUS_JS`] sends: `tag \t role \t editable \t editor \t terminal`
    /// with the last three as `1` or `0`. A malformed line reads as a
    /// control, so a bare key stays out of the way.
    #[must_use]
    pub fn parse(wire: &str) -> Self {
        let mut parts = wire.split('\t');
        let (Some(tag), Some(role), Some(editable), Some(editor), Some(terminal)) = (
            parts.next(),
            parts.next(),
            parts.next(),
            parts.next(),
            parts.next(),
        ) else {
            return Self {
                tag: "button".to_string(),
                ..Self::default()
            };
        };
        Self {
            tag: tag.to_string(),
            role: role.to_string(),
            editable: editable == "1",
            in_editor: editor == "1",
            in_terminal: terminal == "1",
        }
    }
}

/// The one rule for bare-key shortcuts (Space, J, K, ⌫, F8…): they fire only
/// when focus is on the page, a list or a table. A button keeps Space (it
/// presses it), and anything you type into keeps every character. ⌘ chords do
/// not ask this; they work everywhere.
#[must_use]
pub fn bare_keys_allowed(focus: &FocusInfo) -> bool {
    if focus.editable || focus.in_editor || focus.in_terminal {
        return false;
    }
    if matches!(
        focus.tag.as_str(),
        "button" | "input" | "textarea" | "select" | "a" | "summary"
    ) {
        return false;
    }
    !matches!(
        focus.role.as_str(),
        "button"
            | "link"
            | "textbox"
            | "searchbox"
            | "combobox"
            | "switch"
            | "checkbox"
            | "radio"
            | "tab"
            | "menuitem"
            | "menuitemcheckbox"
            | "menuitemradio"
            | "slider"
            | "spinbutton"
    )
}

/// The last answer of [`bare_keys_allowed`]; the page reports every focus
/// change ([`use_focus_tracking`]). True until the page says otherwise.
static BARE_KEYS_ALLOWED: AtomicBool = AtomicBool::new(true);

/// Does a control hold focus right now, so bare keys belong to it?
#[must_use]
pub fn bare_keys_blocked() -> bool {
    !BARE_KEYS_ALLOWED.load(Ordering::Relaxed)
}

/// Runs in the page: reports what holds focus on every change, and hands focus
/// back to the shell root when it falls to the body (a dialog closed, a click
/// on empty space), since the root handler only hears keys from inside it.
pub const FOCUS_JS: &str = r#"
if (!window.__berthFocus) {
  window.__berthFocus = true;
  const report = () => {
    let el = document.activeElement;
    if (!el || el === document.body) {
      const dialogs = document.querySelectorAll('.overlay-scrim [role="dialog"]');
      const home = dialogs.length ? dialogs[dialogs.length - 1] : document.querySelector('.shell');
      if (home) {
        if (!home.hasAttribute('tabindex')) home.setAttribute('tabindex', '-1');
        home.focus({ preventScroll: true });
      }
      el = document.activeElement || document.body;
    }
    const flag = (value) => (value ? '1' : '0');
    dioxus.send([
      el.tagName.toLowerCase(),
      el.getAttribute('role') || '',
      flag(el.isContentEditable),
      flag(el.closest('.cm-editor')),
      flag(el.closest('.xterm')),
    ].join('\t'));
  };
  document.addEventListener('focusin', report, true);
  document.addEventListener('focusout', () => setTimeout(report, 0), true);
  report();
}
"#;

/// Runs in the page: Esc closes an open popup menu (More, Connect, the engine
/// list) by clicking what dismisses it with the mouse: its backdrop, else the
/// button that opened it, else the Containers screen (it closes the row and
/// group menus on any click). Overlays and sheets are not touched: with one
/// open the shell's Esc closes it (`overlay.close`).
pub const POPUP_ESCAPE_JS: &str = r#"
if (!window.__berthPopupEscape) {
  window.__berthPopupEscape = true;
  document.addEventListener('keydown', (e) => {
    if (e.key !== 'Escape' || e.defaultPrevented) return;
    if (document.querySelector('.overlay-scrim')) return;
    const menu = document.querySelector('[role="menu"]');
    if (!menu) return;
    let dismiss = document.querySelector('[class$="-backdrop"], [class$="-scrim"]');
    for (let up = menu.parentElement; !dismiss && up && up !== document.body; up = up.parentElement) {
      dismiss = up.querySelector('button[aria-pressed="true"]');
    }
    dismiss = dismiss || document.querySelector('.containers-screen');
    if (!dismiss) return;
    e.preventDefault();
    e.stopPropagation();
    dismiss.click();
  }, true);
}
"#;

/// Runs in the page: a modal dialog takes focus when it opens (the first
/// field, else the dialog itself, which a screen reader announces by its
/// label), keeps Tab inside it, and gives focus back to what opened it.
pub const DIALOG_FOCUS_JS: &str = r#"
if (!window.__berthDialogFocus) {
  window.__berthDialogFocus = true;
  const topDialog = () => {
    const all = document.querySelectorAll('.overlay-scrim [role="dialog"]');
    return all.length ? all[all.length - 1] : null;
  };
  const tabbable = (root) => Array.from(root.querySelectorAll(
    'button, [href], input, select, textarea, [tabindex]:not([tabindex="-1"])'
  )).filter((el) => !el.disabled && el.type !== 'hidden' && el.getClientRects().length > 0);
  let current = null;
  let opener = null;
  const sync = () => {
    const dialog = topDialog();
    if (dialog === current) return;
    const previous = current;
    current = dialog;
    if (dialog) {
      if (!previous) opener = document.activeElement;
      if (!dialog.contains(document.activeElement)) {
        const field = dialog.querySelector('input:not([type="hidden"]), textarea, select');
        if (field) {
          field.focus({ preventScroll: true });
        } else {
          if (!dialog.hasAttribute('tabindex')) dialog.setAttribute('tabindex', '-1');
          dialog.focus({ preventScroll: true });
        }
      }
    } else if (previous) {
      const back = opener && opener.isConnected && opener !== document.body
        ? opener : document.querySelector('.shell');
      opener = null;
      if (back) back.focus({ preventScroll: true });
    }
  };
  document.addEventListener('keydown', (e) => {
    if (e.key !== 'Tab' || !current) return;
    const items = tabbable(current);
    if (!items.length) { e.preventDefault(); return; }
    const first = items[0];
    const last = items[items.length - 1];
    const active = document.activeElement;
    if (!current.contains(active) || active === current) {
      e.preventDefault();
      (e.shiftKey ? last : first).focus();
    } else if (e.shiftKey && active === first) {
      e.preventDefault();
      last.focus();
    } else if (!e.shiftKey && active === last) {
      e.preventDefault();
      first.focus();
    }
  }, true);
  const watch = () => {
    const shell = document.querySelector('.shell');
    if (!shell) { setTimeout(watch, 50); return; }
    new MutationObserver(sync).observe(shell, { childList: true });
    sync();
  };
  watch();
}
"#;

/// Start the focus report, the popup Esc and the dialog focus for the shell.
/// Call once, from the shell root.
pub fn use_focus_tracking() {
    use_future(move || async move {
        let _ = document::eval(POPUP_ESCAPE_JS);
        let _ = document::eval(DIALOG_FOCUS_JS);
        let mut eval = document::eval(FOCUS_JS);
        while let Ok(wire) = eval.recv::<String>().await {
            let allowed = bare_keys_allowed(&FocusInfo::parse(&wire));
            BARE_KEYS_ALLOWED.store(allowed, Ordering::Relaxed);
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn focus(tag: &str, role: &str) -> FocusInfo {
        FocusInfo {
            tag: tag.to_string(),
            role: role.to_string(),
            ..FocusInfo::default()
        }
    }

    #[test]
    fn bare_keys_fire_on_the_page_lists_and_tables_only() {
        // The body, the shell root and the things a list is made of.
        for allowed in [
            FocusInfo::body(),
            focus("div", ""),
            focus("div", "row"),
            focus("div", "option"),
            focus("div", "listbox"),
            focus("div", "table"),
            focus("li", "treeitem"),
        ] {
            assert!(bare_keys_allowed(&allowed), "{allowed:?}");
        }
        // Everything that takes a key for itself.
        for held in [
            focus("button", ""),
            focus("input", ""),
            focus("textarea", ""),
            focus("select", ""),
            focus("a", ""),
            focus("summary", ""),
            focus("div", "button"),
            focus("div", "switch"),
            focus("div", "checkbox"),
            focus("div", "radio"),
            focus("div", "tab"),
            focus("div", "menuitem"),
            focus("div", "textbox"),
            focus("div", "combobox"),
            FocusInfo {
                editable: true,
                ..focus("div", "")
            },
            FocusInfo {
                in_editor: true,
                ..focus("div", "")
            },
            FocusInfo {
                in_terminal: true,
                ..focus("textarea", "")
            },
            FocusInfo {
                in_terminal: true,
                ..focus("div", "")
            },
        ] {
            assert!(!bare_keys_allowed(&held), "{held:?}");
        }
    }

    #[test]
    fn focus_info_reads_the_pages_line_and_distrusts_a_broken_one() {
        let wire = "div\trow\t0\t0\t0";
        assert_eq!(FocusInfo::parse(wire), focus("div", "row"));
        assert_eq!(
            FocusInfo::parse("div\t\t0\t1\t0"),
            FocusInfo {
                in_editor: true,
                ..focus("div", "")
            }
        );
        assert_eq!(
            FocusInfo::parse("textarea\t\t0\t0\t1"),
            FocusInfo {
                in_terminal: true,
                ..focus("textarea", "")
            }
        );
        assert!(FocusInfo::parse("div\t\t1\t0\t0").editable);
        // A line that is not ours keeps bare keys out of the way.
        assert!(!bare_keys_allowed(&FocusInfo::parse("garbage")));
    }

    #[test]
    fn the_page_scripts_use_the_classes_the_markup_has() {
        // The scripts find overlays and the shell by class and role; a rename
        // in the markup would silently turn them off.
        for class in ["overlay-scrim", "shell"] {
            assert!(
                DIALOG_FOCUS_JS.contains(class) || FOCUS_JS.contains(class),
                "{class}"
            );
        }
        let html = crate::scenes::render_scene_html("Containers / confirm delete / 1440")
            .expect("scene renders");
        assert!(html.contains("class=\"shell\""), "{html}");
        assert!(html.contains("overlay-scrim"), "{html}");
        assert!(html.contains("role=\"dialog\""), "{html}");
        assert!(html.contains("containers-screen"), "{html}");
    }
}
