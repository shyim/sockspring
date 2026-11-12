fn main() {
    let path = std::env::args()
        .nth(1)
        .expect("usage: socks5-rs config.json");
    let config = std::fs::read_to_string(path).expect("read config");
    if let Err(e) = socks5_rs::start(&config) {
        eprintln!("{e}");
        std::process::exit(1);
    }
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    runtime.block_on(tokio::signal::ctrl_c()).unwrap();
    socks5_rs::stop();
}
