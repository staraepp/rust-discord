//! `dc-cli`: headless dev tool for M0.
//!
//! The token comes from the `DC_TOKEN` environment variable and nowhere else.
//! It is wrapped in `Secret` immediately and never printed or logged.

mod args;
mod format;

use std::process::ExitCode;
use std::time::Duration;

use anyhow::{Context, Result, bail};
use dc_api::ApiClient;
use dc_api::http::real::{ReqwestTransport, TokioClock as ApiClock};
use dc_gateway::session::{Session, SessionConfig};
use dc_gateway::ws::{TokioClock as GatewayClock, WsConnector, random_jitter};
use dc_gateway::{GatewayClient, GatewayError, GatewayEvent};
use dc_protocol::gateway::{ClientProperties, DispatchEvent};
use dc_protocol::{Secret, Snowflake};
use tokio::sync::mpsc;

use args::Command;

const TOKEN_VAR: &str = "DC_TOKEN";
const CLIENT_NAME: &str = "dc-client";

type Api = ApiClient<ReqwestTransport, ApiClock>;

#[tokio::main(flavor = "current_thread")]
async fn main() -> ExitCode {
    init_logging();
    let command = match args::parse(std::env::args().skip(1)) {
        Ok(command) => command,
        Err(message) => {
            eprintln!("error: {message}\n\n{}", args::USAGE);
            return ExitCode::from(2);
        }
    };
    match run(command).await {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            // Errors in this workspace never embed the token (see the
            // redaction tests); `{:#}` prints the context chain.
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
fn token_from_env() -> Result<Secret<String>> {
    let raw = match std::env::var(TOKEN_VAR) {
        Ok(raw) => raw,
        Err(std::env::VarError::NotPresent) => bail!(
            "{TOKEN_VAR} is not set. Set it in your shell for this session only \
             (see docs/LOCAL_VERIFY.md); never put it in a file or in arguments."
        ),
        Err(std::env::VarError::NotUnicode(_)) => bail!("{TOKEN_VAR} is not valid UTF-8"),
    };
    let token = raw.trim().trim_matches('"').to_owned();
    if token.is_empty() {
        bail!("{TOKEN_VAR} is empty");
    }
    Ok(Secret::new(token))
}

async fn run(command: Command) -> Result<()> {
    if command == Command::Help {
        print!("{}", args::USAGE);
        return Ok(());
    }
    let token = token_from_env()?;
    let http = ReqwestTransport::new().context("could not set up the HTTPS client")?;
    let api = ApiClient::new(http, ApiClock::new(), Some(token.clone()));

    match command {
        Command::Help => Ok(()),
        Command::History { channel_id, limit } => history(&api, channel_id, limit).await,
        Command::Guilds => gateway(&api, token, None).await,
        Command::Watch { channel_id } => gateway(&api, token, Some(channel_id)).await,
    }
}

async fn history(api: &Api, channel_id: Snowflake, limit: u8) -> Result<()> {
    let mut messages = api
        .get_messages(channel_id, limit, None)
        .await
        .with_context(|| format!("could not fetch messages for channel {channel_id}"))?;
    // The API returns newest first; print oldest first like a chat log.
    messages.reverse();
    for message in &messages {
        println!("{}", format::message(message));
    }
    eprintln!("-- {} message(s)", messages.len());
    Ok(())
}

/// Connects to the gateway. With `watch == None`, prints the guild/channel
/// overview and exits; otherwise prints new messages in that channel.
async fn gateway(api: &Api, token: Secret<String>, watch: Option<Snowflake>) -> Result<()> {
    let info = api
        .get_gateway()
        .await
        .context("could not fetch the gateway URL")?;

    let properties = ClientProperties {
        os: std::env::consts::OS.to_owned(),
        browser: CLIENT_NAME.to_owned(),
        device: CLIENT_NAME.to_owned(),
    };
    let session = Session::new(token, SessionConfig::new(properties), random_jitter());
    let client = GatewayClient::new(WsConnector, GatewayClock::new(), session, info.url);

    let (tx, mut events) = mpsc::channel(256);
    let mut task = tokio::spawn(client.run(tx));
    eprintln!("-- connecting to the gateway...");

    let outcome: Result<()> = loop {
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {
                eprintln!("-- interrupted, closing");
                break Ok(());
            }
            event = events.recv() => match event {
                Some(GatewayEvent::Dispatch(event)) => {
                    if handle_dispatch(event, watch) {
                        break Ok(());
                    }
                }
                Some(GatewayEvent::Reconnecting { delay }) => {
                    eprintln!("-- disconnected; reconnecting in {:.1}s", delay.as_secs_f64());
                }
                None => {
                    // The client stopped on its own: report why.
                    break match (&mut task).await {
                        Ok(Ok(())) => Ok(()),
                        Ok(Err(GatewayError::Fatal(reason))) => Err(reason.into()),
                        Err(join) => Err(anyhow::anyhow!("gateway task failed: {join}")),
                    };
                }
            }
        }
    };

    // Dropping the receiver makes the client close the socket with 1000.
    drop(events);
    if !task.is_finished() {
        let _ = tokio::time::timeout(Duration::from_secs(3), &mut task).await;
        task.abort();
    }
    outcome
}

/// Returns `true` when the command is finished.
fn handle_dispatch(event: DispatchEvent, watch: Option<Snowflake>) -> bool {
    match event {
        DispatchEvent::Ready(ready) => match watch {
            None => {
                print!("{}", format::overview(&ready));
                true
            }
            Some(channel_id) => {
                eprintln!(
                    "-- connected as {} ({} guilds); watching channel {channel_id}",
                    format::sanitize(ready.user.display_name()),
                    ready.guilds.len()
                );
                false
            }
        },
        DispatchEvent::Resumed => {
            eprintln!("-- session resumed");
            false
        }
        DispatchEvent::MessageCreate(message) if Some(message.channel_id) == watch => {
            println!("{}", format::message(&message));
            false
        }
        _ => false,
    }
}
