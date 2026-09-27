//! The command line: flags in, one query, one packet, rendered.
//!
//! This file is an adapter, not a dispatcher over four implementations. Each arm
//! turns its flags into the one `Query` shape the library takes, runs it, and
//! renders the packet. Nothing here knows how searching works.

use clap::Parser;
use kgrep::cli::{Cli, Command, GrepArgs};
use kgrep::model::{Budget, RenderOptions};
use kgrep::{find, lexical, outline, packet, peak, trace};

/// The grep budget, which is the only verb whose flags include the opt-out.
fn grep_budget(args: &GrepArgs) -> Budget {
    let default = Budget::default();
    Budget {
        max_hits: if args.unbounded {
            None
        } else {
            args.max_hits.or(default.max_hits)
        },
        max_detail_tokens: if args.unbounded {
            None
        } else {
            Some(
                args.max_tokens
                    .unwrap_or(default.max_detail_tokens.unwrap_or(8_000)),
            )
        },
        ..default
    }
}

fn main() {
    let cli = Cli::parse();

    let exit = match &cli.command {
        Command::Grep(args) => {
            let query = args.to_query();
            match lexical::run_grep(&query, grep_budget(args)) {
                Ok(result) => {
                    if args.json {
                        println!(
                            "{}",
                            serde_json::to_string_pretty(&packet::grep_json(&result))
                                .expect("serialize grep json")
                        );
                    } else {
                        println!("{}", packet::render_grep_text(&result));
                    }
                    0
                }
                Err(err) => {
                    eprintln!("error: {err}");
                    2
                }
            }
        }
        Command::Outline(args) => {
            let query = args.to_query();
            match outline::run_outline(&query) {
                Ok(result) => {
                    if args.json {
                        println!(
                            "{}",
                            serde_json::to_string_pretty(&packet::outline_json(&result))
                                .expect("serialize outline json")
                        );
                    } else {
                        println!("{}", packet::render_outline_text(&result));
                    }
                    0
                }
                Err(err) => {
                    eprintln!("error: {err}");
                    2
                }
            }
        }
        Command::Find(args) => {
            let query = args.to_query();
            // `find` caps by its own `--max-files`; the shared hit cap is a
            // second, larger backstop and the smaller of the two wins.
            match find::run_find(&query, Budget::default()) {
                Ok(result) => {
                    let options = RenderOptions {
                        debug_score: args.debug_score,
                    };
                    if args.json {
                        println!(
                            "{}",
                            serde_json::to_string_pretty(&packet::find_json(&result))
                                .expect("serialize find json")
                        );
                    } else {
                        println!("{}", packet::render_find_text(&result, &options));
                    }
                    0
                }
                Err(err) => {
                    eprintln!("error: {err}");
                    2
                }
            }
        }
        Command::Trace(args) => match args.to_query() {
            Ok(query) => match trace::run_trace(&query, Budget::default()) {
                Ok(result) => {
                    let options = RenderOptions {
                        debug_score: args.debug_score,
                    };
                    if args.json {
                        println!(
                            "{}",
                            serde_json::to_string_pretty(&packet::trace_json(&result))
                                .expect("serialize trace json")
                        );
                    } else {
                        println!("{}", packet::render_trace_text(&result, &options));
                    }
                    0
                }
                Err(err) => {
                    eprintln!("error: {err}");
                    2
                }
            },
            Err(err) => {
                eprintln!("error: {err}");
                eprintln!();
                eprintln!("trace queries use a small DSL. Example:");
                eprintln!("  kgrep trace subject:auth_status relation:rendered support:ui");
                2
            }
        },
    };

    peak::report_if_requested();
    if exit != 0 {
        std::process::exit(exit);
    }
}
