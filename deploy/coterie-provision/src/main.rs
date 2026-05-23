use anyhow::Result;
use clap::{Parser, Subcommand};
use coterie_provision::fs_ops::RealFs;
use coterie_provision::install::{self, InstallArgs};
use coterie_provision::prompts::InquirePrompter;
use coterie_provision::system::RealSystem;

#[derive(Parser, Debug)]
#[command(
    name = "coterie-provision",
    version,
    about = "End-to-end install wizard for Coterie."
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand, Debug)]
enum Command {
    /// Install Coterie on this Debian/Ubuntu host end-to-end.
    Install(Box<InstallArgs>),
    /// Swap Stripe keys from test mode to live mode. (Implemented in a25.)
    SwitchStripeToLive,
}

fn main() {
    if let Err(e) = real_main() {
        eprintln!("Error: {e}");
        let mut chain = e.chain().skip(1);
        if chain.clone().next().is_some() {
            eprintln!();
            eprintln!("Caused by:");
            for (i, cause) in chain.by_ref().enumerate() {
                eprintln!("    {i}: {cause}");
            }
        }
        std::process::exit(1);
    }
}

fn real_main() -> Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Command::Install(args) => {
            let sys = RealSystem;
            let fs = RealFs;
            let prompts = InquirePrompter;
            install::run(*args, &sys, &fs, &prompts)
        }
        Command::SwitchStripeToLive => {
            // Filled in by a25 — stub here so the binary surface exists.
            todo!("switch-stripe-to-live is implemented by spec a25")
        }
    }
}
