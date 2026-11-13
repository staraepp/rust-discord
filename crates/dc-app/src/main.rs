//! `dc-client`: the desktop app.
//!
//! Two threads: GPUI owns the main thread; a tokio runtime on a second
//! thread runs the gateway and REST clients. They talk over channels, and
//! the UI never sees the token.

mod backend;
mod config;
mod credentials;
mod plugins;

use std::process::ExitCode;
use std::time::Duration;

use anyhow::{Context, Result, bail};
use dc_protocol::Secret;

const TOKEN_VAR: &str = "DC_TOKEN";

fn main() -> ExitCode {
    init_logging();
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("error: {err:#}");
            ExitCode::FAILURE
        }
    }
}

fn init_logging() {
    let level = match std::env::var("DC_LOG").as_deref() {
        Ok("error") => tracing::Level::ERROR,
        Ok("info") => tracing::Level::INFO,
        Ok("debug") => tracing::Level::DEBUG,
        Ok("trace") => tracing::Level::TRACE,
        _ => tracing::Level::WARN,
    };
    tracing_subscriber::fmt()
        .with_max_level(level)
        .with_writer(std::io::stderr)
        .init();
}

/// Reads the token from the environment. Never from argv, never from a file.
// The OS keyring replaces this in a later milestone (blueprint §3).
fn token_from_env() -> Result<Secret<String>> {
    let raw = match std::env::var(TOKEN_VAR) {
        Ok(raw) => raw,
        Err(std::env::VarError::NotPresent) => return credentials::load(),
        Err(std::env::VarError::NotUnicode(_)) => bail!("{TOKEN_VAR} is not valid UTF-8"),
    };
    let token = raw.trim().trim_matches('"').to_owned();
    if token.is_empty() {
        bail!("{TOKEN_VAR} is empty");
    }
    Ok(Secret::new(token))
}

fn run() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.as_slice() {
        [arg] if arg == "--help" || arg == "-h" => {
            println!(
                "dc-client [--store-token | --forget-token | --version]\nCredentials: DC_TOKEN or the OS keyring. Never pass a credential as an argument."
            );
            return Ok(());
        }
        [arg] if arg == "--version" => {
            println!("dc-client {}", env!("CARGO_PKG_VERSION"));
            return Ok(());
        }

        [arg] if arg == "--store-token" => {
            credentials::save(token_from_env()?)?;
            println!("Saved to the OS credential store. You may unset DC_TOKEN.");
            return Ok(());
        }
        [arg] if arg == "--forget-token" => {
            credentials::forget()?;
            println!("Removed the OS credential.");
            return Ok(());
        }
        [] => {}
        _ => bail!(
            "Usage: dc-client [--store-token | --forget-token]; credentials are never accepted in arguments"
        ),
    }
    let token = token_from_env()?;
    let overlay = config::load_overlay().context("could not load the cosmetics config")?;

    let preferences = config::load_preferences().context("could not load settings")?;

    let (event_tx, event_rx) = futures::channel::mpsc::unbounded();
    let (command_tx, command_rx) = tokio::sync::mpsc::unbounded_channel();

    let backend_preferences = preferences.clone();
    let backend = std::thread::Builder::new()
        .name("dc-backend".to_owned())
        .spawn(move || backend::run(token, event_tx, command_rx, backend_preferences))
        .context("could not start the backend thread")?;

    // Blocks until the window is closed.
    let options = dc_ui::UiOptions {
        overlay,
        preferences,
        utc_offset_minutes: chrono::Local::now().offset().local_minus_utc() / 60,
    };
    dc_ui::run(event_rx, options, move |command| {
        let _ = command_tx.send(command);
    });

    // The command sender was dropped with the UI; the backend notices,
    // closes the gateway session cleanly and exits. Do not hang forever.
    let deadline = std::time::Instant::now() + Duration::from_secs(4);
    while !backend.is_finished() && std::time::Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(50));
    }
    Ok(())
}
