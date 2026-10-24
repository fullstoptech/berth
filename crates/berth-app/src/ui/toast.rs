//! Toasts (PLAN D34, tokens `--dwell-toast*`): 5 s, or 10 s when the toast
//! carries an action (Undo/View). Only for the user's own non-routine
//! actions. Timing is a pure helper so SSR renders stay deterministic.

use std::time::Duration;

use dioxus::prelude::*;

use crate::state::{AppState, Toast, ToastTone};

/// How long a toast stays up: 5 s plain, 10 s with an action.
pub fn toast_timeout(toast: &Toast) -> Duration {
    if toast.action.is_some() {
        Duration::from_secs(10)
    } else {
        Duration::from_secs(5)
    }
}

/// Remove the toast with `id` from a list. Pure; the signal writes wrap it.
pub fn dismiss_from(toasts: &mut Vec<Toast>, id: uuid::Uuid) {
    toasts.retain(|t| t.id != id);
}

/// Remove one toast from `AppState.toasts`.
pub fn dismiss_toast(mut toasts: Signal<Vec<Toast>>, id: uuid::Uuid) {
    dismiss_from(&mut toasts.write(), id);
}

pub fn tone_class(tone: ToastTone) -> &'static str {
    match tone {
        ToastTone::Neutral => "tone-neutral",
        ToastTone::Success => "tone-success",
        ToastTone::Danger => "tone-danger",
    }
}

pub fn tone_icon(tone: ToastTone) -> &'static str {
    match tone {
        ToastTone::Neutral => "info",
        ToastTone::Success => "circle-check",
        ToastTone::Danger => "alert-circle",
    }
}

/// One toast. Its auto-dismiss timer starts on mount (effect + spawned
/// delay); SSR never runs effects, so scenes render every toast.
#[component]
fn ToastItem(toast: Toast, onaction: Option<EventHandler<&'static str>>) -> Element {
    let state = use_context::<AppState>();
    let id = toast.id;
    let timeout = toast_timeout(&toast);
    use_effect(move || {
        let mut toasts = state.toasts;
        spawn(async move {
            let ms = timeout.as_millis();
            let script = format!("await new Promise(r => setTimeout(r, {ms}));");
            let _ = document::eval(&script).await;
            dismiss_from(&mut toasts.write(), id);
        });
    });

    let tone = tone_class(toast.tone);
    let icon = tone_icon(toast.tone);
    let dismiss_label = format!("Dismiss: {}", toast.text);
    rsx! {
        div {
            class: "toast {tone}",
            role: "status",
            svg { class: "toast-icon", "aria-hidden": "true",
                r#use { href: "/icons.svg#{icon}" }
            }
            span { class: "toast-text", "{toast.text}" }
            if let Some((label, action_id)) = toast.action.clone() {
                button {
                    class: "toast-action",
                    r#type: "button",
                    onclick: move |_| {
                        if let Some(handler) = onaction {
                            handler.call(action_id);
                        }
                        dismiss_toast(state.toasts, id);
                    },
                    "{label}"
                }
            }
            button {
                class: "toast-dismiss",
                r#type: "button",
                "aria-label": "{dismiss_label}",
                onclick: move |_| dismiss_toast(state.toasts, id),
                svg { "aria-hidden": "true",
                    r#use { href: "/icons.svg#x" }
                }
            }
        }
    }
}

/// The stack of live toasts, read from `AppState.toasts` (fixed bottom-right).
#[component]
pub fn ToastStack(onaction: Option<EventHandler<&'static str>>) -> Element {
    let state = use_context::<AppState>();
    let toasts = state.toasts.read().clone();
    rsx! {
        div { class: "toast-stack", "aria-live": "polite",
            for toast in toasts {
                ToastItem { key: "{toast.id}", toast: toast.clone(), onaction }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::use_app_state;

    fn toast(text: &str, tone: ToastTone, action: Option<(String, &'static str)>) -> Toast {
        Toast {
            id: uuid::Uuid::now_v7(),
            text: text.to_string(),
            tone,
            action,
        }
    }

    #[test]
    fn feedback_toast_timeout_plain_is_five_seconds() {
        let t = toast("Removed 3 items", ToastTone::Success, None);
        assert_eq!(toast_timeout(&t), Duration::from_secs(5));
    }

    #[test]
    fn feedback_toast_timeout_with_action_is_ten_seconds() {
        let t = toast(
            "Removed 3 items",
            ToastTone::Success,
            Some(("Undo".to_string(), "undo")),
        );
        assert_eq!(toast_timeout(&t), Duration::from_secs(10));
    }

    #[test]
    fn feedback_toast_dismiss_removes_only_the_named_toast() {
        let a = toast("A", ToastTone::Neutral, None);
        let b = toast("B", ToastTone::Danger, None);
        let mut toasts = vec![a.clone(), b.clone()];
        dismiss_from(&mut toasts, a.id);
        assert_eq!(toasts, vec![b.clone()]);
        dismiss_from(&mut toasts, uuid::Uuid::now_v7());
        assert_eq!(toasts, vec![b], "unknown id is a no-op");
    }

    #[test]
    fn feedback_toast_tone_classes_and_icons() {
        assert_eq!(tone_class(ToastTone::Neutral), "tone-neutral");
        assert_eq!(tone_class(ToastTone::Success), "tone-success");
        assert_eq!(tone_class(ToastTone::Danger), "tone-danger");
        assert_eq!(tone_icon(ToastTone::Success), "circle-check");
        assert_eq!(tone_icon(ToastTone::Danger), "alert-circle");
        assert_eq!(tone_icon(ToastTone::Neutral), "info");
    }

    #[component]
    fn ToastScene() -> Element {
        let mut state = use_app_state();
        use_context_provider(|| state);
        use_hook(move || {
            state.toasts.set(vec![
                toast("Removed 3 items (1.4 GB)", ToastTone::Success, None),
                toast(
                    "Pull failed: image not found",
                    ToastTone::Danger,
                    Some(("Retry".to_string(), "image.pull")),
                ),
            ]);
        });
        rsx! { ToastStack {} }
    }

    #[test]
    fn feedback_toast_stack_ssr_renders_app_state_toasts_with_action() {
        let mut dom = VirtualDom::new(ToastScene);
        let _ = dom.rebuild_to_vec();
        let html = dioxus_ssr::render(&dom);
        assert!(html.contains("toast-stack"), "{html}");
        assert!(html.contains("Removed 3 items (1.4 GB)"), "{html}");
        assert!(html.contains("tone-success"), "{html}");
        assert!(html.contains("Pull failed: image not found"), "{html}");
        assert!(html.contains("tone-danger"), "{html}");
        assert!(html.contains("Retry"), "{html}");
        assert!(html.contains("toast-action"), "{html}");
        assert_eq!(html.matches("toast-dismiss").count(), 2, "{html}");
    }

    #[component]
    fn EmptyToastScene() -> Element {
        let state = use_app_state();
        use_context_provider(|| state);
        rsx! { ToastStack {} }
    }

    #[test]
    fn feedback_toast_stack_ssr_empty_state_has_no_toasts() {
        let mut dom = VirtualDom::new(EmptyToastScene);
        let _ = dom.rebuild_to_vec();
        let html = dioxus_ssr::render(&dom);
        assert!(html.contains("toast-stack"), "{html}");
        assert!(!html.contains("toast-text"), "{html}");
    }
}
