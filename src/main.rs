use clap::Parser;
use kgrep::cli::{Cli, Command};
use kgrep::model::Budget;
use kgrep::{lexical, outline, packet, peak};
use std::path::PathBuf;

fn resolve_root(path: &Option<String>) -> PathBuf {
    match path {
        Some(path) => PathBuf::from(path),
        None => std::env::current_dir().expect("current directory"),
    }
}

fn main() {
    let cli = Cli::parse();

    let exit = match &cli.command {
        Command::Grep(args) => {
            let root = resolve_root(&args.scope.path);
            let budget = Budget {
                max_hits: if args.unbounded {
                    None
                } else {
                    args.max_hits.or(Budget::default().max_hits)
                },
                max_detail_tokens: if args.unbounded {
                    None
                } else {
                    Some(
                        args.max_tokens
                            .unwrap_or(Budget::default().max_detail_tokens.unwrap_or(8_000)),
                    )
                },
                ..Budget::default()
            };
            match lexical::run_grep(&root, args, budget) {
                Ok(packet) => {
                    if args.json {
                        println!(
                            "{}",
                            serde_json::to_string_pretty(&packet::grep_json(&packet))
                                .expect("serialize grep json")
                        );
                    } else {
                        println!("{}", packet::render_grep_text(&packet, args));
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
            let root = resolve_root(&args.scope.path);
            match outline::run_outline(&root, args) {
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
        Command::Find(_) => {
            eprintln!("error: `find` is not implemented yet");
            2
        }
        Command::Trace(_) => {
            eprintln!("error: `trace` is not implemented yet");
            2
        }
    };

    peak::report_if_requested();
    if exit != 0 {
        std::process::exit(exit);
    }
}
