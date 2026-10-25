use dioxus::prelude::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum IconSize {
    Sm,
    #[default]
    Md,
}

impl IconSize {
    pub fn class(self) -> &'static str {
        match self {
            Self::Sm => "icon icon-sm",
            Self::Md => "icon icon-md",
        }
    }
}

/// An icon from the sprite (`/icons.svg`, PLAN 7.3). Size comes from the
/// `--size-icon-sm` / `--size-icon-md` tokens via CSS.
#[component]
pub fn Icon(name: &'static str, #[props(default)] size: IconSize) -> Element {
    rsx! {
        svg { class: size.class(), "aria-hidden": "true",
            r#use { href: "/icons.svg#{name}" }
        }
    }
}
