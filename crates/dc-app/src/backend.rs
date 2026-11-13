//! The networking side: a tokio runtime that owns the gateway connection and
//! the REST client, and translates between them and the UI's channels.

use dc_api::http::real::{ReqwestTransport, TokioClock as ApiClock};
use dc_api::{ApiClient, HttpTransport};
use dc_gateway::session::{Session, SessionConfig};
use dc_gateway::ws::{TokioClock as GatewayClock, WsConnector, random_jitter};
use dc_gateway::{GatewayClient, GatewayError, GatewayEvent};
use dc_protocol::Secret;
use dc_protocol::gateway::{ClientProperties, DispatchEvent};
use dc_protocol::time::Clock;
use dc_store::cache::{HistoryCache, SqliteCache};
use dc_ui::{BackendEvent, UiCommand};
use futures::channel::mpsc::UnboundedSender;
use std::collections::VecDeque;
use std::sync::Arc;
use tokio::sync::mpsc::UnboundedReceiver;

const CLIENT_NAME: &str = "dc-client";
/// Messages fetched when a channel is opened.
const HISTORY_PAGE: u8 = 50;

type Events = UnboundedSender<BackendEvent>;

fn status(events: &Events, text: impl Into<String>) {
    let _ = events.unbounded_send(BackendEvent::Status(text.into()));
}

/// Runs until the UI goes away (its command channel closes).
pub fn run(
    token: Secret<String>,
    events: Events,
    commands: UnboundedReceiver<UiCommand>,
    preferences: dc_core::preferences::Preferences,
) {
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
    runtime.block_on(serve(token, events, commands, preferences));
}

async fn serve(
    token: Secret<String>,
    events: Events,
    mut commands: UnboundedReceiver<UiCommand>,
    mut preferences: dc_core::preferences::Preferences,
) {
    // Do not connect until the owner explicitly accepts the first-run risk.
    if !preferences.risk_accepted {
        status(&events, "Accept the first-run notice to connect");
        loop {
            match commands.recv().await {
                Some(UiCommand::SavePreferences(prefs)) if prefs.risk_accepted => {
                    preferences = prefs.clone();
                    let error = crate::config::save_preferences(&prefs)
                        .err()
                        .map(|e| e.to_string());
                    let _ = events.unbounded_send(BackendEvent::PreferencesSaved { error });
                    break;
                }
                Some(_) => {}
                None => return,
            }
        }
    }
    let http = match ReqwestTransport::new() {
        Ok(http) => http,
        Err(err) => {
            status(&events, format!("Could not set up HTTPS: {err}"));
            return;
        }
    };
    let api = Arc::new(ApiClient::new(http, ApiClock::new(), Some(token.clone())));

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
    let (gateway_commands, gateway_command_rx) = tokio::sync::mpsc::channel(32);
    let client = GatewayClient::new(WsConnector, GatewayClock::new(), session, gateway_url)
        .with_commands(gateway_command_rx);
    let (gateway_tx, mut gateway_rx) = tokio::sync::mpsc::channel(256);
    let mut gateway_task = tokio::spawn(client.run(gateway_tx));
    status(&events, "Connecting to the gateway…");

    let mut plugins = crate::plugins::Plugins::load(&preferences);
    let _ = events.unbounded_send(BackendEvent::Plugins(plugins.info.clone()));
    let mut transformed: std::collections::HashMap<String, dc_protocol::model::CreateMessage> =
        Default::default();
    let mut current_user = None;
    let mut cache: Option<SqliteCache> = None;
    let mut foreground = VecDeque::new();
    let mut images = VecDeque::new();
    let mut foreground_jobs = tokio::task::JoinSet::new();
    let mut image_jobs = tokio::task::JoinSet::new();
    let mut gateway_open = true;
    loop {
        if foreground_jobs.is_empty()
            && let Some(command) = foreground.pop_front()
        {
            let api = api.clone();
            foreground_jobs.spawn(async move { handle_command(api.as_ref(), command).await });
        }
        while image_jobs.len() < 2 {
            let Some(command) = images.pop_front() else {
                break;
            };
            let api = api.clone();
            image_jobs.spawn(async move { handle_command(api.as_ref(), command).await });
        }
        tokio::select! {
            command = commands.recv() => match command {
                Some(UiCommand::SubscribeMembers { guild_id, channel_id, offset }) => {
                    let _ = gateway_commands.try_send(dc_gateway::GatewayCommand::Members { guild: guild_id, channel: channel_id, offset });
                }
                Some(UiCommand::LoadHistory(channel)) => {
                    if let Some(cache)=&cache && let Ok(messages)=cache.history(channel,50) && !messages.is_empty() { let _=events.unbounded_send(BackendEvent::History { channel_id:channel, messages }); }
                    foreground.push_back(UiCommand::LoadHistory(channel));
                }
                Some(UiCommand::ClearHistoryCache) => {
                    let result=if let Some(cache)=&mut cache {cache.clear().map_err(|_|"Could not erase local history")}
                        else if let Some(user)=current_user {match open_cache(user,&events) {Some(mut cache)=>cache.clear().map_err(|_|"Could not erase local history"),None=>Err("No local cache is available")}}
                        else {Err("Wait until your account is connected")};
                    status(&events,match result {Ok(())=>"Local cached history erased; Discord messages are unchanged",Err(error)=>error});
                }
                Some(UiCommand::SavePreferences(prefs)) => {
                    preferences=prefs.clone();
                    plugins=crate::plugins::Plugins::load(&preferences);
                    let _=events.unbounded_send(BackendEvent::Plugins(plugins.info.clone()));
                    if !preferences.cache_history { cache=None; } else if let Some(user)=current_user { cache=open_cache(user,&events); }
                    foreground.push_back(UiCommand::SavePreferences(prefs));
                }
                Some(UiCommand::SendMessage {channel_id,mut message,attachments}) => {
                    if let Some(saved)=transformed.get(&message.nonce) { message=saved.clone(); }
                    else {
                        match plugins.transform(&message.content) {
                            Ok(content)=>message.content=content,
                            Err(error)=>{ let _=events.unbounded_send(BackendEvent::SendFailed {channel_id,nonce:message.nonce,error}); continue; }
                        }
                        if transformed.len()>=128 { transformed.clear(); }
                        transformed.insert(message.nonce.clone(),message.clone());
                    }
                    foreground.push_front(UiCommand::SendMessage {channel_id,message,attachments});
                }
                Some(command @ UiCommand::FetchImage(_)) => images.push_back(command),
                Some(command) => foreground.push_back(command),
                None => break,
            },
            result = foreground_jobs.join_next(), if !foreground_jobs.is_empty() => { if let Some(Ok(event)) = result { cache_event(&mut cache,&event); if events.unbounded_send(event).is_err() { break; } } },
            result = image_jobs.join_next(), if !image_jobs.is_empty() => { if let Some(Ok(event)) = result && events.unbounded_send(event).is_err() { break; } },
            event = gateway_rx.recv(), if gateway_open => match event {
                Some(GatewayEvent::Dispatch(event)) => {
                    if matches!(&event,DispatchEvent::Unknown {name,..} if name=="VOICE_SERVER_UPDATE") { continue; }
                    match &event {
                        DispatchEvent::Ready(ready) => {
                            current_user=Some(ready.user.id);
                            cache=if preferences.cache_history { open_cache(ready.user.id,&events) } else { None };
                            status(&events,format!("Connected as {}",ready.user.display_name()));
                        },
                        DispatchEvent::Resumed => status(&events, "Reconnected"),
                        _ => {}
                    }
                    cache_dispatch(&mut cache,&event);
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

async fn handle_command<H: HttpTransport, C: Clock>(
    api: &ApiClient<H, C>,
    command: UiCommand,
) -> BackendEvent {
    match command {
        UiCommand::ClearHistoryCache => {
            BackendEvent::OperationFailed("Cache command is handled by the backend".into())
        }
        UiCommand::LoadOlder { channel_id, before } => match api
            .get_messages(channel_id, HISTORY_PAGE, Some(before))
            .await
        {
            Ok(messages) => BackendEvent::OlderHistory {
                channel_id,
                messages,
            },
            Err(error) => BackendEvent::OperationFailed(error.to_string()),
        },
        UiCommand::EditMessage {
            channel_id,
            message_id,
            content,
        } => match api.edit_message(channel_id, message_id, &content).await {
            Ok(message) => BackendEvent::MessageChanged(Box::new(message)),
            Err(error) => BackendEvent::OperationFailed(error.to_string()),
        },
        UiCommand::DeleteMessage {
            channel_id,
            message_id,
        } => match api.delete_message(channel_id, message_id).await {
            Ok(()) => BackendEvent::MessageRemoved {
                channel_id,
                message_id,
            },
            Err(error) => BackendEvent::OperationFailed(error.to_string()),
        },

        UiCommand::SaveOverlay(json) => {
            let error = crate::config::save_overlay(&json)
                .err()
                .map(|e| e.to_string());
            BackendEvent::OverlaySaved { json, error }
        }
        UiCommand::SavePreferences(prefs) => {
            let error = crate::config::save_preferences(&prefs)
                .err()
                .map(|e| e.to_string());
            BackendEvent::PreferencesSaved { error }
        }

        UiCommand::SubscribeMembers { .. } => {
            BackendEvent::Status("Member subscription is handled by the gateway".into())
        }
        UiCommand::SendMessage {
            channel_id,
            message,
            attachments,
        } => {
            let nonce = message.nonce.clone();
            let files = match tokio::task::spawn_blocking(move || read_uploads(attachments)).await {
                Ok(Ok(files)) => files,
                Ok(Err(error)) => {
                    return BackendEvent::SendFailed {
                        channel_id,
                        nonce,
                        error: error.into(),
                    };
                }
                Err(_) => {
                    return BackendEvent::SendFailed {
                        channel_id,
                        nonce,
                        error: "Attachment worker failed".into(),
                    };
                }
            };
            match api
                .create_message_with_files(channel_id, &message, &files)
                .await
            {
                Ok(message) => BackendEvent::Sent {
                    channel_id,
                    nonce,
                    message: Box::new(message),
                },
                Err(error) => BackendEvent::SendFailed {
                    channel_id,
                    nonce,
                    error: error.to_string(),
                },
            }
        }
        UiCommand::LoadHistory(channel_id) => {
            match api.get_messages(channel_id, HISTORY_PAGE, None).await {
                Ok(messages) => BackendEvent::History {
                    channel_id,
                    messages,
                },
                Err(err) => BackendEvent::Status(format!("Could not load messages: {err}")),
            }
        }
        UiCommand::FetchImage(url) => match api.get_cdn(&url).await {
            Ok(bytes) => BackendEvent::Image { url, bytes },
            Err(_) => BackendEvent::ImageFailed { url },
        },
    }
}

fn open_cache(user: dc_protocol::Snowflake, events: &Events) -> Option<SqliteCache> {
    let path = crate::config::cosmetics_path()?.with_file_name(format!("messages-{user}.sqlite"));
    match SqliteCache::open(&path) {
        Ok(cache) => Some(cache),
        Err(_) => {
            status(
                events,
                "Could not open local history cache; continuing without it",
            );
            None
        }
    }
}
fn cache_event(cache: &mut Option<SqliteCache>, event: &BackendEvent) {
    let Some(cache) = cache else {
        return;
    };
    match event {
        BackendEvent::History { messages, .. } => {
            for message in messages {
                let _ = cache.remember(message);
            }
        }
        BackendEvent::Sent { message, .. } => {
            let _ = cache.remember(message);
        }
        _ => {}
    }
}
fn cache_dispatch(cache: &mut Option<SqliteCache>, event: &DispatchEvent) {
    let Some(cache) = cache else {
        return;
    };
    match event {
        DispatchEvent::MessageCreate(message) => {
            let _ = cache.remember(message);
        }
        DispatchEvent::MessageDelete(message) => {
            let _ = cache.remove(message.channel_id, message.id);
        }
        _ => {}
    }
}

fn read_uploads(paths: Vec<std::path::PathBuf>) -> Result<Vec<dc_api::UploadFile>, &'static str> {
    use std::io::Read;
    if paths.len() > 10 {
        return Err("At most ten attachments are supported");
    }
    let mut total = 0usize;
    let mut files = Vec::new();
    for path in paths {
        let file = std::fs::File::open(&path).map_err(|_| "Could not open attachment")?;
        let metadata = file
            .metadata()
            .map_err(|_| "Could not read attachment metadata")?;
        let remaining = 25 * 1024 * 1024 - total;
        if !metadata.is_file() || metadata.len() > remaining as u64 {
            return Err("Select regular files totaling at most 25 MiB");
        }
        let mut bytes = Vec::new();
        file.take(remaining as u64 + 1)
            .read_to_end(&mut bytes)
            .map_err(|_| "Could not read attachment")?;
        if bytes.len() > remaining {
            return Err("Attachments exceed 25 MiB total");
        }
        total += bytes.len();
        files.push(dc_api::UploadFile {
            filename: path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default(),
            bytes,
        });
    }
    Ok(files)
}

#[cfg(test)]
mod tests {
    use super::*;
    use dc_api::http::mock::MockHttp;
    use dc_protocol::time::FakeClock;

    #[tokio::test]
    async fn image_download_returns_bytes_without_authorization() {
        let http = MockHttp::new();
        http.push(200, &[], "synthetic image bytes");
        let api = ApiClient::new(
            http.clone(),
            FakeClock::new(),
            Some(Secret::new("synthetic-test-credential".to_owned())),
        );
        let url = "https://cdn.discordapp.com/embed/avatars/0.png";
        let event = handle_command(&api, UiCommand::FetchImage(url.into())).await;
        assert!(matches!(event, BackendEvent::Image { url: actual, bytes }
            if actual == url && bytes == b"synthetic image bytes"));
        let requests = http.requests();
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].url, url);
        assert!(requests[0].authorization.is_none());
    }

    #[tokio::test]
    async fn failed_or_invalid_image_url_keeps_the_fallback() {
        let http = MockHttp::new();
        http.push(404, &[], "");
        let api = ApiClient::new(http.clone(), FakeClock::new(), None);
        for url in [
            "https://cdn.discordapp.com/missing.png",
            "https://example.invalid/image.png",
        ] {
            let event = handle_command(&api, UiCommand::FetchImage(url.into())).await;
            assert!(matches!(event, BackendEvent::ImageFailed { url: actual } if actual == url));
        }
        assert_eq!(http.requests().len(), 1);
    }
}
