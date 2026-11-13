//! The GPUI window. Drawing only: every decision about *what* to show comes
//! from [`crate::model`].
//!
//! Layout (left to right): server rail, channel column with the user bar at
//! its bottom, message pane, and in DMs a profile panel. Clicking an avatar
//! or name opens a profile popup.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;

use dc_core::overlay::{Color, LocalOverlay};
use dc_protocol::Snowflake;
use dc_protocol::model::ChannelType;
use dc_store::{Change, Store};
use futures::StreamExt;
use futures::channel::mpsc::UnboundedReceiver;
use gpui::{
    AnyElement, App, Application, Bounds, ClickEvent, Context, FontWeight, Hsla, Image,
    ImageFormat, ImageSource, ObjectFit, Rgba, ScrollHandle, SharedString, TitlebarOptions, Window,
    WindowBounds, WindowOptions, div, img, linear_color_stop, linear_gradient, prelude::*, px, rgb,
    rgba, size,
};

use crate::model::{self, ImageRef, MessageRow, ProfileView, RailItem, RowKind, Scope, SidebarRow};
use crate::{BackendEvent, UiCommand};

// Dark palette, picked by eye to sit close to the reference screenshots.
const BG_RAIL: u32 = 0x121214;
const BG_COLUMN: u32 = 0x121214;
const BG_PANE: u32 = 0x1a1a1e;
const BG_CARD: u32 = 0x232428;
const BG_INPUT: u32 = 0x222327;
const BG_HOVER: u32 = 0x24252a;
const BG_SELECTED: u32 = 0x2e2f35;
const BG_REPLY_BAR: u32 = 0x4e5058;
const BORDER: u32 = 0x2b2c31;
const TEXT: u32 = 0xdbdee1;
const TEXT_MUTED: u32 = 0x949ba4;
const TEXT_STRONG: u32 = 0xf2f3f5;
const ACCENT: u32 = 0x5865f2;
const DEFAULT_BANNER: u32 = 0x3f4147;

const WINDOW_TITLE: &str = "dc-client";

/// Startup options for [`run`].
#[derive(Clone, Debug, Default)]
pub struct UiOptions {
    pub overlay: LocalOverlay,
    /// Local time zone offset, used for message times.
    pub utc_offset_minutes: i32,
}

fn hex(color: Option<Color>, fallback: u32) -> Rgba {
    rgb(color.map_or(fallback, |c| c.0))
}

/// Scales an RGB color towards black (factor < 1) for gradients.
fn darken(color: u32, factor: f32) -> u32 {
    let scale = |shift: u32| {
        let channel = ((color >> shift) & 0xff) as f32 * factor;
        (channel.clamp(0.0, 255.0) as u32) << shift
    };
    scale(16) | scale(8) | scale(0)
}

/// Picks the decoder from the file's magic bytes.
fn sniff_format(bytes: &[u8]) -> ImageFormat {
    match bytes {
        [0x89, b'P', b'N', b'G', ..] => ImageFormat::Png,
        [0xff, 0xd8, ..] => ImageFormat::Jpeg,
        [b'G', b'I', b'F', ..] => ImageFormat::Gif,
        [
            b'R',
            b'I',
            b'F',
            b'F',
            _,
            _,
            _,
            _,
            b'W',
            b'E',
            b'B',
            b'P',
            ..,
        ] => ImageFormat::Webp,
        _ => ImageFormat::Png,
    }
}

enum Remote {
    Loading,
    Ready(Arc<Image>),
    Failed,
}

pub struct ChatView {
    store: Store,
    overlay: LocalOverlay,
    utc_offset_minutes: i32,
    scope: Scope,
    selected: Option<Snowflake>,
    /// User whose profile popup is open.
    popup: Option<Snowflake>,
    status: SharedString,
    /// Channels whose history has been requested already.
    requested: HashSet<Snowflake>,
    images: HashMap<String, Remote>,
    // Cached view-models, rebuilt only when the store changes.
    rail: Vec<RailItem>,
    sidebar: Vec<SidebarRow>,
    rows: Vec<MessageRow>,
    own: Option<ProfileView>,
    title: SharedString,
    scroll: ScrollHandle,
    on_command: Box<dyn Fn(UiCommand)>,
}

impl ChatView {
    fn new(
        mut events: UnboundedReceiver<BackendEvent>,
        options: UiOptions,
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
            overlay: options.overlay,
            utc_offset_minutes: options.utc_offset_minutes,
            scope: Scope::Home,
            selected: None,
            popup: None,
            status: "Connecting…".into(),
            requested: HashSet::new(),
            images: HashMap::new(),
            rail: Vec::new(),
            sidebar: Vec::new(),
            rows: Vec::new(),
            own: None,
            title: "Select a channel".into(),
            scroll: ScrollHandle::new(),
            on_command,
        }
    }

    // ----------------------------------------------------------------------
    // State
    // ----------------------------------------------------------------------

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
            BackendEvent::Image { url, bytes } => {
                let image = Arc::new(Image::from_bytes(sniff_format(&bytes), bytes));
                self.images.insert(url, Remote::Ready(image));
            }
            BackendEvent::ImageFailed { url } => {
                self.images.insert(url, Remote::Failed);
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
        self.rail = model::rail(&self.store);
        self.sidebar = model::sidebar(&self.store, &self.overlay, self.scope);
        self.own = model::own_profile(&self.store, &self.overlay);
        self.title = model::channel_title(&self.store, &self.overlay, self.selected).into();
    }

    fn refresh_messages(&mut self) {
        self.rows = match self.selected {
            Some(channel_id) => model::messages(
                &self.store,
                &self.overlay,
                channel_id,
                self.utc_offset_minutes,
            ),
            None => Vec::new(),
        };
        self.scroll.scroll_to_bottom();
    }

    fn set_scope(&mut self, scope: Scope) {
        self.scope = scope;
        self.sidebar = model::sidebar(&self.store, &self.overlay, scope);
    }

    fn select(&mut self, channel_id: Snowflake) {
        self.selected = Some(channel_id);
        self.title = model::channel_title(&self.store, &self.overlay, self.selected).into();
        self.refresh_messages();
        if self.requested.insert(channel_id) {
            (self.on_command)(UiCommand::LoadHistory(channel_id));
        }
    }

    /// The other user of the selected one-to-one DM, if that is what is open.
    fn dm_partner(&self) -> Option<Snowflake> {
        let channel = self.store.channel(self.selected?)?;
        match (channel.kind, channel.recipients.as_slice()) {
            (ChannelType::Dm, [user]) => Some(user.id),
            _ => None,
        }
    }

    /// An image source if the image is available, requesting remote images
    /// from the backend the first time they are needed.
    fn image(&mut self, image: &ImageRef) -> Option<ImageSource> {
        match image {
            ImageRef::Local(path) => {
                let path = path.trim();
                (!path.is_empty()).then(|| PathBuf::from(path).into())
            }
            ImageRef::Remote(url) => match self.images.get(url) {
                Some(Remote::Ready(image)) => Some(image.clone().into()),
                Some(Remote::Loading | Remote::Failed) => None,
                None => {
                    self.images.insert(url.clone(), Remote::Loading);
                    (self.on_command)(UiCommand::FetchImage(url.clone()));
                    None
                }
            },
        }
    }

    // ----------------------------------------------------------------------
    // Building blocks
    // ----------------------------------------------------------------------

    /// A round avatar of `size` px, with initials until the image loads.
    fn avatar(&mut self, image: &ImageRef, name: &str, size: f32) -> AnyElement {
        let base = div()
            .flex_none()
            .size(px(size))
            .rounded_full()
            .overflow_hidden()
            .bg(rgb(BG_SELECTED));
        match self.image(image) {
            Some(source) => base
                .child(img(source).size_full().object_fit(ObjectFit::Cover))
                .into_any_element(),
            None => base
                .flex()
                .items_center()
                .justify_center()
                .text_color(rgb(TEXT_STRONG))
                .text_size(px(size * 0.38))
                .child(SharedString::from(model::initials(name)))
                .into_any_element(),
        }
    }

    /// Avatar with its decoration drawn on top, as on profiles.
    fn decorated_avatar(
        &mut self,
        avatar: &ImageRef,
        decoration: Option<&ImageRef>,
        name: &str,
        size: f32,
    ) -> AnyElement {
        let inner = self.avatar(avatar, name, size);
        let decoration = decoration.and_then(|d| self.image(d));
        // Decorations are drawn 20% larger than the avatar, centered on it.
        let overhang = size * 0.1;
        div()
            .relative()
            .flex_none()
            .size(px(size))
            .child(inner)
            .when_some(decoration, |slot, source| {
                slot.child(
                    img(source)
                        .absolute()
                        .top(px(-overhang))
                        .left(px(-overhang))
                        .size(px(size + overhang * 2.0))
                        .object_fit(ObjectFit::Contain),
                )
            })
            .into_any_element()
    }

    // ----------------------------------------------------------------------
    // Regions
    // ----------------------------------------------------------------------

    fn render_rail(&mut self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let home_selected = self.scope == Scope::Home;
        let items = self.rail.clone();
        let current = self.scope;
        let mut icons: Vec<AnyElement> = Vec::with_capacity(items.len());
        for (index, item) in items.iter().enumerate() {
            let selected = current == Scope::Guild(item.guild_id);
            let guild_id = item.guild_id;
            let face = match item.icon.as_ref().and_then(|i| self.image(i)) {
                Some(source) => div()
                    .size_full()
                    .child(img(source).size_full().object_fit(ObjectFit::Cover)),
                None => div()
                    .size_full()
                    .flex()
                    .items_center()
                    .justify_center()
                    .bg(rgb(if selected { ACCENT } else { BG_SELECTED }))
                    .text_color(rgb(TEXT_STRONG))
                    .text_sm()
                    .child(SharedString::from(item.initials.clone())),
            };
            icons.push(
                div()
                    .id(("guild", index))
                    .relative()
                    .flex()
                    .justify_center()
                    .w_full()
                    .cursor_pointer()
                    .when(selected, |row| {
                        row.child(
                            div()
                                .absolute()
                                .left_0()
                                .top(px(4.))
                                .h(px(40.))
                                .w(px(4.))
                                .rounded_r_md()
                                .bg(rgb(TEXT_STRONG)),
                        )
                    })
                    .child(
                        div()
                            .size(px(48.))
                            .overflow_hidden()
                            .when(selected, |icon| icon.rounded_xl())
                            .when(!selected, |icon| icon.rounded_full())
                            .when(item.unavailable, |icon| icon.opacity(0.4))
                            .hover(|style| style.rounded_xl())
                            .child(face),
                    )
                    .on_click(cx.listener(move |view, _: &ClickEvent, _, cx| {
                        view.set_scope(Scope::Guild(guild_id));
                        cx.notify();
                    }))
                    .into_any_element(),
            );
        }

        div()
            .id("rail")
            .flex()
            .flex_col()
            .flex_none()
            .items_center()
            .gap_2()
            .w(px(72.))
            .h_full()
            .py_3()
            .bg(rgb(BG_RAIL))
            .overflow_y_scroll()
            .child(
                div()
                    .id("home")
                    .size(px(48.))
                    .flex_none()
                    .flex()
                    .items_center()
                    .justify_center()
                    .cursor_pointer()
                    .bg(rgb(if home_selected { ACCENT } else { BG_SELECTED }))
                    .when(home_selected, |home| home.rounded_xl())
                    .when(!home_selected, |home| home.rounded_full())
                    .text_color(rgb(TEXT_STRONG))
                    .font_weight(FontWeight::BOLD)
                    .child("DM")
                    .on_click(cx.listener(|view, _: &ClickEvent, _, cx| {
                        view.set_scope(Scope::Home);
                        cx.notify();
                    })),
            )
            .child(
                div()
                    .flex_none()
                    .w(px(32.))
                    .h(px(2.))
                    .rounded_full()
                    .bg(rgb(BORDER)),
            )
            .children(icons)
    }

    fn render_sidebar_row(
        &mut self,
        index: usize,
        row: &SidebarRow,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let label: SharedString = row.label.clone().into();
        let selected = row.channel_id.is_some() && row.channel_id == self.selected;
        match row.kind {
            RowKind::Category => div()
                .pt_4()
                .pb_1()
                .px_2()
                .text_xs()
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(rgb(TEXT_MUTED))
                .truncate()
                .child(label)
                .into_any_element(),
            RowKind::Inert => div()
                .h(px(32.))
                .px_2()
                .flex()
                .items_center()
                .gap_2()
                .text_color(rgb(TEXT_MUTED))
                .opacity(0.6)
                .child(div().w(px(16.)).child("▤"))
                .child(div().truncate().child(label))
                .into_any_element(),
            RowKind::Channel | RowKind::Voice | RowKind::Dm => {
                let channel_id = row.channel_id;
                let leading: AnyElement = match row.kind {
                    RowKind::Dm => match row.avatar.clone() {
                        Some(avatar) => self.avatar(&avatar, &row.label, 32.),
                        None => div()
                            .size(px(32.))
                            .flex_none()
                            .rounded_full()
                            .bg(rgb(ACCENT))
                            .flex()
                            .items_center()
                            .justify_center()
                            .text_xs()
                            .text_color(rgb(TEXT_STRONG))
                            .child(SharedString::from(model::initials(&row.label)))
                            .into_any_element(),
                    },
                    RowKind::Voice => div().w(px(20.)).child("🔊").into_any_element(),
                    _ => div()
                        .w(px(20.))
                        .text_lg()
                        .text_color(rgb(TEXT_MUTED))
                        .child("#")
                        .into_any_element(),
                };
                let height = if row.kind == RowKind::Dm { 44. } else { 32. };
                div()
                    .id(("row", index))
                    .h(px(height))
                    .px_2()
                    .flex()
                    .items_center()
                    .gap_2()
                    .rounded_md()
                    .cursor_pointer()
                    .text_color(rgb(if selected { TEXT_STRONG } else { TEXT_MUTED }))
                    .when(selected, |row| row.bg(rgb(BG_SELECTED)))
                    .hover(|style| style.bg(rgb(BG_HOVER)).text_color(rgb(TEXT)))
                    .child(leading)
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .min_w(px(0.))
                            .child(div().truncate().child(label))
                            .when_some(row.subtitle.clone(), |text, subtitle| {
                                text.child(
                                    div()
                                        .text_xs()
                                        .text_color(rgb(TEXT_MUTED))
                                        .truncate()
                                        .child(SharedString::from(subtitle)),
                                )
                            }),
                    )
                    .on_click(cx.listener(move |view, _: &ClickEvent, _, cx| {
                        if let Some(channel_id) = channel_id {
                            view.select(channel_id);
                            cx.notify();
                        }
                    }))
                    .into_any_element()
            }
        }
    }

    fn render_user_bar(&mut self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let Some(own) = self.own.clone() else {
            return div()
                .flex_none()
                .h(px(56.))
                .m_2()
                .px_3()
                .flex()
                .items_center()
                .rounded_lg()
                .bg(rgb(BG_CARD))
                .text_color(rgb(TEXT_MUTED))
                .child("Not connected");
        };
        let avatar = self.decorated_avatar(&own.avatar, own.decoration.as_ref(), &own.name, 36.);
        let user_id = own.user_id;
        // Like a nameplate: a horizontal gradient from the profile colors.
        let start = own.profile_color.map_or(BG_CARD, |c| c.0);
        let end = own.banner_color.map_or(start, |c| c.0);
        div()
            .flex_none()
            .h(px(56.))
            .m_2()
            .px_2()
            .flex()
            .items_center()
            .gap_2()
            .rounded_lg()
            .border_1()
            .border_color(rgb(BORDER))
            .bg(linear_gradient(
                90.,
                linear_color_stop(rgb(start), 0.),
                linear_color_stop(rgb(darken(end, 0.85)), 1.),
            ))
            .child(
                div()
                    .id("own-avatar")
                    .cursor_pointer()
                    .child(avatar)
                    .on_click(cx.listener(move |view, _: &ClickEvent, _, cx| {
                        view.popup = Some(user_id);
                        cx.notify();
                    })),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .min_w(px(0.))
                    .child(
                        div()
                            .truncate()
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(hex(own.name_color, TEXT_STRONG))
                            .child(SharedString::from(own.name)),
                    )
                    .child(
                        div()
                            .truncate()
                            .text_xs()
                            .text_color(rgb(TEXT_MUTED))
                            .child(SharedString::from(own.username)),
                    ),
            )
    }

    fn render_channel_column(&mut self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let title = model::scope_title(&self.store, self.scope);
        let rows = self.sidebar.clone();
        let mut children: Vec<AnyElement> = Vec::with_capacity(rows.len());
        for (index, row) in rows.iter().enumerate() {
            children.push(self.render_sidebar_row(index, row, cx));
        }
        let empty = children.is_empty();
        let user_bar = self.render_user_bar(cx);

        div()
            .flex()
            .flex_col()
            .flex_none()
            .w(px(260.))
            .h_full()
            .bg(rgb(BG_COLUMN))
            .border_r_1()
            .border_color(rgb(BORDER))
            .child(
                div()
                    .flex_none()
                    .h(px(48.))
                    .px_4()
                    .flex()
                    .items_center()
                    .border_b_1()
                    .border_color(rgb(BORDER))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(rgb(TEXT_STRONG))
                    .truncate()
                    .child(SharedString::from(title)),
            )
            .child(
                div()
                    .id("channels")
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_h(px(0.))
                    .p_2()
                    .gap(px(2.))
                    .overflow_y_scroll()
                    .when(empty, |list| {
                        list.child(
                            div()
                                .p_2()
                                .text_color(rgb(TEXT_MUTED))
                                .child("Nothing here yet."),
                        )
                    })
                    .children(children),
            )
            .child(user_bar)
    }

    fn render_message(
        &mut self,
        index: usize,
        row: &MessageRow,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let author_id = row.author_id;
        let open_profile =
            move |view: &mut Self, _: &ClickEvent, _: &mut Window, cx: &mut Context<Self>| {
                view.popup = Some(author_id);
                cx.notify();
            };
        let mut body = row.content.clone();
        if row.edited {
            body.push_str(" (edited)");
        }
        let text = div()
            .text_color(rgb(TEXT))
            .when(!body.is_empty(), |t| t.child(SharedString::from(body)))
            .children(row.attachments.iter().map(|name| {
                div()
                    .mt_1()
                    .px_2()
                    .py_1()
                    .rounded_md()
                    .bg(rgb(BG_CARD))
                    .text_color(rgb(TEXT_MUTED))
                    .child(SharedString::from(format!("📎 {name}")))
            }));

        if !row.show_header {
            return div()
                .id(("msg", index))
                .pl(px(72.))
                .pr_4()
                .py(px(2.))
                .hover(|style| style.bg(rgb(BG_HOVER)))
                .child(text)
                .into_any_element();
        }

        let avatar = self.avatar(&row.avatar, &row.author, 40.);
        div()
            .id(("msg", index))
            .flex()
            .flex_col()
            .mt_4()
            .px_4()
            .py(px(2.))
            .hover(|style| style.bg(rgb(BG_HOVER)))
            .when_some(row.reply.clone(), |message, reply| {
                message.child(
                    div()
                        .flex()
                        .items_center()
                        .gap_1()
                        .pl(px(20.))
                        .mb_1()
                        .text_xs()
                        .text_color(rgb(TEXT_MUTED))
                        .child(
                            // The curved connector from the avatar to the
                            // replied-to message.
                            div()
                                .flex_none()
                                .w(px(32.))
                                .h(px(10.))
                                .mt(px(8.))
                                .border_l_2()
                                .border_t_2()
                                .rounded_tl_md()
                                .border_color(rgb(BG_REPLY_BAR)),
                        )
                        .child(
                            div()
                                .flex_none()
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(hex(reply.author_color, TEXT))
                                .child(SharedString::from(format!("@{}", reply.author))),
                        )
                        .child(div().truncate().child(SharedString::from(reply.content))),
                )
            })
            .child(
                div()
                    .flex()
                    .gap_4()
                    .child(
                        div()
                            .id(("msg-avatar", index))
                            .cursor_pointer()
                            .child(avatar)
                            .on_click(cx.listener(open_profile)),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .flex_1()
                            .min_w(px(0.))
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap_2()
                                    .child(
                                        div()
                                            .id(("msg-author", index))
                                            .cursor_pointer()
                                            .font_weight(FontWeight::SEMIBOLD)
                                            .text_color(hex(row.author_color, TEXT_STRONG))
                                            .hover(|style| style.underline())
                                            .child(SharedString::from(row.author.clone()))
                                            .on_click(cx.listener(open_profile)),
                                    )
                                    .when(row.is_bot, |line| {
                                        line.child(
                                            div()
                                                .px_1()
                                                .rounded_sm()
                                                .bg(rgb(ACCENT))
                                                .text_xs()
                                                .text_color(rgb(TEXT_STRONG))
                                                .child("APP"),
                                        )
                                    })
                                    .child(
                                        div()
                                            .text_xs()
                                            .text_color(rgb(TEXT_MUTED))
                                            .child(SharedString::from(row.time.clone())),
                                    ),
                            )
                            .child(text),
                    ),
            )
            .into_any_element()
    }

    fn render_message_pane(&mut self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let rows = self.rows.clone();
        let mut messages: Vec<AnyElement> = Vec::with_capacity(rows.len());
        for (index, row) in rows.iter().enumerate() {
            messages.push(self.render_message(index, row, cx));
        }
        let hint: Option<&'static str> = if self.selected.is_none() {
            Some("Pick a server on the left, then a channel or conversation.")
        } else if rows.is_empty() {
            Some("No messages loaded yet.")
        } else {
            None
        };
        let placeholder = match self.selected {
            Some(_) => format!("Message {} — sending is not built yet", self.title),
            None => "Select a channel".to_owned(),
        };

        div()
            .flex()
            .flex_col()
            .flex_1()
            .h_full()
            .min_w(px(0.))
            .bg(rgb(BG_PANE))
            .child(
                div()
                    .flex_none()
                    .h(px(48.))
                    .px_4()
                    .flex()
                    .items_center()
                    .border_b_1()
                    .border_color(rgb(BORDER))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(rgb(TEXT_STRONG))
                    .truncate()
                    .child(self.title.clone()),
            )
            .child(
                div()
                    .id("messages")
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_h(px(0.))
                    .pb_4()
                    .overflow_y_scroll()
                    .track_scroll(&self.scroll)
                    .when_some(hint, |list, hint| {
                        list.child(div().p_4().text_color(rgb(TEXT_MUTED)).child(hint))
                    })
                    .children(messages),
            )
            .child(
                div()
                    .flex_none()
                    .mx_4()
                    .mb_2()
                    .h(px(44.))
                    .px_4()
                    .flex()
                    .items_center()
                    .rounded_lg()
                    .bg(rgb(BG_INPUT))
                    .text_color(rgb(TEXT_MUTED))
                    .truncate()
                    .child(SharedString::from(placeholder)),
            )
            .child(
                div()
                    .flex_none()
                    .h(px(20.))
                    .px_4()
                    .text_xs()
                    .text_color(rgb(TEXT_MUTED))
                    .truncate()
                    .child(self.status.clone()),
            )
    }

    /// A profile card. `compact` is the popup; otherwise the side panel.
    fn render_profile(&mut self, profile: &ProfileView, compact: bool) -> AnyElement {
        let card = profile.profile_color.map_or(BG_CARD, |c| c.0);
        let card_end = profile
            .banner_color
            .map_or(darken(card, 0.7), |c| darken(c.0, 0.55));
        let background = if profile.profile_color.is_some() {
            linear_gradient(
                180.,
                linear_color_stop(rgb(card), 0.),
                linear_color_stop(rgb(card_end), 1.),
            )
        } else {
            linear_gradient(
                180.,
                linear_color_stop(rgb(BG_CARD), 0.),
                linear_color_stop(rgb(BG_CARD), 1.),
            )
        };
        let banner_height = if compact { 105. } else { 120. };
        let avatar_size = 80.;
        let banner_color = profile.banner_color.map_or(DEFAULT_BANNER, |c| c.0);
        let banner_image = profile.banner.as_ref().and_then(|b| self.image(b));
        let avatar = self.decorated_avatar(
            &profile.avatar,
            profile.decoration.as_ref(),
            &profile.name,
            avatar_size,
        );
        let ring: Hsla = rgb(card).into();

        div()
            .flex()
            .flex_col()
            .w_full()
            .rounded_lg()
            .overflow_hidden()
            .bg(background)
            .border_1()
            .border_color(rgb(BORDER))
            .child(
                div()
                    .relative()
                    .w_full()
                    .h(px(banner_height))
                    .bg(rgb(banner_color))
                    .when_some(banner_image, |banner, source| {
                        banner.child(img(source).size_full().object_fit(ObjectFit::Cover))
                    }),
            )
            .child(
                // The avatar overlaps the banner by half its height.
                div().relative().h(px(avatar_size / 2. + 8.)).child(
                    div()
                        .absolute()
                        .left(px(16.))
                        .top(px(-(avatar_size / 2.) - 6.))
                        .p(px(6.))
                        .rounded_full()
                        .bg(ring)
                        .child(avatar),
                ),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .px_4()
                    .pb_4()
                    .child(
                        div()
                            .text_xl()
                            .font_weight(FontWeight::BOLD)
                            .text_color(hex(profile.name_color, TEXT_STRONG))
                            .child(SharedString::from(profile.name.clone())),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .text_sm()
                            .text_color(rgb(TEXT))
                            .child(SharedString::from(profile.username.clone()))
                            .when(profile.is_bot, |line| {
                                line.child(
                                    div()
                                        .px_1()
                                        .rounded_sm()
                                        .bg(rgb(ACCENT))
                                        .text_xs()
                                        .text_color(rgb(TEXT_STRONG))
                                        .child("APP"),
                                )
                            }),
                    )
                    .when_some(profile.bio.clone(), |info, bio| {
                        info.child(
                            div()
                                .mt_3()
                                .p_3()
                                .rounded_md()
                                .bg(rgba(0x0000_0033))
                                .text_sm()
                                .text_color(rgb(TEXT))
                                .child(SharedString::from(bio)),
                        )
                    })
                    .when(profile.customized, |info| {
                        info.child(
                            div()
                                .mt_3()
                                .text_xs()
                                .text_color(rgb(TEXT_MUTED))
                                .child("✦ Local cosmetics — only you see these"),
                        )
                    })
                    .when(profile.is_self, |info| {
                        info.child(
                            div()
                                .mt_3()
                                .p_2()
                                .rounded_md()
                                .bg(rgba(0xffff_ff14))
                                .text_xs()
                                .text_color(rgb(TEXT_MUTED))
                                .child("Edit your look in ~/.config/dc-client/cosmetics.json"),
                        )
                    }),
            )
            .into_any_element()
    }

    fn render_side_panel(&mut self) -> Option<AnyElement> {
        let partner = self.dm_partner()?;
        let profile = model::profile(&self.store, &self.overlay, partner)?;
        let card = self.render_profile(&profile, false);
        Some(
            div()
                .id("side-panel")
                .flex_none()
                .w(px(340.))
                .h_full()
                .p_3()
                .bg(rgb(BG_COLUMN))
                .border_l_1()
                .border_color(rgb(BORDER))
                .overflow_y_scroll()
                .child(card)
                .into_any_element(),
        )
    }

    fn render_popup(&mut self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let user_id = self.popup?;
        let profile = model::profile(&self.store, &self.overlay, user_id)?;
        let card = self.render_profile(&profile, true);
        Some(
            div()
                .absolute()
                .top_0()
                .left_0()
                .size_full()
                .child(
                    // Backdrop: clicking anywhere outside the card closes it.
                    div()
                        .id("popup-backdrop")
                        .absolute()
                        .top_0()
                        .left_0()
                        .size_full()
                        .bg(rgba(0x0000_0066))
                        .on_click(cx.listener(|view, _: &ClickEvent, _, cx| {
                            view.popup = None;
                            cx.notify();
                        })),
                )
                .child(
                    div()
                        .id("popup-card")
                        .absolute()
                        .top(px(80.))
                        .left(px(360.))
                        .w(px(320.))
                        .occlude()
                        .shadow_lg()
                        .child(card),
                )
                .into_any_element(),
        )
    }
}

impl Render for ChatView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let rail = self.render_rail(cx);
        let column = self.render_channel_column(cx);
        let pane = self.render_message_pane(cx);
        let side = self.render_side_panel();
        let popup = self.render_popup(cx);

        div()
            .relative()
            .flex()
            .size_full()
            .bg(rgb(BG_PANE))
            .text_color(rgb(TEXT))
            .text_sm()
            .child(rail)
            .child(column)
            .child(pane)
            .children(side)
            .children(popup)
    }
}

/// Opens the window and runs the UI loop until the window is closed.
///
/// `events` feeds the UI; `on_command` is called (on the UI thread) whenever
/// the UI needs the backend to do something.
pub fn run(
    events: UnboundedReceiver<BackendEvent>,
    options: UiOptions,
    on_command: impl Fn(UiCommand) + 'static,
) {
    Application::new().run(move |cx: &mut App| {
        let bounds = Bounds::centered(None, size(px(1280.), px(760.)), cx);
        let window_options = WindowOptions {
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
        let opened = cx.open_window(window_options, |_window, cx| {
            cx.new(|cx| ChatView::new(events, options, Box::new(on_command), cx))
        });
        if opened.is_err() {
            // No display, or the GPU backend could not start.
            cx.quit();
            return;
        }
        cx.activate(true);
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn darken_scales_each_channel() {
        assert_eq!(darken(0xff8040, 0.5), 0x7f4020);
        assert_eq!(darken(0x123456, 1.0), 0x123456);
        assert_eq!(darken(0xffffff, 0.0), 0);
    }

    #[test]
    fn sniffs_image_formats() {
        assert_eq!(sniff_format(b"\x89PNG\r\n"), ImageFormat::Png);
        assert_eq!(sniff_format(b"\xff\xd8\xff"), ImageFormat::Jpeg);
        assert_eq!(sniff_format(b"GIF89a"), ImageFormat::Gif);
        assert_eq!(sniff_format(b"RIFF\0\0\0\0WEBPVP8"), ImageFormat::Webp);
        assert_eq!(sniff_format(b""), ImageFormat::Png);
    }
}
