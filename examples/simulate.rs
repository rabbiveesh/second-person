//! Run synthetic players through the adaptive difficulty engine (`second_person::adapt`).
//!
//! ```sh
//! cargo run --example simulate -- --profile learner --seed 7 --rounds 200
//! cargo run --example simulate -- --all              # summary table, every profile
//! cargo run --example simulate -- --all --seeds 20   # averaged over 20 seeds
//! ```
//!
//! Profiles: see `adapt::sim::PLAYERS`. An example rather than a bin so plain `cargo run` still
//! starts the game.

use second_person::adapt::sim::{self, Metrics, SimRun, TARGET, WARMUP};
use second_person::adapt::{Cue, Skill};

struct Args {
    profile: String,
    seed: u64,
    rounds: usize,
    all: bool,
    seeds: u64,
}

fn parse() -> Args {
    let mut a = Args { profile: "learner".into(), seed: 0, rounds: 200, all: false, seeds: 1 };
    let argv: Vec<String> = std::env::args().skip(1).collect();
    let mut i = 0;
    let val = |i: usize| argv.get(i + 1).cloned().unwrap_or_else(|| usage(&format!("missing value for {}", argv[i])));
    while i < argv.len() {
        match argv[i].as_str() {
            "--profile" => {
                a.profile = val(i);
                i += 1;
            }
            "--seed" => {
                a.seed = val(i).parse().unwrap_or_else(|_| usage("bad --seed"));
                i += 1;
            }
            "--rounds" => {
                a.rounds = val(i).parse().unwrap_or_else(|_| usage("bad --rounds"));
                i += 1;
            }
            "--seeds" => {
                a.seeds = val(i).parse::<u64>().unwrap_or_else(|_| usage("bad --seeds")).max(1);
                i += 1;
            }
            "--all" => a.all = true,
            "-h" | "--help" => usage(""),
            other => usage(&format!("unknown argument {other}")),
        }
        i += 1;
    }
    a
}

fn usage(msg: &str) -> ! {
    if !msg.is_empty() {
        eprintln!("{msg}");
    }
    let names: Vec<&str> = sim::PLAYERS.iter().map(|p| p.name).collect();
    eprintln!("usage: simulate [--profile <{}>] [--seed N] [--rounds N] [--all [--seeds N]]", names.join("|"));
    std::process::exit(if msg.is_empty() { 0 } else { 2 });
}

fn main() {
    let a = parse();
    if a.all {
        summary(a.seed, a.seeds, a.rounds);
    } else {
        let p = sim::player(&a.profile).unwrap_or_else(|| usage(&format!("unknown profile {}", a.profile)));
        detail(&sim::run(p, a.seed, a.rounds));
    }
}

fn centers(c: &[u8; Skill::COUNT]) -> String {
    c.iter().map(|b| format!("{b:>3}")).collect()
}

/// Round by round: bands played, result, dial, centers after, and what the engine did.
fn detail(run: &SimRun) {
    println!("{} ({})", run.player.name, run.player.about);
    println!("{:>4} {:>8} {:>6} {:>6} {:>8}  cues", "#", "bands", "result", "dial", "centers");
    for (i, r) in run.rounds.iter().enumerate() {
        let result = match (r.result.won, r.result.spotted_first) {
            (true, false) => "WIN*",
            (true, true) => "win",
            (false, _) => "lost",
        };
        let cues: Vec<String> = r
            .cues
            .iter()
            .map(|c| match c {
                Cue::Promoted(s, b) => format!("{}↑{b}", s.name()),
                Cue::Demoted(s, b) => format!("{}↓{b}", s.name()),
                Cue::Eased(s, b) => format!("{} eased→{b}", s.name()),
                Cue::Frustrated(f) => format!("frustrated ({f:?})"),
                Cue::Calibrated(p) => format!("calibrated band {} dial {:.2}", p.band, p.assists),
            })
            .collect();
        println!("{i:>4} {:>8} {result:>6} {:>6.2} {:>8}  {}", centers(&r.bands), r.assists, centers(&r.centers), cues.join(", "));
    }
    print_metrics(&run.metrics());
    println!("(WIN* = got the first hit in before he engaged)");
}

fn print_metrics(m: &Metrics) {
    println!(
        "wins {:.0}% (after warm-up {:.0}%), near {:.0}–{:.0}% target {:.0}% of rounds, osc {} rev {}, prom/dem {}/{}, frustrated {}, mean dial {:.2}, maxed {:.0}%, unassisted {:.0}%, final {}",
        m.win_rate * 100.0,
        m.win_after_warmup * 100.0,
        TARGET.0 * 100.0,
        TARGET.1 * 100.0,
        m.time_near_target * 100.0,
        m.oscillations,
        m.reversals,
        m.promotions,
        m.demotions,
        m.frustrations,
        m.mean_assists,
        m.maxed_assists * 100.0,
        m.unassisted * 100.0,
        centers(&m.final_centers),
    );
}

/// Every profile, averaged over `seeds` seeds.
fn summary(seed: u64, seeds: u64, rounds: usize) {
    println!("{rounds} rounds, seeds {seed}..{}; warm = after round {WARMUP}; centers = stealth gunfight", seed + seeds - 1);
    println!(
        "| {:<9} | {:>4} | {:>4} | {:>4} | {:>4} | {:>4} | {:>9} | {:>5} | {:>4} | {:>5} | {:>5} | {:>7} |",
        "profile", "wins", "warm", "near", "osc", "rev", "prom/dem", "frust", "dial", "maxed", "unast", "centers"
    );
    println!("|{}|", ["-----------", "------", "------", "------", "------", "------", "-----------", "-------", "------", "-------", "-------", "---------"].join("|"));
    for p in sim::PLAYERS {
        let ms: Vec<Metrics> = (seed..seed + seeds).map(|s| sim::run(p, s, rounds).metrics()).collect();
        let n = ms.len() as f32;
        let avg = |f: &dyn Fn(&Metrics) -> f32| ms.iter().map(f).sum::<f32>() / n;
        let center = |k: usize| avg(&|m| m.final_centers[k] as f32);
        println!(
            "| {:<9} | {:>3.0}% | {:>3.0}% | {:>3.0}% | {:>4.1} | {:>4.1} | {:>4.1}/{:<4.1} | {:>5.1} | {:>4.2} | {:>4.0}% | {:>4.0}% | {:>3.1} {:>3.1} |",
            p.name,
            avg(&|m| m.win_rate) * 100.0,
            avg(&|m| m.win_after_warmup) * 100.0,
            avg(&|m| m.time_near_target) * 100.0,
            avg(&|m| m.oscillations as f32),
            avg(&|m| m.reversals as f32),
            avg(&|m| m.promotions as f32),
            avg(&|m| m.demotions as f32),
            avg(&|m| m.frustrations as f32),
            avg(&|m| m.mean_assists),
            avg(&|m| m.maxed_assists) * 100.0,
            avg(&|m| m.unassisted) * 100.0,
            center(0),
            center(1),
        );
    }
}
