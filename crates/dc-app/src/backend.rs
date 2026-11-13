//! The networking side: a tokio runtime that owns the gateway connection and
//! the REST client, and translates between them and the UI's channels.

use dc_api::ApiClient;
use dc_api::http::real::{ReqwestTransport, TokioClock as ApiClock};
use dc_gateway::session::{Session, SessionConfig};
use dc_gateway::ws::{TokioClock as GatewayClock, WsConnector, random_jitter};
use dc_gateway::{GatewayClient, GatewayError, GatewayEvent};
use dc_protocol::gateway::{ClientProperties, DispatchEvent};
use dc_protocol::{Secret, Snowflake};
use dc_ui::{BackendEvent, UiCommand};
use futures::channel::mpsc::UnboundedSender;
use tokio::sync::mpsc::UnboundedReceiver;

const CLIENT_NAME: &str = "dc-client";
/// Messages fetched when a channel is opened.
const HISTORY_PAGE: u8 = 50;

type Events = UnboundedSender<BackendEvent>;

fn status(events: &Events, text: impl Into<String>) {
    let _ = events.unbounded_send(BackendEvent::Status(text.into()));
}

/// Runs until the UI goes away (its command channel closes).
pub fn run(token: Secret<String>, events: Events, commands: UnboundedReceiver<UiCommand>) {
    let runtime = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(err) => {
            status(&events, format!("Could not start networking: {err}"));
            return;
        }
    };
    runtime.block_on(serve(token, events, commands));
}

async fn serve(token: Secret<String>, events: Events, mut commands: UnboundedReceiver<UiCommand>) {
    let http = match ReqwestTransport::new() {
        Ok(http) => http,
        Err(err) => {
            status(&events, format!("Could not set up HTTPS: {err}"));
            return;
        }
    };
    let api = ApiClient::new(http, ApiClock::new(), Some(token.clone()));

    status(&events, "Fetching gateway URL…");
    let gateway_url = match api.get_gateway().await {
        Ok(info) => info.url,
        Err(err) => {
            status(&events, format!("Could not reach Discord: {err}"));
            // Keep the window usable for reading the error; wait for close.
            while commands.recv().await.is_some() {}
            return;
        }
    };

    let properties = ClientProperties {
        os: std::env::consts::OS.to_owned(),
        browser: CLIENT_NAME.to_owned(),
        device: CLIENT_NAME.to_owned(),
    };
    let session = Session::new(token, SessionConfig::new(properties), random_jitter());
    let client = GatewayClient::new(WsConnector, GatewayClock::new(), session, gateway_url);
    let (gateway_tx, mut gateway_rx) = tokio::sync::mpsc::channel(256);
    let mut gateway_task = tokio::spawn(client.run(gateway_tx));
    status(&events, "Connecting to the gateway…");

    let mut gateway_open = true;
    loop {
        tokio::select! {
            command = commands.recv() => match command {
                Some(UiCommand::LoadHistory(channel_id)) => {
                    load_history(&api, &events, channel_id).await;
                }
                // The window closed.
                None => break,
            },
            event = gateway_rx.recv(), if gateway_open => match event {
                Some(GatewayEvent::Dispatch(event)) => {
                    match &event {
                        DispatchEvent::Ready(ready) => status(
                            &events,
                            format!("Connected as {}", ready.user.display_name()),
                        ),
                        DispatchEvent::Resumed => status(&events, "Reconnected"),
                        _ => {}
                    }
                    if events.unbounded_send(BackendEvent::Dispatch(event)).is_err() {
                        break;
                    }
                }
                Some(GatewayEvent::Reconnecting { delay }) => status(
                    &events,
                    format!("Disconnected; reconnecting in {:.0}s…", delay.as_secs_f64().ceil()),
                ),
                None => {
                    gateway_open = false;
                    let reason = match (&mut gateway_task).await {
                        Ok(Err(GatewayError::Fatal(reason))) => reason.to_string(),
                        Ok(Ok(())) => "connection closed".to_owned(),
                        Err(join) => format!("gateway task failed: {join}"),
                    };
                    status(&events, format!("Stopped: {reason}"));
                }
            }
        }
    }

    // Dropping the receiver makes the gateway client close with code 1000.
    drop(gateway_rx);
    if !gateway_task.is_finished() {
        let _ = tokio::time::timeout(std::time::Duration::from_secs(3), &mut gateway_task).await;
        gateway_task.abort();
    }
}

async fn load_history(
    api: &ApiClient<ReqwestTransport, ApiClock>,
    events: &Events,
    channel_id: Snowflake,
) {
    match api.get_messages(channel_id, HISTORY_PAGE, None).await {
        Ok(messages) => {
            let _ = events.unbounded_send(BackendEvent::History {
                channel_id,
                messages,
            });
        }
        Err(err) => status(events, format!("Could not load messages: {err}")),
    }
}
