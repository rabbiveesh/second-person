//! The duel's match server, and a headless bot to play against. Players join from the game itself
//! (`--duel <server>`, or `?duel=` on the web).
//!
//!   duel server [--lag <rtt ms>]
//!   duel bot [--server <host:port or ws(s):// URL>] [--lag <rtt ms>]

#[cfg(not(target_family = "wasm"))]
fn main() {
    use rand::Rng;
    use second_person::duel::{self, client::Options};

    let args: Vec<String> = std::env::args().collect();
    let arg = |name: &str| args.iter().position(|a| a == name).and_then(|i| args.get(i + 1).cloned());
    let lag = arg("--lag").and_then(|s| s.parse().ok()).unwrap_or(0);
    match args.get(1).map(String::as_str) {
        Some("server") => {
            duel::server_app(lag).run();
        }
        Some("bot") => {
            let spec = arg("--server").unwrap_or_else(|| format!("127.0.0.1:{}", duel::PORT));
            let opts = Options { id: rand::rng().random(), url: duel::server_url(&spec, false), lag, bot: true };
            duel::headless_client_app(opts).run();
        }
        _ => eprintln!("usage: duel server [--lag ms] | duel bot [--server host:port] [--lag ms]"),
    }
}

#[cfg(target_family = "wasm")]
fn main() {}
