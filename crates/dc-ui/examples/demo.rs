//! Opens the real window with synthetic data. No account, no network.
//!
//! ```text
//! cargo run -p dc-ui --example demo
//! DC_COSMETICS_DEMO=1 cargo run -p dc-ui --example demo   # with sample cosmetics
//! ```

use dc_core::LocalOverlay;
use dc_protocol::gateway::{DispatchEvent, GatewayReceive};
use dc_protocol::model::Message;
use dc_ui::{BackendEvent, UiCommand};

macro_rules! fixture {
    ($path:literal) => {
        include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../fixtures/",
            $path
        ))
    };
}

fn dispatch(text: &str) -> BackendEvent {
    match GatewayReceive::parse(text).expect("fixture envelope") {
        GatewayReceive::Dispatch { name, data, .. } => {
            BackendEvent::Dispatch(DispatchEvent::decode(&name, data).expect("fixture event"))
        }
        other => panic!("not a dispatch: {other:?}"),
    }
}

fn main() {
    let overlay = if std::env::var_os("DC_COSMETICS_DEMO").is_some() {
        LocalOverlay::from_json(
            r##"{
                "me": {"display_name": "me but cooler", "name_color": "#ffd166", "accent_color": "#ff66aa"},
                "users": {"100000000000000002": {"display_name": "bestie", "name_color": "#00ccff"}}
            }"##,
        )
        .expect("demo overlay")
    } else {
        LocalOverlay::default()
    };

    let (events, receiver) = futures::channel::mpsc::unbounded();
    let _ = events.unbounded_send(BackendEvent::Status("Demo data (not connected)".into()));
    let _ = events.unbounded_send(dispatch(fixture!("gateway/ready.json")));

    dc_ui::run(receiver, overlay, move |command| {
        let UiCommand::LoadHistory(channel_id) = command;
        let messages: Vec<Message> =
            serde_json::from_str(fixture!("rest/messages.json")).expect("fixture messages");
        let _ = events.unbounded_send(BackendEvent::History {
            channel_id,
            messages,
        });
    });
}
