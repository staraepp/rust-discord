//! The GPUI window. Drawing only: every decision about *what* to show comes
//! from [`crate::model`].
//!
//! UNVERIFIED visually: the sandbox this was written in cannot display a
//! window. See `docs/LOCAL_VERIFY.md`.

use std::collections::HashSet;
use std::path::PathBuf;

use dc_core::overlay::{Color, LocalOverlay};
use dc_protocol::Snowflake;
use dc_store::{Change, Store};
use futures::StreamExt;
use futures::channel::mpsc::UnboundedReceiver;
use gpui::{
    App, Application, Bounds, Context, FontWeight, ObjectFit, Rgba, ScrollHandle, SharedString,
    TitlebarOptions, Window, WindowBounds, WindowOptions, div, img, prelude::*, px, rgb, size,
    uniform_list,
};

use crate::model::{self, MessageRow, ProfileCard, RowKind, SidebarRow};
use crate::{BackendEvent, UiCommand};

// A plain dark palette.
const BG_MAIN: u32 = 0x1e1f22;
const BG_SIDEBAR: u32 = 0x17181a;
const BG_HEADER: u32 = 0x1a1b1e;
const BG_SELECTED: u32 = 0x34363c;
const BG_HOVER: u32 = 0x2a2c31;
const TEXT: u32 = 0xdcddde;
const TEXT_MUTED: u32 = 0x8e9297;
const TEXT_HEADING: u32 = 0xffffff;
const DEFAULT_ACCENT: u32 = 0x3a3d44;

const WINDOW_TITLE: &str = "dc-client";

fn color(color: Option<Color>, fallback: u32) -> Rgba {
    rgb(color.map_or(fallback, |c| c.0))
}

/// Only local files are loaded for cosmetics; remote URLs are ignored for
/// now (no HTTP client is wired into the image loader yet).
fn local_image(source: Option<&str>) -> Option<PathBuf> {
    let source = source?.trim();
    if source.is_empty() || source.contains("://") {
        return None;
    }
    Some(PathBuf::from(source))
}

pub struct ChatView {
    store: Store,
    overlay: LocalOverlay,
    selected: Option<Snowflake>,
    status: SharedString,
    /// Channels whose history has been requested already.
    requested: HashSet<Snowflake>,
    // Cached view-models, rebuilt only when the store changes.
    sidebar: Vec<SidebarRow>,
    rows: Vec<MessageRow>,
    profile: Option<ProfileCard>,
    title: SharedString,
    scroll: ScrollHandle,
    on_command: Box<dyn Fn(UiCommand)>,
}

impl ChatView {
    fn new(
        mut events: UnboundedReceiver<BackendEvent>,
        overlay: LocalOverlay,
        on_command: Box<dyn Fn(UiCommand)>,
        cx: &mut Context<Self>,
    ) -> Self {
        cx.spawn(async move |this, cx| {
            while let Some(event) = events.next().await {
                let alive = this.update(cx, |view, cx| {
                    view.apply(event);
                    cx.notify();
                });
                if alive.is_err() {
                    break;
                }
            }
        })
        .detach();

        ChatView {
            store: Store::new(),
            overlay,
            selected: None,
            status: "Connecting…".into(),
            requested: HashSet::new(),
            sidebar: Vec::new(),
            rows: Vec::new(),
            profile: None,
            title: model::channel_title(&Store::new(), None).into(),
            scroll: ScrollHandle::new(),
            on_command,
        }
    }

    fn apply(&mut self, event: BackendEvent) {
        match event {
            BackendEvent::Status(status) => self.status = status.into(),
            BackendEvent::History {
                channel_id,
                messages,
            } => {
                self.store.insert_history(channel_id, messages);
                self.refresh(Change::Messages(channel_id));
            }
            BackendEvent::Dispatch(event) => {
                let change = self.store.apply(&event);
                self.refresh(change);
            }
        }
    }

    fn refresh(&mut self, change: Change) {
        match change {
            Change::None => {}
            Change::All => {
                // A new READY: history must be fetched again.
                self.requested.clear();
                self.refresh_tree();
                if let Some(channel_id) = self.selected {
                    self.select(channel_id);
                }
            }
            Change::Guilds => self.refresh_tree(),
            Change::Messages(channel_id) => {
                if self.selected == Some(channel_id) {
                    self.refresh_messages();
                }
            }
        }
    }

    fn refresh_tree(&mut self) {
        self.sidebar = model::sidebar(&self.store);
        self.profile = model::profile_card(&self.store, &self.overlay);
        self.title = model::channel_title(&self.store, self.selected).into();
    }

    fn refresh_messages(&mut self) {
        self.rows = match self.selected {
            Some(channel_id) => model::messages(&self.store, &self.overlay, channel_id),
            None => Vec::new(),
        };
        self.scroll.scroll_to_bottom();
    }

    fn select(&mut self, channel_id: Snowflake) {
        self.selected = Some(channel_id);
        self.title = model::channel_title(&self.store, self.selected).into();
        self.refresh_messages();
        if self.requested.insert(channel_id) {
            (self.on_command)(UiCommand::LoadHistory(channel_id));
        }
    }

    fn render_profile(&self) -> impl IntoElement {
        let card = self.profile.as_ref();
        let banner = local_image(card.and_then(|c| c.banner.as_deref()));
        let avatar = local_image(card.and_then(|c| c.avatar.as_deref()));
        let decoration = local_image(card.and_then(|c| c.avatar_decoration.as_deref()));
        let accent = color(card.and_then(|c| c.accent_color), DEFAULT_ACCENT);
        let name: SharedString = card
            .map_or_else(|| "Not connected".to_owned(), |c| c.name.clone())
            .into();
        let name_color = color(card.and_then(|c| c.name_color), TEXT_HEADING);

        div()
            .flex()
            .flex_col()
            .flex_none()
            .child(match banner {
                Some(path) => div()
                    .h(px(72.))
                    .w_full()
                    .bg(accent)
                    .child(img(path).size_full().object_fit(ObjectFit::Cover)),
                None => div().h(px(40.)).w_full().bg(accent),
            })
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .px_3()
                    .py_2()
                    .when(avatar.is_some() || decoration.is_some(), |row| {
                        row.child(
                            div()
                                .relative()
                                .flex_none()
                                .size(px(40.))
                                .when_some(avatar, |slot, path| {
                                    slot.child(
                                        img(path)
                                            .size_full()
                                            .rounded_full()
                                            .object_fit(ObjectFit::Cover),
                                    )
                                })
                                .when_some(decoration, |slot, path| {
                                    slot.child(
                                        img(path)
                                            .absolute()
                                            .top_0()
                                            .left_0()
                                            .size_full()
                                            .object_fit(ObjectFit::Contain),
                                    )
                                }),
                        )
                    })
                    .child(
                        div()
                            .truncate()
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(name_color)
                            .child(name),
                    ),
            )
    }

    fn render_sidebar_row(
        &self,
        index: usize,
        row: &SidebarRow,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let label: SharedString = row.label.clone().into();
        let base = div()
            .id(index)
            .h(px(28.))
            .w_full()
            .flex()
            .items_center()
            .truncate();
        match row.kind {
            RowKind::Heading => base
                .px_3()
                .font_weight(FontWeight::BOLD)
                .text_color(rgb(TEXT_HEADING))
                .child(label),
            RowKind::Category => base
                .px_3()
                .text_xs()
                .text_color(rgb(TEXT_MUTED))
                .child(label),
            RowKind::Inert => base.px_5().text_color(rgb(TEXT_MUTED)).child(label),
            RowKind::Channel => {
                let selected = row.channel_id.is_some() && row.channel_id == self.selected;
                let channel_id = row.channel_id;
                base.px_5()
                    .cursor_pointer()
                    .text_color(rgb(if selected { TEXT_HEADING } else { TEXT }))
                    .when(selected, |row| row.bg(rgb(BG_SELECTED)))
                    .hover(|style| style.bg(rgb(BG_HOVER)))
                    .on_click(cx.listener(move |view, _event, _window, cx| {
                        if let Some(channel_id) = channel_id {
                            view.select(channel_id);
                            cx.notify();
                        }
                    }))
                    .child(label)
            }
        }
    }

    fn render_message(row: &MessageRow) -> impl IntoElement + use<> {
        let mut body = row.content.clone();
        if row.edited {
            body.push_str(" (edited)");
        }
        div()
            .flex()
            .flex_col()
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(
                        div()
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(color(row.author_color, TEXT_HEADING))
                            .child(SharedString::from(row.author.clone())),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(rgb(TEXT_MUTED))
                            .child(SharedString::from(row.time.clone())),
                    ),
            )
            .when(!body.is_empty(), |message| {
                message.child(div().child(SharedString::from(body)))
            })
            .children(row.attachments.iter().map(|name| {
                div()
                    .text_color(rgb(TEXT_MUTED))
                    .child(SharedString::from(format!("📎 {name}")))
            }))
    }
}

impl Render for ChatView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let row_count = self.sidebar.len();
        let empty_hint: Option<SharedString> = if self.selected.is_none() {
            Some("Pick a channel on the left.".into())
        } else if self.rows.is_empty() {
            Some("No messages loaded yet.".into())
        } else {
            None
        };

        div()
            .flex()
            .size_full()
            .bg(rgb(BG_MAIN))
            .text_color(rgb(TEXT))
            .text_sm()
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_none()
                    .w(px(260.))
                    .h_full()
                    .bg(rgb(BG_SIDEBAR))
                    .child(self.render_profile())
                    .child(
                        uniform_list(
                            "channels",
                            row_count,
                            cx.processor(|view, range: std::ops::Range<usize>, _window, cx| {
                                let mut items = Vec::with_capacity(range.len());
                                for index in range {
                                    if let Some(row) = view.sidebar.get(index).cloned() {
                                        items.push(view.render_sidebar_row(index, &row, cx));
                                    }
                                }
                                items
                            }),
                        )
                        .flex_1(),
                    ),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .h_full()
                    .min_w(px(0.))
                    .child(
                        div()
                            .flex()
                            .flex_none()
                            .items_center()
                            .h(px(44.))
                            .px_4()
                            .bg(rgb(BG_HEADER))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(rgb(TEXT_HEADING))
                            .child(self.title.clone()),
                    )
                    .child(
                        div()
                            .id("messages")
                            .flex()
                            .flex_col()
                            .flex_1()
                            .min_h(px(0.))
                            .gap_3()
                            .p_4()
                            .overflow_y_scroll()
                            .track_scroll(&self.scroll)
                            .when_some(empty_hint, |list, hint| {
                                list.child(div().text_color(rgb(TEXT_MUTED)).child(hint))
                            })
                            .children(self.rows.iter().map(Self::render_message)),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_none()
                            .items_center()
                            .h(px(24.))
                            .px_4()
                            .bg(rgb(BG_HEADER))
                            .text_xs()
                            .text_color(rgb(TEXT_MUTED))
                            .child(self.status.clone()),
                    ),
            )
    }
}

/// Opens the window and runs the UI loop until the window is closed.
///
/// `events` feeds the UI; `on_command` is called (on the UI thread) whenever
/// the UI needs the backend to do something.
pub fn run(
    events: UnboundedReceiver<BackendEvent>,
    overlay: LocalOverlay,
    on_command: impl Fn(UiCommand) + 'static,
) {
    Application::new().run(move |cx: &mut App| {
        let bounds = Bounds::centered(None, size(px(1100.), px(720.)), cx);
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(bounds)),
            titlebar: Some(TitlebarOptions {
                title: Some(WINDOW_TITLE.into()),
                ..Default::default()
            }),
            ..Default::default()
        };
        cx.on_window_closed(|cx| {
            if cx.windows().is_empty() {
                cx.quit();
            }
        })
        .detach();
        let opened = cx.open_window(options, |_window, cx| {
            cx.new(|cx| ChatView::new(events, overlay, Box::new(on_command), cx))
        });
        if opened.is_err() {
            // No display, or the GPU backend could not start.
            cx.quit();
            return;
        }
        cx.activate(true);
    });
}
