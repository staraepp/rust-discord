//! Dev task runner: `cargo xtask <ci|fmt|clippy|test|deny|licenses>`.
//!
//! `ci` runs exactly the "Definition of done" checks from CLAUDE.md, in order.

use std::process::{Command, ExitCode};
mod licenses;

const STEPS: &[(&str, &[&str])] = &[
    ("fmt", &["fmt", "--all", "--", "--check"]),
    (
        "clippy",
        &[
            "clippy",
            "--workspace",
            "--all-targets",
            "--",
            "-D",
            "warnings",
        ],
    ),
    ("test", &["test", "--workspace"]),
    ("deny", &["deny", "check"]),
];

fn run(name: &str, args: &[&str]) -> bool {
    println!("==> {name}: cargo {}", args.join(" "));
    let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".to_owned());
    match Command::new(cargo).args(args).status() {
        Ok(status) if status.success() => true,
        Ok(status) => {
            println!("xtask: step `{name}` failed ({status})");
            false
        }
        Err(err) => {
            println!("xtask: could not run cargo for step `{name}`: {err}");
            false
        }
    }
}

fn main() -> ExitCode {
    let task = std::env::args().nth(1).unwrap_or_default();
    let ok = match task.as_str() {
        "licenses" => {
            let path = std::env::args()
                .nth(2)
                .unwrap_or_else(|| "work/THIRD_PARTY_LICENSES.txt".into());
            match licenses::collect(std::path::Path::new(&path)) {
                Ok(()) => true,
                Err(error) => {
                    eprintln!("License collection failed: {error}");
                    false
                }
            }
        }
        "ci" => STEPS.iter().all(|(name, args)| run(name, args)),
        other => match STEPS.iter().find(|(name, _)| *name == other) {
            Some((name, args)) => run(name, args),
            None => {
                println!("usage: cargo xtask <ci|fmt|clippy|test|deny|licenses>");
                false
            }
        },
    };
    if ok {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}
