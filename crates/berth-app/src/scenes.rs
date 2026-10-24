//! Frame scenes for screenshots and SSR tests (D58, ADR 0002).
//! Every UI task adds the scenes for its frames here.

use dioxus::prelude::*;

use crate::app::Shell;
use crate::state::{use_app_state, AppState, Section};

pub struct Scene {
    pub frame: &'static str,
    pub setup: fn(&mut AppState),
}

fn projects_empty(state: &mut AppState) {
    state.section.set(Section::Projects);
    state.projects.set(Vec::new());
}

const SCENES: &[Scene] = &[Scene {
    frame: "Projects / empty / 1440",
    setup: projects_empty,
}];

pub fn scenes() -> &'static [Scene] {
    SCENES
}

pub fn find(frame: &str) -> Option<&'static Scene> {
    SCENES.iter().find(|s| s.frame == frame)
}

#[component]
fn SceneRoot(frame: String) -> Element {
    let mut state = use_app_state();
    use_context_provider(|| state);
    let setup_frame = frame.clone();
    use_hook(move || {
        if let Some(scene) = find(&setup_frame) {
            (scene.setup)(&mut state);
        }
    });
    rsx! { Shell {} }
}

/// Render a frame's scene to HTML with dioxus-ssr, for tests and scripts/ssr.sh.
pub fn render_scene_html(frame: &str) -> Option<String> {
    find(frame)?;
    let mut dom = VirtualDom::new_with_props(
        SceneRoot,
        SceneRootProps {
            frame: frame.to_string(),
        },
    );
    let _ = dom.rebuild_to_vec();
    Some(dioxus_ssr::render(&dom))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scenes_registry_contains_projects_empty() {
        assert!(find("Projects / empty / 1440").is_some());
        assert_eq!(scenes().len(), 1);
        assert!(find("No such frame").is_none());
    }

    #[test]
    fn scenes_render_projects_empty_shell() {
        let html = render_scene_html("Projects / empty / 1440").expect("scene renders");
        assert!(html.contains("titlebar"), "{html}");
        assert!(html.contains("sidebar"));
        assert!(html.contains("Projects"));
        assert!(html.contains("Containers"));
        assert!(html.contains("No problems"));
        assert!(html.contains("/icons.svg#folder"));
    }

    #[test]
    fn scenes_unknown_frame_renders_nothing() {
        assert!(render_scene_html("Projects / bogus").is_none());
    }
}
