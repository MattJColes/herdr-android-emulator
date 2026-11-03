fn main() {
    match std::env::args().nth(1).as_deref() {
        Some("--version" | "-V") => println!("herdr-android {}", env!("CARGO_PKG_VERSION")),
        _ => {
            eprintln!("herdr-android {}: early development, no commands yet", env!("CARGO_PKG_VERSION"));
            std::process::exit(2);
        }
    }
}
