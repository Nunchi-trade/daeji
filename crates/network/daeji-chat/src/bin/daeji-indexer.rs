//! Phase-2 POC indexer CLI.
//!
//! In production: this binary subscribes to `IdentityRegistry::AgentRegistered` log events
//! via JSON-RPC `eth_subscribe`, materializes views, and writes the registry file used by
//! agent processes.
//!
//! In the POC: this binary just edits a local JSON file, simulating chain registrations.

use clap::{Parser, Subcommand};
use daeji_chat::registry::Registry;
use std::{path::PathBuf, process::ExitCode};

#[derive(Parser, Debug)]
#[command(version, about = "Daeji POC registry editor (stand-in for the on-chain indexer).")]
struct Args {
    /// Registry file path. Shared with the agent's `--registry-path`.
    #[arg(long, default_value = "/tmp/daeji-registry.json")]
    path: PathBuf,

    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand, Debug)]
enum Cmd {
    /// Initialize an empty registry. Fails if the file already exists.
    Init,
    /// Initialize a registry pre-populated with the given agent seeds.
    Seed {
        /// Comma-separated agent seeds.
        #[arg(value_delimiter = ',')]
        seeds: Vec<u64>,
    },
    /// Add an agent seed.
    Add {
        seed: u64,
    },
    /// Remove an agent seed.
    Remove {
        seed: u64,
    },
    /// Print the current registry as JSON.
    Show,
}

fn main() -> ExitCode {
    let args = Args::parse();
    match args.cmd {
        Cmd::Init => {
            if args.path.exists() {
                eprintln!("error: {} already exists", args.path.display());
                return ExitCode::from(1);
            }
            let r = Registry::empty();
            r.save(&args.path).expect("save");
            println!("initialized empty registry at {}", args.path.display());
        }
        Cmd::Seed { seeds } => {
            let mut r = Registry::empty();
            for s in seeds {
                r.add(s);
            }
            r.save(&args.path).expect("save");
            println!(
                "wrote registry epoch={} agents={:?} -> {}",
                r.epoch,
                r.seeds(),
                args.path.display()
            );
        }
        Cmd::Add { seed } => {
            let mut r = match Registry::load(&args.path) {
                Ok(r) => r,
                Err(_) => Registry::empty(),
            };
            let added = r.add(seed);
            r.save(&args.path).expect("save");
            if added {
                println!(
                    "added seed={} → epoch={} agents={:?}",
                    seed,
                    r.epoch,
                    r.seeds()
                );
            } else {
                println!("seed={} already present (no-op, epoch unchanged)", seed);
            }
        }
        Cmd::Remove { seed } => {
            let mut r = match Registry::load(&args.path) {
                Ok(r) => r,
                Err(e) => {
                    eprintln!("error: {}", e);
                    return ExitCode::from(2);
                }
            };
            let removed = r.remove(seed);
            r.save(&args.path).expect("save");
            if removed {
                println!(
                    "removed seed={} → epoch={} agents={:?}",
                    seed,
                    r.epoch,
                    r.seeds()
                );
            } else {
                println!("seed={} not present (no-op, epoch unchanged)", seed);
            }
        }
        Cmd::Show => {
            let r = match Registry::load(&args.path) {
                Ok(r) => r,
                Err(e) => {
                    eprintln!("error: {}", e);
                    return ExitCode::from(2);
                }
            };
            println!("{}", serde_json::to_string_pretty(&r).unwrap());
        }
    }
    ExitCode::SUCCESS
}
