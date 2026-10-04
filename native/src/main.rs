fn main() {
    let request = std::env::args().nth(1).unwrap_or_else(|| {
        eprintln!("Usage: ipsw-wallpaper '<JSON request>'\nExample: {{\"op\":\"extract\",\"input\":\"firmware.ipsw\",\"output\":\"out\"}}");
        std::process::exit(2)
    });
    match wallpaper_core::run_sync(&request) {
        Ok(value) => println!("{}", value),
        Err(error) => { eprintln!("{error:#}"); std::process::exit(1); }
    }
}
