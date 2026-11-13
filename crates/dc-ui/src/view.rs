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
    AnyElement, App, Application, Bounds, ClickEvent, Context, Entity, Focusable, FontWeight, Hsla,
    Image, ImageFormat, ImageSource, ObjectFit, Rgba, ScrollHandle, SharedString, TitlebarOptions,
    Window, WindowBounds, WindowOptions, div, img, linear_color_stop, linear_gradient, prelude::*,
    px, rgb, rgba, size,
};

use crate::input::{InputEvent, TextInput};
use crate::model::{self, ImageRef, MessageRow, ProfileView, RailItem, RowKind, Scope, SidebarRow};
use crate::{BackendEvent, UiCommand};
use dc_core::composer::Composer;

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
    pub preferences: dc_core::preferences::Preferences,
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

struct SettingsForm {
    target: Option<Snowflake>,
    fields: Vec<(&'static str, Entity<TextInput>)>,
    error: Option<String>,
    saving: bool,
}

pub struct ChatView {
    store: Store,
    editor: Entity<TextInput>,
    composer: Composer,
    attachments: HashMap<Snowflake, Vec<PathBuf>>,
    show_members: bool,
    member_offset: usize,
    overlay: LocalOverlay,
    preferences: dc_core::preferences::Preferences,
    settings: Option<SettingsForm>,
    editing: Option<(Snowflake, Snowflake)>,
    edit_input: Entity<TextInput>,
    delete_confirm: Option<(Snowflake, Snowflake)>,
    dialog_pending: bool,
    history_loading: bool,
    history_end: HashSet<Snowflake>,
    plugins: Vec<dc_core::plugin::PluginInfo>,
    picker_open: bool,
    emoji_search: Entity<TextInput>,
    utc_offset_minutes: i32,
    scope: Scope,
    last_selected: HashMap<Scope, Snowflake>,
    search: Entity<TextInput>,
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
                    view.apply(event, cx);
                    cx.notify();
                });
                if alive.is_err() {
                    break;
                }
            }
        })
        .detach();

        let search = cx.new(|cx| TextInput::new("Search loaded messages", cx));
        cx.subscribe(&search, |_, _, _, cx| cx.notify()).detach();
        let edit_input = cx.new(|cx| TextInput::new("Edit message", cx));
        let emoji_search = cx.new(|cx| TextInput::new("Search emoji…", cx));
        cx.subscribe(&emoji_search, |_, _, _, cx| cx.notify())
            .detach();
        let editor = cx.new(|cx| TextInput::new("Select a channel", cx));
        cx.subscribe(&editor, |view, editor, event, cx| {
            match event {
                InputEvent::Changed => {
                    if let Some(channel) = view.selected {
                        view.composer
                            .edit(channel, editor.read(cx).value().to_owned());
                    }
                }
                InputEvent::Submit => view.submit(cx),
            }
            cx.notify();
        })
        .detach();
        ChatView {
            editor,
            composer: Composer::default(),
            attachments: HashMap::new(),
            show_members: true,
            member_offset: 0,
            store: Store::new(),
            overlay: options.overlay,
            preferences: options.preferences,
            settings: None,
            editing: None,
            edit_input,
            delete_confirm: None,
            dialog_pending: false,
            history_loading: false,
            history_end: HashSet::new(),
            plugins: Vec::new(),
            picker_open: false,
            emoji_search,
            utc_offset_minutes: options.utc_offset_minutes,
            scope: Scope::Home,
            last_selected: HashMap::new(),
            search,
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

    fn apply(&mut self, event: BackendEvent, cx: &mut Context<Self>) {
        match event {
            BackendEvent::OlderHistory {
                channel_id,
                messages,
            } => {
                self.history_loading = false;
                if messages.len() < 50 {
                    self.history_end.insert(channel_id);
                }
                self.store.insert_history(channel_id, messages);
                if self.selected == Some(channel_id) {
                    self.rows = model::messages(
                        &self.store,
                        &self.overlay,
                        channel_id,
                        self.utc_offset_minutes,
                    );
                }
            }
            BackendEvent::MessageChanged(message) => {
                self.dialog_pending = false;
                let channel_id = message.channel_id;
                self.store.insert_history(channel_id, vec![*message]);
                self.editing = None;
                self.refresh(Change::Messages(channel_id), cx);
            }
            BackendEvent::MessageRemoved {
                channel_id,
                message_id,
            } => {
                self.store
                    .apply(&dc_protocol::gateway::DispatchEvent::MessageDelete(
                        dc_protocol::model::MessageDelete {
                            id: message_id,
                            channel_id,
                            guild_id: None,
                        },
                    ));
                self.delete_confirm = None;
                self.dialog_pending = false;
                self.refresh(Change::Messages(channel_id), cx);
            }
            BackendEvent::OperationFailed(error) => {
                self.dialog_pending = false;
                self.history_loading = false;
                self.status = error.into();
            }

            BackendEvent::Plugins(plugins) => self.plugins = plugins,
            BackendEvent::OverlaySaved { json, error } => {
                if let Some(form) = self.settings.as_mut() {
                    form.saving = false;
                    form.error = error.clone();
                }
                if error.is_none()
                    && let Ok(overlay) = LocalOverlay::from_json(&json)
                {
                    self.overlay = overlay;
                    self.refresh_tree();
                    self.refresh_messages();
                    self.settings = None;
                    self.status = "Cosmetics saved".into();
                }
            }
            BackendEvent::PreferencesSaved { error } => {
                self.status = error.unwrap_or_else(|| "Settings saved".into()).into()
            }
            BackendEvent::Sent {
                channel_id,
                nonce,
                message,
            } => {
                self.store.insert_history(channel_id, vec![*message]);
                if self.composer.complete(channel_id, &nonce, None) {
                    self.attachments.remove(&channel_id);
                }
                if self.selected == Some(channel_id) {
                    self.sync_editor(cx);
                    self.refresh_messages();
                    self.scroll.scroll_to_bottom();
                }
            }
            BackendEvent::SendFailed {
                channel_id,
                nonce,
                error,
            } => {
                self.composer.complete(channel_id, &nonce, Some(error));
                if self.selected == Some(channel_id) {
                    self.sync_editor(cx);
                }
            }
            BackendEvent::Status(status) => self.status = status.into(),
            BackendEvent::History {
                channel_id,
                messages,
            } => {
                self.store.insert_history(channel_id, messages);
                self.refresh(Change::Messages(channel_id), cx);
            }
            BackendEvent::Dispatch(event) => {
                let change = self.store.apply(&event);
                self.refresh(change, cx);
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

    fn refresh(&mut self, change: Change, cx: &mut Context<Self>) {
        match change {
            Change::None => {}
            Change::All => {
                // A new READY: history must be fetched again.
                self.requested.clear();
                self.refresh_tree();
                if let Some(channel_id) = self.selected {
                    self.select(channel_id, cx);
                }
            }
            Change::Guilds => {
                if self
                    .selected
                    .is_some_and(|id| self.store.channel(id).is_none())
                {
                    self.selected = None;
                    self.rows.clear();
                    self.sync_editor(cx);
                }
                self.refresh_tree();
            }
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
        let follow =
            self.rows.is_empty() || self.scroll.bottom_item() + 1 >= self.scroll.children_count();
        self.rows = match self.selected {
            Some(channel_id) => model::messages(
                &self.store,
                &self.overlay,
                channel_id,
                self.utc_offset_minutes,
            ),
            None => Vec::new(),
        };
        if follow {
            self.scroll.scroll_to_bottom();
        }
    }

    fn set_scope(&mut self, scope: Scope, cx: &mut Context<Self>) {
        if self.scope == scope {
            return;
        }
        if let Some(id) = self.selected {
            self.last_selected.insert(self.scope, id);
        }
        self.scope = scope;
        self.sidebar = model::sidebar(&self.store, &self.overlay, scope);
        self.selected = None;
        self.rows.clear();
        self.popup = None;
        self.search.update(cx, |input, cx| input.set_value("", cx));
        if let Some(id) = self
            .last_selected
            .get(&scope)
            .copied()
            .filter(|id| self.store.channel(*id).is_some())
        {
            self.select(id, cx);
        } else {
            self.title = "Select a channel".into();
            self.sync_editor(cx);
        }
    }

    fn select(&mut self, channel_id: Snowflake, cx: &mut Context<Self>) {
        self.selected = Some(channel_id);
        self.title = model::channel_title(&self.store, &self.overlay, self.selected).into();
        self.member_offset = 0;
        self.history_loading = false;
        if let Some(guild) = self.store.channel(channel_id).and_then(|c| c.guild_id) {
            self.store.clear_member_list(guild);
            (self.on_command)(UiCommand::SubscribeMembers {
                guild_id: guild,
                channel_id,
                offset: 0,
            });
        }
        self.sync_editor(cx);
        self.title = model::channel_title(&self.store, &self.overlay, self.selected).into();
        self.refresh_messages();
        self.scroll.scroll_to_bottom();
        if self.requested.insert(channel_id) {
            (self.on_command)(UiCommand::LoadHistory(channel_id));
        }
    }

    fn choose_files(&mut self, cx: &mut Context<Self>) {
        let Some(channel) = self.selected else {
            return;
        };
        if self.composer.draft(channel).pending {
            return;
        }
        let paths = cx.prompt_for_paths(gpui::PathPromptOptions {
            files: true,
            directories: false,
            multiple: true,
            prompt: Some("Attach files".into()),
        });
        cx.spawn(async move |view, cx| {
            let result = paths.await;
            let _ = view.update(cx, |view, cx| {
                match result {
                    Ok(Ok(Some(paths))) => {
                        let files = view.attachments.entry(channel).or_default();
                        for path in paths {
                            if files.len() < 10 && !files.contains(&path) {
                                files.push(path);
                            }
                        }
                    }
                    Ok(Ok(None)) => {}
                    _ => view.status = "Could not open the native file picker".into(),
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn sync_editor(&mut self, cx: &mut Context<Self>) {
        let draft = self
            .selected
            .map(|id| self.composer.draft(id))
            .unwrap_or_default();
        let disabled = self.selected.is_none() || draft.pending;
        let placeholder = match self.selected {
            Some(_) => format!("Message {}", self.title),
            None => "Select a channel".into(),
        };
        self.editor.update(cx, |editor, cx| {
            editor.set_value(draft.content, cx);
            editor.set_placeholder(placeholder, cx);
            editor.disabled = disabled;
        });
    }

    fn submit(&mut self, cx: &mut Context<Self>) {
        let Some(channel) = self.selected else {
            return;
        };
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;
        let attachments = self.attachments.get(&channel).cloned().unwrap_or_default();
        match self
            .composer
            .submit_with_files(channel, now, !attachments.is_empty())
        {
            Ok(message) => {
                (self.on_command)(UiCommand::SendMessage {
                    channel_id: channel,
                    message,
                    attachments,
                });
                self.sync_editor(cx);
            }
            Err(error) => self.status = error.to_owned().into(),
        }
        cx.notify();
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
                        view.set_scope(Scope::Guild(guild_id), cx);
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
                        view.set_scope(Scope::Home, cx);
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
                            view.select(channel_id, cx);
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
            .child(
                div()
                    .id("open-settings")
                    .ml_auto()
                    .px_2()
                    .cursor_pointer()
                    .text_color(rgb(TEXT_MUTED))
                    .child("⚙")
                    .on_click(cx.listener(|view, _, _, cx| {
                        view.open_settings(None, cx);
                        cx.notify();
                    })),
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
        let Some(channel_id) = self.selected else {
            return div().into_any_element();
        };
        let message_id = row.id;
        let own = self
            .store
            .current_user()
            .is_some_and(|u| u.id == row.author_id);
        let content = row.content.clone();
        let actions = div()
            .flex()
            .justify_end()
            .gap_3()
            .text_xs()
            .text_color(rgb(TEXT_MUTED))
            .child(
                div()
                    .id(("copy-message", index))
                    .cursor_pointer()
                    .child("Copy")
                    .on_click(cx.listener(move |view, _, _, cx| {
                        if let Some(message) = view
                            .store
                            .messages(channel_id)
                            .iter()
                            .find(|m| m.id == message_id)
                        {
                            cx.write_to_clipboard(gpui::ClipboardItem::new_string(
                                message.content.clone(),
                            ));
                        }
                    })),
            )
            .child(
                div()
                    .id(("reply-message", index))
                    .cursor_pointer()
                    .child("Reply")
                    .on_click(cx.listener(move |view, _, _, cx| {
                        view.composer.reply(channel_id, Some(message_id));
                        cx.notify();
                    })),
            )
            .when(own, |actions| {
                actions
                    .child(
                        div()
                            .id(("edit-message", index))
                            .cursor_pointer()
                            .child("Edit")
                            .on_click(cx.listener(move |view, _, window, cx| {
                                let content = view
                                    .store
                                    .messages(channel_id)
                                    .iter()
                                    .find(|m| m.id == message_id)
                                    .map(|m| m.content.clone())
                                    .unwrap_or_else(|| content.clone());
                                view.edit_input
                                    .update(cx, |input, cx| input.set_value(content, cx));
                                view.editing = Some((channel_id, message_id));
                                window.focus(&view.edit_input.read(cx).focus_handle(cx));
                                cx.notify();
                            })),
                    )
                    .child(
                        div()
                            .id(("delete-message", index))
                            .cursor_pointer()
                            .text_color(rgb(0xf38b8e))
                            .child("Delete")
                            .on_click(cx.listener(move |view, _, _, cx| {
                                view.delete_confirm = Some((channel_id, message_id));
                                cx.notify();
                            })),
                    )
            });
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
        let rich = dc_core::markdown::parse(&body);
        let styled = gpui::StyledText::new(SharedString::from(rich.text.clone())).with_highlights(
            rich.spans.iter().map(|span| {
                let mut style = gpui::HighlightStyle::default();
                if span.style.bold {
                    style.font_weight = Some(FontWeight::BOLD);
                }
                if span.style.italic {
                    style.font_style = Some(gpui::FontStyle::Italic);
                }
                if span.style.code {
                    style.background_color = Some(rgb(BG_CARD).into());
                    style.color = Some(rgb(0xe3b8d0).into());
                }
                if span.style.strike {
                    style.strikethrough = Some(gpui::StrikethroughStyle {
                        thickness: px(1.),
                        color: None,
                    });
                }
                (span.range.clone(), style)
            }),
        );
        let images: Vec<AnyElement> = row
            .images
            .iter()
            .filter_map(|image| self.image(image))
            .map(|source| {
                div()
                    .mt_2()
                    .max_w_full()
                    .child(
                        img(source)
                            .max_w(px(360.))
                            .max_h(px(240.))
                            .object_fit(ObjectFit::Contain),
                    )
                    .into_any_element()
            })
            .collect();
        let links = rich.links.clone();
        let text = div()
            .text_color(rgb(TEXT))
            .when(!body.is_empty(), |t| {
                t.child(
                    gpui::InteractiveText::new(("message-text", index), styled).on_click(
                        links.iter().map(|(r, _)| r.clone()).collect(),
                        move |index, _, cx| {
                            if let Some((_, url)) = links.get(index) {
                                cx.open_url(url);
                            }
                        },
                    ),
                )
            })
            .children(row.attachments.iter().map(|name| {
                div()
                    .mt_1()
                    .px_2()
                    .py_1()
                    .rounded_md()
                    .bg(rgb(BG_CARD))
                    .text_color(rgb(TEXT_MUTED))
                    .child(SharedString::from(format!("📎 {name}")))
            }))
            .children(images);

        if !row.show_header {
            return div()
                .id(("msg", index))
                .pl(px(72.))
                .pr_4()
                .py(px(2.))
                .hover(|style| style.bg(rgb(BG_HOVER)))
                .child(text)
                .child(actions)
                .into_any_element();
        }

        let avatar = self.avatar(&row.avatar, &row.author, 40.);
        let message_id = row.id;
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
                                    .child(
                                        div()
                                            .id(("reply", index))
                                            .cursor_pointer()
                                            .text_xs()
                                            .text_color(rgb(TEXT_MUTED))
                                            .child("Reply")
                                            .on_click(cx.listener(move |view, _, _, cx| {
                                                if let Some(channel) = view.selected {
                                                    view.composer.reply(channel, Some(message_id));
                                                    cx.notify();
                                                }
                                            })),
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
            .child(actions)
            .into_any_element()
    }

    fn render_message_pane(&mut self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let rows = model::filter_messages(&self.rows, self.search.read(cx).value());
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
        let draft = self
            .selected
            .map(|id| self.composer.draft(id))
            .unwrap_or_default();
        let can_send =
            self.selected.is_some() && !draft.pending && !draft.content.trim().is_empty();

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
                    .child(self.title.clone())
                    .child(
                        div()
                            .id("jump-latest")
                            .px_2()
                            .cursor_pointer()
                            .text_xs()
                            .text_color(rgb(TEXT_MUTED))
                            .child("Latest")
                            .on_click(cx.listener(|view, _, _, cx| {
                                view.scroll.scroll_to_bottom();
                                cx.notify();
                            })),
                    )
                    .child(
                        div()
                            .ml_auto()
                            .w(px(220.))
                            .p_1()
                            .rounded_md()
                            .bg(rgb(BG_INPUT))
                            .child(self.search.clone()),
                    )
                    .child(
                        div()
                            .id("toggle-members")
                            .ml_auto()
                            .px_2()
                            .cursor_pointer()
                            .text_xs()
                            .text_color(rgb(TEXT_MUTED))
                            .child("Members")
                            .on_click(cx.listener(|view, _, _, cx| {
                                view.show_members = !view.show_members;
                                cx.notify();
                            })),
                    ),
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
                    .child(
                        div()
                            .id("load-older")
                            .px_4()
                            .py_2()
                            .text_xs()
                            .text_color(rgb(TEXT_MUTED))
                            .cursor_pointer()
                            .child(if self.history_loading {
                                "Loading older messages…"
                            } else {
                                "Load older messages"
                            })
                            .on_click(cx.listener(|view, _, _, cx| {
                                if view.history_loading {
                                    return;
                                }
                                if let Some(channel_id) = view.selected
                                    && !view.history_end.contains(&channel_id)
                                    && view.store.messages(channel_id).len() < 500
                                    && let Some(before) =
                                        view.store.messages(channel_id).first().map(|m| m.id)
                                {
                                    view.history_loading = true;
                                    (view.on_command)(UiCommand::LoadOlder { channel_id, before });
                                    cx.notify();
                                }
                            })),
                    )
                    .children(messages),
            )
            .children(
                self.selected
                    .and_then(|id| self.attachments.get(&id))
                    .cloned()
                    .unwrap_or_default()
                    .iter()
                    .enumerate()
                    .map(|(index, path)| {
                        div()
                            .id(("pending-file", index))
                            .mx_4()
                            .text_xs()
                            .text_color(rgb(TEXT_MUTED))
                            .cursor_pointer()
                            .child(SharedString::from(format!(
                                "📎 {} · click to remove",
                                path.file_name().unwrap_or_default().to_string_lossy()
                            )))
                            .on_click(cx.listener(move |view, _, _, cx| {
                                if let Some(channel) = view.selected
                                    && !view.composer.draft(channel).pending
                                    && let Some(files) = view.attachments.get_mut(&channel)
                                    && index < files.len()
                                {
                                    files.remove(index);
                                    cx.notify();
                                }
                            }))
                            .into_any_element()
                    }),
            )
            .when_some(draft.error.clone(), |pane, error| {
                pane.child(
                    div()
                        .mx_4()
                        .mb_1()
                        .text_xs()
                        .text_color(rgb(0xf38b8e))
                        .child(SharedString::from(format!(
                            "Not sent: {error}. Your draft is preserved; use Send to retry."
                        ))),
                )
            })
            .when_some(draft.reply, |pane, id| {
                pane.child(
                    div()
                        .mx_4()
                        .mb_1()
                        .flex()
                        .justify_between()
                        .text_xs()
                        .text_color(rgb(TEXT_MUTED))
                        .child(SharedString::from(format!("Replying to message {id}")))
                        .child(
                            div()
                                .id("cancel-reply")
                                .cursor_pointer()
                                .child("Cancel")
                                .on_click(cx.listener(|view, _, _, cx| {
                                    if let Some(channel) = view.selected {
                                        view.composer.reply(channel, None);
                                        cx.notify();
                                    }
                                })),
                        ),
                )
            })
            .child(
                div()
                    .flex_none()
                    .mx_4()
                    .mb_1()
                    .p_2()
                    .flex()
                    .items_end()
                    .gap_2()
                    .rounded_lg()
                    .bg(rgb(BG_INPUT))
                    .child(
                        div()
                            .id("attach-files")
                            .px_2()
                            .py_2()
                            .cursor_pointer()
                            .child("+")
                            .on_click(cx.listener(|view, _, _, cx| view.choose_files(cx))),
                    )
                    .child(
                        div()
                            .id("open-emoji")
                            .px_2()
                            .py_2()
                            .cursor_pointer()
                            .child("☺")
                            .on_click(cx.listener(|view, _, window, cx| {
                                view.picker_open = !view.picker_open;
                                if view.picker_open {
                                    window.focus(&view.emoji_search.read(cx).focus_handle(cx));
                                }
                                cx.notify();
                            })),
                    )
                    .child(div().flex_1().min_w(px(0.)).child(self.editor.clone()))
                    .child(
                        div()
                            .id("send-message")
                            .flex_none()
                            .px_3()
                            .py_2()
                            .rounded_md()
                            .bg(rgb(if can_send { ACCENT } else { BG_SELECTED }))
                            .text_color(rgb(TEXT_STRONG))
                            .when(can_send, |b| b.cursor_pointer())
                            .child(if draft.pending { "Sending…" } else { "Send" })
                            .on_click(cx.listener(|view, _, _, cx| {
                                view.submit(cx);
                            })),
                    ),
            )
            .child(
                div()
                    .flex_none()
                    .mx_4()
                    .mb_1()
                    .text_xs()
                    .text_color(rgb(TEXT_MUTED))
                    .child(SharedString::from(format!(
                        "Enter to send · Shift+Enter for a new line · {}/2000",
                        draft.content.encode_utf16().count()
                    ))),
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
    fn render_profile(
        &mut self,
        profile: &ProfileView,
        compact: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
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
            .child(
                div()
                    .id(("customize-profile", profile.user_id.get()))
                    .mx_4()
                    .mb_4()
                    .p_2()
                    .rounded_md()
                    .bg(rgba(0xffffff14))
                    .cursor_pointer()
                    .text_sm()
                    .child("Customize this profile locally")
                    .on_click(cx.listener({
                        let user = profile.user_id;
                        let own = profile.is_self;
                        move |view, _, _, cx| {
                            view.open_settings(if own { None } else { Some(user) }, cx);
                            cx.notify();
                        }
                    })),
            )
            .into_any_element()
    }

    fn open_settings(&mut self, target: Option<Snowflake>, cx: &mut Context<Self>) {
        let current = match target {
            Some(id) => self.overlay.users.get(&id).cloned().unwrap_or_default(),
            None => self.overlay.me.clone(),
        };
        let value = serde_json::to_value(current).unwrap_or_default();
        let fields = [
            "display_name",
            "name_color",
            "accent_color",
            "profile_color",
            "bio",
            "banner",
            "avatar",
            "avatar_decoration",
        ]
        .into_iter()
        .map(|key| {
            let text = value
                .get(key)
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_owned();
            let editor = cx.new(|cx| {
                let mut input = TextInput::new(key, cx);
                input.set_value(text, cx);
                input
            });
            (key, editor)
        })
        .collect();
        self.settings = Some(SettingsForm {
            target,
            fields,
            error: None,
            saving: false,
        });
        self.popup = None;
    }
    fn save_cosmetics(&mut self, cx: &mut Context<Self>) {
        let Some(form) = self.settings.as_mut() else {
            return;
        };
        if form.saving {
            return;
        }
        let mut value = serde_json::Map::new();
        for (key, input) in &form.fields {
            let text = input.read(cx).value();
            if !text.is_empty() {
                value.insert((*key).into(), serde_json::Value::String(text.to_owned()));
            }
        }
        match serde_json::from_value::<dc_core::ProfileOverride>(serde_json::Value::Object(value)) {
            Ok(profile) => {
                let mut overlay = self.overlay.clone();
                if let Some(id) = form.target {
                    overlay.users.insert(id, profile);
                } else {
                    overlay.me = profile;
                }
                form.saving = true;
                (self.on_command)(UiCommand::SaveOverlay(overlay.to_json()));
            }
            Err(error) => form.error = Some(error.to_string()),
        }
        cx.notify();
    }
    fn render_settings(&mut self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let form = self.settings.as_ref()?;
        let title = form
            .target
            .map(|id| format!("Local profile for {id}"))
            .unwrap_or_else(|| "Your local profile".into());
        let mut fields: Vec<AnyElement> = Vec::new();
        for (key, input) in &form.fields {
            fields.push(
                div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(div().text_xs().text_color(rgb(TEXT_MUTED)).child(*key))
                    .child(
                        div()
                            .p_2()
                            .rounded_md()
                            .bg(rgb(BG_INPUT))
                            .child(input.clone()),
                    )
                    .into_any_element(),
            );
        }
        fields.push(
            div()
                .id("clear-history-cache")
                .p_2()
                .rounded_md()
                .bg(rgb(BG_INPUT))
                .cursor_pointer()
                .child("Erase local cached history (keeps Discord messages)")
                .on_click(cx.listener(|view, _, _, cx| {
                    (view.on_command)(UiCommand::ClearHistoryCache);
                    cx.notify();
                }))
                .into_any_element(),
        );
        for info in &self.plugins {
            let id = info.manifest.id.clone();
            let granted = self.preferences.plugin_permissions.contains_key(&id);
            let label = format!(
                "{} · {}",
                info.manifest.name,
                if granted {
                    "Enabled: reads and changes submitted text"
                } else {
                    "Disabled: enable outgoing text transform"
                }
            );
            fields.push(
                div()
                    .id(SharedString::from(format!("plugin-toggle-{id}")))
                    .p_2()
                    .rounded_md()
                    .bg(rgb(BG_INPUT))
                    .cursor_pointer()
                    .child(SharedString::from(label))
                    .when_some(info.error.clone(), |row, error| {
                        row.child(
                            div()
                                .text_xs()
                                .text_color(rgb(0xf38b8e))
                                .child(SharedString::from(error)),
                        )
                    })
                    .on_click(cx.listener(move |view, _, _, cx| {
                        if view.preferences.plugin_permissions.remove(&id).is_none() {
                            view.preferences
                                .plugin_permissions
                                .insert(id.clone(), vec!["outgoing-transform".into()]);
                        }
                        cx.notify();
                    }))
                    .into_any_element(),
            );
        }
        let error = form.error.clone();
        let saving = form.saving;
        Some(
            div()
                .absolute()
                .top_0()
                .left_0()
                .size_full()
                .flex()
                .items_center()
                .justify_center()
                .child(
                    div()
                        .id("settings-backdrop")
                        .absolute()
                        .top_0()
                        .left_0()
                        .size_full()
                        .bg(rgba(0x00000099))
                        .on_click(cx.listener(|view, _, _, cx| {
                            if !view.settings.as_ref().is_some_and(|f| f.saving) {
                                view.settings = None;
                            }
                            cx.notify();
                        })),
                )
                .child(
                    div()
                        .id("settings-card")
                        .relative()
                        .occlude()
                        .w(px(680.))
                        .max_w_full()
                        .max_h_full()
                        .m_4()
                        .rounded_lg()
                        .bg(rgb(BG_CARD))
                        .flex()
                        .flex_col()
                        .child(
                            div()
                                .p_4()
                                .text_lg()
                                .font_weight(FontWeight::BOLD)
                                .child(SharedString::from(title)),
                        )
                        .child(
                            div()
                                .id("settings-fields")
                                .min_h(px(0.))
                                .overflow_y_scroll()
                                .p_4()
                                .flex()
                                .flex_col()
                                .gap_3()
                                .children(fields)
                                .child(
                                    div()
                                        .mt_3()
                                        .text_xs()
                                        .text_color(rgb(TEXT_MUTED))
                                        .child("Appearance & behavior"),
                                )
                                .child(
                                    div()
                                        .id("toggle-fake-nitro")
                                        .p_2()
                                        .rounded_md()
                                        .bg(rgb(BG_INPUT))
                                        .cursor_pointer()
                                        .child(SharedString::from(format!(
                                            "Custom emoji as image links: {}",
                                            if self.preferences.fake_nitro {
                                                "On"
                                            } else {
                                                "Off"
                                            }
                                        )))
                                        .on_click(cx.listener(|view, _, _, cx| {
                                            view.preferences.fake_nitro =
                                                !view.preferences.fake_nitro;
                                            cx.notify();
                                        })),
                                )
                                .child(
                                    div()
                                        .id("toggle-history-cache")
                                        .p_2()
                                        .rounded_md()
                                        .bg(rgb(BG_INPUT))
                                        .cursor_pointer()
                                        .child(SharedString::from(format!(
                                            "Save message history on this device: {}",
                                            if self.preferences.cache_history {
                                                "On"
                                            } else {
                                                "Off"
                                            }
                                        )))
                                        .on_click(cx.listener(|view, _, _, cx| {
                                            view.preferences.cache_history =
                                                !view.preferences.cache_history;
                                            cx.notify();
                                        })),
                                )
                                .child(
                                    div()
                                        .flex()
                                        .gap_2()
                                        .child(
                                            div()
                                                .id("font-smaller")
                                                .cursor_pointer()
                                                .child("A−")
                                                .on_click(cx.listener(|view, _, _, cx| {
                                                    view.preferences.font_size = view
                                                        .preferences
                                                        .font_size
                                                        .saturating_sub(1)
                                                        .max(11);
                                                    cx.notify();
                                                })),
                                        )
                                        .child(SharedString::from(format!(
                                            "Text size: {}",
                                            self.preferences.font_size
                                        )))
                                        .child(
                                            div()
                                                .id("font-larger")
                                                .cursor_pointer()
                                                .child("A+")
                                                .on_click(cx.listener(|view, _, _, cx| {
                                                    view.preferences.font_size =
                                                        (view.preferences.font_size + 1).min(24);
                                                    cx.notify();
                                                })),
                                        ),
                                )
                                .when_some(error, |form, error| {
                                    form.child(
                                        div()
                                            .text_color(rgb(0xf38b8e))
                                            .child(SharedString::from(error)),
                                    )
                                }),
                        )
                        .child(
                            div()
                                .flex()
                                .gap_3()
                                .p_4()
                                .child(
                                    div()
                                        .id("save-cosmetics")
                                        .px_3()
                                        .py_2()
                                        .rounded_md()
                                        .bg(rgb(ACCENT))
                                        .cursor_pointer()
                                        .child(if saving { "Saving…" } else { "Save profile" })
                                        .on_click(
                                            cx.listener(|view, _, _, cx| view.save_cosmetics(cx)),
                                        ),
                                )
                                .child(
                                    div()
                                        .id("save-preferences")
                                        .px_3()
                                        .py_2()
                                        .rounded_md()
                                        .bg(rgb(BG_SELECTED))
                                        .cursor_pointer()
                                        .child("Save appearance")
                                        .on_click(cx.listener(|view, _, _, cx| {
                                            (view.on_command)(UiCommand::SavePreferences(
                                                view.preferences.clone(),
                                            ));
                                            cx.notify();
                                        })),
                                ),
                        ),
                )
                .into_any_element(),
        )
    }
    fn render_emoji_picker(&mut self, cx: &mut Context<Self>) -> Option<AnyElement> {
        if !self.picker_open {
            return None;
        }
        let query = self.emoji_search.read(cx).value().to_lowercase();
        let mut tiles = Vec::new();
        for (index, (emoji, _name)) in dc_core::emoji::UNICODE
            .iter()
            .enumerate()
            .filter(|(_, (_, name))| name.contains(&query))
        {
            let text = (*emoji).to_owned();
            tiles.push(
                div()
                    .id(("unicode-emoji", index))
                    .size(px(40.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .text_xl()
                    .rounded_md()
                    .cursor_pointer()
                    .hover(|s| s.bg(rgb(BG_SELECTED)))
                    .child(*emoji)
                    .on_click(cx.listener(move |view, _, window, cx| {
                        view.editor
                            .update(cx, |editor, cx| editor.insert(&text, window, cx));
                        window.focus(&view.editor.read(cx).focus_handle(cx));
                        view.picker_open = false;
                        cx.notify();
                    }))
                    .into_any_element(),
            );
        }
        let custom: Vec<_> = self
            .store
            .guilds()
            .iter()
            .flat_map(|guild| {
                guild
                    .emojis
                    .iter()
                    .map(move |emoji| (guild.id, emoji.clone()))
            })
            .filter(|(_, e)| e.available && e.name.to_lowercase().contains(&query))
            .collect();
        for (index, (_, emoji)) in custom.into_iter().enumerate() {
            let source = self.image(&ImageRef::Remote(dc_core::emoji::url(
                emoji.id,
                emoji.animated,
                64,
            )));
            let insertion = dc_core::emoji::insertion(
                &emoji,
                self.preferences.fake_nitro,
                self.preferences.emoji_size,
            );
            let label = emoji.name;
            tiles.push(
                div()
                    .id(("custom-emoji", index))
                    .size(px(40.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .text_xs()
                    .rounded_md()
                    .cursor_pointer()
                    .hover(|s| s.bg(rgb(BG_SELECTED)))
                    .when_some(source, |tile, source| {
                        tile.child(img(source).size(px(32.)).object_fit(ObjectFit::Contain))
                    })
                    .child(SharedString::from(label))
                    .on_click(cx.listener(move |view, _, window, cx| {
                        let text = format!("{insertion} ");
                        view.editor
                            .update(cx, |editor, cx| editor.insert(&text, window, cx));
                        window.focus(&view.editor.read(cx).focus_handle(cx));
                        view.picker_open = false;
                        cx.notify();
                    }))
                    .into_any_element(),
            );
        }
        Some(
            div()
                .id("emoji-picker")
                .absolute()
                .bottom(px(100.))
                .right(px(16.))
                .w(px(380.))
                .h(px(420.))
                .occlude()
                .shadow_lg()
                .rounded_lg()
                .border_1()
                .border_color(rgb(BORDER))
                .bg(rgb(BG_CARD))
                .flex()
                .flex_col()
                .child(div().p_3().font_weight(FontWeight::BOLD).child("Emoji"))
                .child(
                    div()
                        .mx_3()
                        .p_2()
                        .rounded_md()
                        .bg(rgb(BG_INPUT))
                        .child(self.emoji_search.clone()),
                )
                .child(
                    div()
                        .id("emoji-grid")
                        .flex_1()
                        .min_h(px(0.))
                        .overflow_y_scroll()
                        .p_3()
                        .flex()
                        .flex_wrap()
                        .gap_1()
                        .children(tiles),
                )
                .child(div().p_3().text_xs().text_color(rgb(TEXT_MUTED)).child(
                    if self.preferences.fake_nitro {
                        "Custom emoji insert as image links · only sent when you press Send"
                    } else {
                        "Custom emoji use normal syntax · server permissions still apply"
                    },
                ))
                .into_any_element(),
        )
    }
    fn render_risk_notice(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        if self.preferences.risk_accepted {
            return None;
        }
        Some(div().absolute().top_0().left_0().size_full().bg(rgba(0x000000dd)).flex().items_center().justify_center().occlude().child(div().w(px(520.)).max_w_full().m_4().p_5().rounded_lg().bg(rgb(BG_CARD)).flex().flex_col().gap_4().child(div().text_xl().font_weight(FontWeight::BOLD).child("Welcome to dc-client ✦")).child("This is an unofficial Discord client. Third-party clients violate Discord’s Terms of Service and can put your account at risk. Continue only if you accept that risk.").child("Cosmetics only change your view. Message history caching is off by default. The client does not collect telemetry.").child(div().id("accept-risk").p_3().rounded_md().bg(rgb(ACCENT)).cursor_pointer().child("I understand — continue").on_click(cx.listener(|view, _, _, cx| { view.preferences.risk_accepted=true; (view.on_command)(UiCommand::SavePreferences(view.preferences.clone())); cx.notify(); })))).into_any_element())
    }

    fn render_message_dialog(&mut self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let editing = self.editing;
        let (channel_id, message_id) = editing.or(self.delete_confirm)?;
        let edit = editing.is_some();
        Some(
            div()
                .absolute()
                .top_0()
                .left_0()
                .size_full()
                .bg(rgba(0x00000099))
                .flex()
                .items_center()
                .justify_center()
                .occlude()
                .child(
                    div()
                        .w(px(560.))
                        .max_w_full()
                        .m_4()
                        .p_4()
                        .rounded_lg()
                        .bg(rgb(BG_CARD))
                        .flex()
                        .flex_col()
                        .gap_3()
                        .child(if edit {
                            "Edit message"
                        } else {
                            "Delete this message? This removes it for everyone."
                        })
                        .when(edit, |card| {
                            card.child(
                                div()
                                    .p_2()
                                    .bg(rgb(BG_INPUT))
                                    .rounded_md()
                                    .child(self.edit_input.clone()),
                            )
                        })
                        .child(
                            div()
                                .flex()
                                .gap_3()
                                .child(
                                    div()
                                        .id("confirm-message-change")
                                        .p_2()
                                        .rounded_md()
                                        .bg(rgb(if edit { ACCENT } else { 0xda373c }))
                                        .cursor_pointer()
                                        .child(if edit {
                                            "Save changes"
                                        } else {
                                            "Delete message"
                                        })
                                        .on_click(cx.listener(move |view, _, _, cx| {
                                            if view.dialog_pending {
                                                return;
                                            }
                                            view.dialog_pending = true;
                                            let command = if edit {
                                                UiCommand::EditMessage {
                                                    channel_id,
                                                    message_id,
                                                    content: view
                                                        .edit_input
                                                        .read(cx)
                                                        .value()
                                                        .to_owned(),
                                                }
                                            } else {
                                                UiCommand::DeleteMessage {
                                                    channel_id,
                                                    message_id,
                                                }
                                            };
                                            (view.on_command)(command);
                                            view.status = "Updating message…".into();
                                            cx.notify();
                                        })),
                                )
                                .child(
                                    div()
                                        .id("cancel-message-change")
                                        .p_2()
                                        .cursor_pointer()
                                        .child("Cancel")
                                        .on_click(cx.listener(|view, _, _, cx| {
                                            view.editing = None;
                                            view.delete_confirm = None;
                                            cx.notify();
                                        })),
                                ),
                        ),
                )
                .into_any_element(),
        )
    }

    fn render_members(&mut self, cx: &mut Context<Self>) -> Option<AnyElement> {
        if !self.show_members {
            return None;
        }
        let guild_id = self.store.channel(self.selected?)?.guild_id?;
        let guild = self.store.guild(guild_id)?.clone();
        let list = self.store.member_list(guild_id).cloned();
        let mut rows: Vec<AnyElement> = Vec::new();
        if let Some(list) = list {
            for (index, item) in list
                .slots
                .into_iter()
                .enumerate()
                .filter_map(|(i, item)| item.map(|item| (i, item)))
            {
                if let Some(group) = item.group {
                    let label = group
                        .id
                        .parse::<Snowflake>()
                        .ok()
                        .and_then(|id| guild.roles.iter().find(|r| r.id == id))
                        .map(|r| r.name.clone())
                        .unwrap_or(group.id);
                    rows.push(
                        div()
                            .mt_4()
                            .px_2()
                            .text_xs()
                            .text_color(rgb(TEXT_MUTED))
                            .child(SharedString::from(format!(
                                "{} — {}",
                                label.to_uppercase(),
                                group.count
                            )))
                            .into_any_element(),
                    );
                }
                if let Some(member) = item.member {
                    let profile = model::profile_of(&member.user, &self.store, &self.overlay);
                    let color = profile.name_color.or_else(|| {
                        guild
                            .roles
                            .iter()
                            .filter(|role| member.roles.contains(&role.id) && role.color != 0)
                            .max_by_key(|r| r.position)
                            .map(|r| Color(r.color))
                    });
                    let user_id = member.user.id;
                    let avatar = self.avatar(&profile.avatar, &profile.name, 32.);
                    let status = item
                        .presence
                        .map(|p| p.status)
                        .unwrap_or_else(|| "offline".into());
                    let status_color = match status.as_str() {
                        "online" => 0x23a55a,
                        "idle" => 0xf0b232,
                        "dnd" => 0xf23f43,
                        _ => TEXT_MUTED,
                    };
                    rows.push(
                        div()
                            .id(("member", index))
                            .flex()
                            .items_center()
                            .gap_2()
                            .p_2()
                            .rounded_md()
                            .cursor_pointer()
                            .hover(|s| s.bg(rgb(BG_HOVER)))
                            .child(avatar)
                            .child(
                                div()
                                    .flex_1()
                                    .min_w(px(0.))
                                    .text_color(hex(color, TEXT))
                                    .child(SharedString::from(profile.name))
                                    .child(
                                        div()
                                            .text_xs()
                                            .text_color(rgb(status_color))
                                            .child(SharedString::from(status)),
                                    ),
                            )
                            .on_click(cx.listener(move |view, _, _, cx| {
                                view.popup = Some(user_id);
                                cx.notify();
                            }))
                            .into_any_element(),
                    );
                }
            }
        }
        if rows.is_empty() {
            rows.push(
                div()
                    .p_3()
                    .text_color(rgb(TEXT_MUTED))
                    .child("Waiting for member data…")
                    .into_any_element(),
            );
        }
        let range = format!(
            "Members {}–{}",
            self.member_offset + 1,
            self.member_offset + 100
        );
        Some(
            div()
                .id("members")
                .flex_none()
                .w(px(240.))
                .h_full()
                .flex()
                .flex_col()
                .bg(rgb(BG_COLUMN))
                .child(
                    div()
                        .p_3()
                        .text_xs()
                        .text_color(rgb(TEXT_MUTED))
                        .child(SharedString::from(range)),
                )
                .child(
                    div()
                        .id("member-rows")
                        .flex_1()
                        .min_h(px(0.))
                        .overflow_y_scroll()
                        .p_2()
                        .children(rows),
                )
                .child(
                    div()
                        .flex()
                        .justify_between()
                        .p_2()
                        .text_xs()
                        .text_color(rgb(TEXT_MUTED))
                        .child(
                            div()
                                .id("members-prev")
                                .cursor_pointer()
                                .child("Previous")
                                .on_click(cx.listener(|view, _, _, cx| {
                                    view.member_offset = view.member_offset.saturating_sub(100);
                                    view.request_member_page();
                                    cx.notify();
                                })),
                        )
                        .child(
                            div()
                                .id("members-next")
                                .cursor_pointer()
                                .child("Next")
                                .on_click(cx.listener(|view, _, _, cx| {
                                    view.member_offset = (view.member_offset + 100).min(1900);
                                    view.request_member_page();
                                    cx.notify();
                                })),
                        ),
                )
                .into_any_element(),
        )
    }
    fn request_member_page(&mut self) {
        if let Some(channel) = self.selected.and_then(|id| self.store.channel(id))
            && let Some(guild_id) = channel.guild_id
        {
            (self.on_command)(UiCommand::SubscribeMembers {
                guild_id,
                channel_id: channel.id,
                offset: self.member_offset,
            });
        }
    }

    fn render_side_panel(&mut self, cx: &mut Context<Self>) -> Option<AnyElement> {
        if self.dm_partner().is_none() {
            return self.render_members(cx);
        }
        let partner = self.dm_partner()?;
        let profile = model::profile(&self.store, &self.overlay, partner)?;
        let card = self.render_profile(&profile, false, cx);
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
        let card = self.render_profile(&profile, true, cx);
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
        let side = self.render_side_panel(cx);
        let popup = self.render_popup(cx);
        let picker = self.render_emoji_picker(cx);
        let settings = self.render_settings(cx);
        let message_dialog = self.render_message_dialog(cx);
        let risk = self.render_risk_notice(cx);

        div()
            .relative()
            .flex()
            .size_full()
            .bg(rgb(BG_PANE))
            .text_color(rgb(TEXT))
            .text_size(px(f32::from(self.preferences.font_size)))
            .child(rail)
            .child(column)
            .child(pane)
            .children(side)
            .children(picker)
            .children(popup)
            .children(settings)
            .children(message_dialog)
            .children(risk)
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
        crate::input::init(cx);
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
