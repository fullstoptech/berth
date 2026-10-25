//! Print where each named program is found on the login-shell-augmented
//! search path: `cargo run -p berth-sys --example which -- claude muse docker colima`

#[tokio::main]
async fn main() {
    let search = berth_sys::SearchPath::from_process()
        .with_login_shell()
        .await;
    let mut missing = false;
    for program in std::env::args().skip(1) {
        match search.find(&program) {
            Some(path) => println!("{}", path.display()),
            None => {
                eprintln!("{program}: not found");
                missing = true;
            }
        }
    }
    if missing {
        std::process::exit(1);
    }
}
