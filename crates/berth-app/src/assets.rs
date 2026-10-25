//! Static assets compiled into the binary (PLAN D5).

use dioxus::desktop::wry::http::{header, Response, StatusCode};
use dioxus::desktop::{use_asset_handler, AssetRequest};

const TOKENS_CSS: &str = include_str!("../assets/css/tokens.css");
const SHELL_CSS: &str = include_str!("../assets/css/shell.css");
const TABLES_CSS: &str = include_str!("../assets/css/tables.css");
const UI_CSS: &str = include_str!("../assets/css/ui.css");
const PALETTE_CSS: &str = include_str!("../assets/css/palette.css");
const LOGS_CSS: &str = include_str!("../assets/css/logs.css");
pub const FONTS_CSS: &str = include_str!("../assets/css/fonts.css");
pub const ICONS_SVG: &str = include_str!("../assets/icons.svg");

const FONTS: &[(&str, &[u8])] = &[
    (
        "IBMPlexSans-400.ttf",
        include_bytes!("../assets/fonts/IBMPlexSans-400.ttf"),
    ),
    (
        "IBMPlexSans-500.ttf",
        include_bytes!("../assets/fonts/IBMPlexSans-500.ttf"),
    ),
    (
        "IBMPlexSans-600.ttf",
        include_bytes!("../assets/fonts/IBMPlexSans-600.ttf"),
    ),
    (
        "JetBrainsMono-400.ttf",
        include_bytes!("../assets/fonts/JetBrainsMono-400.ttf"),
    ),
    (
        "JetBrainsMono-500.ttf",
        include_bytes!("../assets/fonts/JetBrainsMono-500.ttf"),
    ),
];

/// Document head: tokens and shell CSS. @font-face rules are deliberately
/// rendered by the root component after the /fonts/ handler registers (D5).
pub fn head_html() -> String {
    format!(
        "<style>{TOKENS_CSS}</style><style>{SHELL_CSS}</style><style>{UI_CSS}</style><style>{TABLES_CSS}</style><style>{PALETTE_CSS}</style><style>{LOGS_CSS}</style>"
    )
}

fn font_bytes(file: &str) -> Option<&'static [u8]> {
    FONTS
        .iter()
        .find(|(name, _)| *name == file)
        .map(|(_, bytes)| *bytes)
}

fn respond(responder: dioxus::desktop::RequestAsyncResponder, response: Response<Vec<u8>>) {
    responder.respond(response);
}

fn ok(content_type: &str, body: Vec<u8>) -> Response<Vec<u8>> {
    match Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, content_type)
        .body(body)
    {
        Ok(r) => r,
        Err(_) => Response::new(Vec::new()),
    }
}

fn not_found() -> Response<Vec<u8>> {
    match Response::builder()
        .status(StatusCode::NOT_FOUND)
        .body(Vec::new())
    {
        Ok(r) => r,
        Err(_) => Response::new(Vec::new()),
    }
}

/// Serve /fonts/<file> and /icons.svg from the embedded tables. Desktop root only.
pub fn use_assets() {
    use_asset_handler("fonts", |request: AssetRequest, responder| {
        let file = request.uri().path().trim_start_matches("/fonts/");
        let response = match font_bytes(file) {
            Some(bytes) => ok("font/ttf", bytes.to_vec()),
            None => not_found(),
        };
        respond(responder, response);
    });
    use_asset_handler("icons.svg", |_request: AssetRequest, responder| {
        respond(
            responder,
            ok("image/svg+xml", ICONS_SVG.as_bytes().to_vec()),
        );
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_font_face_in_fonts_css_is_embedded() {
        let files: Vec<&str> = FONTS_CSS
            .split("url(\"/fonts/")
            .skip(1)
            .filter_map(|rest| rest.split('"').next())
            .collect();
        assert_eq!(files.len(), 5);
        for file in files {
            assert!(font_bytes(file).is_some_and(|b| b.len() > 10_000), "{file}");
        }
    }

    #[test]
    fn unknown_font_is_none() {
        assert!(font_bytes("Nope.ttf").is_none());
    }

    #[test]
    fn head_has_tokens_but_no_font_faces() {
        let head = head_html();
        assert!(head.contains("--color-bg"));
        assert!(!head.contains("@font-face"));
        assert!(FONTS_CSS.contains("@font-face"));
    }

    #[test]
    fn sprite_has_symbols_without_hex_colours() {
        assert!(ICONS_SVG.matches("<symbol").count() >= 40);
        assert!(!ICONS_SVG.contains("stroke=\"#"));
        assert!(!ICONS_SVG.contains("fill=\"#"));
    }

    /// Every symbol is the icon component's own 16x16 frame, so a glyph
    /// fills the same share of the icon box as in the design (a cropped
    /// viewBox drew glyphs at about 0.6 of that size).
    #[test]
    fn sprite_symbols_are_all_16_by_16_frames() {
        let mut checked = 0;
        for symbol in ICONS_SVG.split("<symbol ").skip(1) {
            let view_box = symbol
                .split("viewBox=\"")
                .nth(1)
                .and_then(|rest| rest.split('"').next())
                .expect("a viewBox");
            let numbers: Vec<f32> = view_box
                .split_whitespace()
                .map(|n| n.parse().expect("a number"))
                .collect();
            assert_eq!(numbers.len(), 4, "{view_box}");
            assert!(
                (numbers[2] - 16.0).abs() < f32::EPSILON
                    && (numbers[3] - 16.0).abs() < f32::EPSILON,
                "viewBox {view_box} is not a 16x16 frame"
            );
            checked += 1;
        }
        assert!(checked >= 40, "only {checked} symbols");
    }
}
