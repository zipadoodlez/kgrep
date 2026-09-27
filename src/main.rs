use clap::Parser;
use graphgrep::cli::{Cli, Command};

fn main() {
    let cli = Cli::parse();

    let verb = match &cli.command {
        Command::Grep(_) => "grep",
        Command::Find(_) => "find",
        Command::Outline(_) => "outline",
        Command::Trace(_) => "trace",
    };

    // The pipeline lands next; until then every verb fails loudly rather than
    // pretending to have looked.
    eprintln!("error: `{verb}` is not implemented yet");
    std::process::exit(2);
}
