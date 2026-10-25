//! Print a scene's SSR HTML. Used by scripts/ssr.sh.
//! Usage: cargo run -q -p berth-app --example ssr -- "Projects / empty / 1440"

fn main() {
    let Some(frame) = std::env::args().nth(1) else {
        eprintln!("usage: ssr \"<frame name>\"");
        std::process::exit(2);
    };
    match berth_app::scenes::render_scene_html(&frame) {
        Some(html) => print!("{html}"),
        None => {
            eprintln!("unknown frame: {frame}");
            for scene in berth_app::scenes::scenes() {
                eprintln!("  {}", scene.frame);
            }
            std::process::exit(1);
        }
    }
}
