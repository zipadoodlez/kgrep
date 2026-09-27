use clap::Parser;
use kgrep::cli::{Cli, Command};
use kgrep::model::Budget;
use kgrep::{find, lexical, outline, packet, peak, trace};
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
        Command::Find(args) => {
            let root = resolve_root(&args.scope.path);
            // `find` caps by its own `--max-files`; the shared hit cap is a
            // second, larger backstop and the smaller of the two wins.
            match find::run_find(&root, args, Budget::default()) {
                Ok(packet) => {
                    if args.json {
                        println!(
                            "{}",
                            serde_json::to_string_pretty(&packet::find_json(&packet))
                                .expect("serialize find json")
                        );
                    } else {
                        println!("{}", packet::render_find_text(&packet, args));
                    }
                    0
                }
                Err(err) => {
                    eprintln!("error: {err}");
                    2
                }
            }
        }
        Command::Trace(args) => {
            let root = resolve_root(&args.scope.path);
            match trace::run_trace(&root, args, Budget::default()) {
                Ok(packet) => {
                    if args.json {
                        println!(
                            "{}",
                            serde_json::to_string_pretty(&packet::trace_json(&packet))
                                .expect("serialize trace json")
                        );
                    } else {
                        println!("{}", packet::render_trace_text(&packet, args));
                    }
                    0
                }
                Err(err) => {
                    eprintln!("error: {err}");
                    eprintln!();
                    eprintln!("trace queries use a small DSL. Example:");
                    eprintln!("  kgrep trace subject:auth_status relation:rendered support:ui");
                    2
                }
            }
        }
    };

    peak::report_if_requested();
    if exit != 0 {
        std::process::exit(exit);
    }
}
