//! Account card, system folders and labels.

use std::collections::{HashMap, HashSet};

use gpui::{
    Action, AnyElement, AppContext as _, Context, FontWeight, InteractiveElement, IntoElement,
    ParentElement, StatefulInteractiveElement, Styled, Window,
    component::{native_menu::NativeMenu, scroll::ScrollableElement as _},
    div,
    prelude::FluentBuilder as _,
};

use super::{
    HEADER_HEIGHT, account_avatar, icon, icon_button, is_secondary, motion, status::thousands,
    tooltip, window_drag_area,
};
use crate::ui::style::*;
use crate::{
    actions::{
        GoToArchive, GoToDrafts, GoToForums, GoToInbox, GoToPromotions, GoToScheduled, GoToSent,
        GoToSnoozed, GoToSocial, GoToSpam, GoToStarred, GoToTrash, GoToUpdates, NewLabel,
        OpenSettingsSection, SignIn, ToggleHiddenLabels, ToggleSidebar,
    },
    app::{Nav, PigeonApp, nav_holds},
    assets::Icon,
    backend::{
        Source,
        categories::{Category, CategoryCounts, in_category},
        gmail,
        labels::label_tree,
    },
    data::{Contact, Folder, Label, ListVisibility, Thread},
    settings::Section,
    theme::Theme,
    ui::thread_list::DraggedThreads,
};

/// W4b: the Inbox's other categories, as views under it (FOL-11).
const CATEGORIES: [(Category, Icon); 4] = [
    (Category::Social, Icon::Users),
    (Category::Promotions, Icon::Megaphone),
    (Category::Updates, Icon::Bell),
    (Category::Forums, Icon::MessagesSquare),
];

/// The categories the sidebar lists: those with threads in the Inbox or
/// unread ones on the server, and the one open.
pub fn shown_categories(nav: &NavCounts, counts: &CategoryCounts, open: Nav) -> Vec<Category> {
    CATEGORIES
        .iter()
        .map(|&(c, _)| c)
        .filter(|&c| open == Nav::Category(c) || counts.unread(c) > 0 || nav.in_inbox[c as usize])
        .collect()
}

/// Folded-label id that stands for the whole Labels section (MB-074),
/// saved with the folded labels. No Gmail label id looks like it.
const LABELS_SECTION: &str = "\u{1}labels";

/// Every view's count from one pass over the threads (MC-037): unread
/// threads per view, Drafts' total, and which Inbox categories hold mail.
/// Built once per frame and read by the sidebar, list header and rail.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct NavCounts {
    /// Unread threads by `slot`: the cached ones', or Gmail's count moved
    /// by what changed here (`PigeonApp::nav_counts`).
    unread: Vec<usize>,
    drafts: usize,
    /// CMP-17: drafts scheduled to send, not counted in `drafts`.
    scheduled: usize,
    /// By `Category as usize`.
    in_inbox: [bool; 5],
}

/// Where a view's count sits in `NavCounts::unread`.
fn slot(nav: Nav) -> usize {
    const CATEGORY: usize = Folder::ALL.len() + 3;
    const LABEL: usize = CATEGORY + Category::ALL.len();
    match nav {
        Nav::Folder(folder) => folder as usize,
        Nav::Starred => Folder::ALL.len(),
        Nav::Snoozed => Folder::ALL.len() + 1,
        Nav::Scheduled => Folder::ALL.len() + 2,
        Nav::Category(category) => CATEGORY + category as usize,
        Nav::Label(ix) => LABEL + ix,
    }
}

impl NavCounts {
    /// The cached threads' counts.
    pub fn local(threads: &[Thread], labels: &[Label]) -> Self {
        let ids: HashMap<&str, usize> = labels
            .iter()
            .enumerate()
            .map(|(ix, l)| (l.id.as_str(), ix))
            .collect();
        let mut this = Self {
            unread: vec![0; slot(Nav::Label(labels.len()))],
            ..Self::default()
        };
        for t in threads {
            if t.folder == Folder::Drafts {
                if t.later.send_at.is_some() {
                    this.scheduled += 1;
                } else {
                    this.drafts += 1;
                }
            }
            if t.folder == Folder::Inbox {
                for c in Category::ALL {
                    if in_category(t, c) {
                        this.in_inbox[c as usize] = true;
                        if t.unread {
                            this.unread[slot(Nav::Category(c))] += 1;
                        }
                    }
                }
            }
            if !t.unread {
                continue;
            }
            for folder in Folder::ALL {
                // The Inbox counts Primary, as its category above.
                if folder != Folder::Inbox && nav_holds(Nav::Folder(folder), t, labels) {
                    this.unread[slot(Nav::Folder(folder))] += 1;
                }
            }
            if nav_holds(Nav::Starred, t, labels) {
                this.unread[slot(Nav::Starred)] += 1;
            }
            if nav_holds(Nav::Snoozed, t, labels) {
                this.unread[slot(Nav::Snoozed)] += 1;
            }
            if !t.is_hidden() {
                for id in &t.labels {
                    if let Some(&ix) = ids.get(id.as_str()) {
                        this.unread[slot(Nav::Label(ix))] += 1;
                    }
                }
            }
        }
        this.unread[slot(Nav::Folder(Folder::Inbox))] =
            this.unread[slot(Nav::Category(Category::Primary))];
        this
    }

    /// Unread threads in a view.
    pub fn unread(&self, nav: Nav) -> usize {
        self.unread.get(slot(nav)).copied().unwrap_or(0)
    }

    /// The number next to a nav item: total drafts, unread elsewhere,
    /// nothing for folders where an unread count means little.
    pub fn shown(&self, nav: Nav) -> usize {
        match nav {
            Nav::Folder(Folder::Drafts) => self.drafts,
            Nav::Scheduled => self.scheduled,
            Nav::Folder(Folder::Sent | Folder::Archive | Folder::Trash)
            | Nav::Starred
            | Nav::Snoozed => 0,
            _ => self.unread(nav),
        }
    }
}

/// A sidebar count: "1,234", capped at "9,999+" (ME-028).
pub fn count_text(n: usize) -> String {
    const CAP: usize = 9_999;
    if n > CAP {
        format!("{}+", thousands(CAP))
    } else {
        thousands(n)
    }
}

/// ME-033: the go-to shortcut a nav item teaches in its tooltip.
fn nav_action(nav: Nav) -> Option<Box<dyn Action>> {
    Some(match nav {
        Nav::Folder(Folder::Inbox) => Box::new(GoToInbox),
        Nav::Folder(Folder::Sent) => Box::new(GoToSent),
        Nav::Folder(Folder::Drafts) => Box::new(GoToDrafts),
        Nav::Folder(Folder::Archive) => Box::new(GoToArchive),
        Nav::Folder(Folder::Spam) => Box::new(GoToSpam),
        Nav::Folder(Folder::Trash) => Box::new(GoToTrash),
        Nav::Starred => Box::new(GoToStarred),
        Nav::Snoozed => Box::new(GoToSnoozed),
        Nav::Scheduled => Box::new(GoToScheduled),
        Nav::Category(Category::Social) => Box::new(GoToSocial),
        Nav::Category(Category::Promotions) => Box::new(GoToPromotions),
        Nav::Category(Category::Updates) => Box::new(GoToUpdates),
        Nav::Category(Category::Forums) => Box::new(GoToForums),
        Nav::Category(Category::Primary) | Nav::Label(_) => return None,
    })
}

/// MB-010: "Mark All as Read" from a folder's or label's context menu.
#[derive(Clone, PartialEq, gpui::Action)]
#[action(namespace = sidebar, no_json)]
pub struct MarkAllRead(pub Nav);

/// A label the sidebar lists (FOL-9, FOL-10).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LabelRow {
    /// Index into the labels.
    pub ix: usize,
    pub depth: usize,
    pub name: String,
    pub children: usize,
    pub folded: bool,
    /// Index into the labels.
    pub parent: Option<usize>,
    /// Its place in the whole tree, which orders it among its siblings.
    pub pos: usize,
}

/// The labels the sidebar lists, as a tree, and apart those it leaves out
/// for their visibility (flat, under their full names). A label shows
/// when it's set to, or while unread for "show if unread", or while open;
/// children show under a parent that shows and isn't folded.
pub fn label_rows(
    labels: &[Label],
    order: &[String],
    folded: &HashSet<String>,
    unread: impl Fn(usize) -> usize,
    open: Option<usize>,
) -> (Vec<LabelRow>, Vec<LabelRow>) {
    #[derive(Clone, Copy, PartialEq)]
    enum Out {
        Shown,
        Folded,
        Hidden,
    }
    let mut state: HashMap<usize, Out> = HashMap::new();
    let (mut rows, mut hidden) = (Vec::new(), Vec::new());
    for (pos, node) in label_tree(labels, order).into_iter().enumerate() {
        let label = &labels[node.ix];
        let parent = node.parent.map(|p| state[&p]);
        let visible = open == Some(node.ix)
            || match label.list_visibility {
                ListVisibility::Show => true,
                ListVisibility::ShowIfUnread => unread(node.ix) > 0,
                ListVisibility::Hide => false,
            };
        let out = match parent {
            Some(Out::Folded) => Out::Folded,
            Some(Out::Hidden) => Out::Hidden,
            _ if !visible => Out::Hidden,
            _ => Out::Shown,
        };
        match out {
            Out::Shown => {
                let is_folded = node.children > 0 && folded.contains(&label.id);
                state.insert(node.ix, if is_folded { Out::Folded } else { Out::Shown });
                rows.push(LabelRow {
                    ix: node.ix,
                    depth: node.depth,
                    name: node.name,
                    children: node.children,
                    folded: is_folded,
                    parent: node.parent,
                    pos,
                });
                continue;
            }
            Out::Hidden => hidden.push(LabelRow {
                ix: node.ix,
                depth: 1,
                name: label.name.clone(),
                children: 0,
                folded: false,
                parent: None,
                pos,
            }),
            Out::Folded => {}
        }
        state.insert(node.ix, out);
    }
    (rows, hidden)
}

/// A label being dragged to a new place among its siblings.
#[derive(Clone)]
struct DraggedLabel {
    id: String,
    name: String,
    color: gpui::Hsla,
    parent: Option<usize>,
    pos: usize,
}

/// What follows the pointer while a label is dragged.
struct DragGhost(DraggedLabel);

impl gpui::Render for DragGhost {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = Theme::of(cx);
        div()
            .h(CONTROL_MD)
            .px(SPACE_MD)
            .flex()
            .items_center()
            .gap(SPACE_SM)
            .rounded(RADIUS_MD)
            .border_1()
            .border_color(theme.overlay_border)
            .bg(theme.overlay_bg)
            .shadow_md()
            .text_size(TEXT_BASE)
            .text_color(theme.text)
            .child(div().size(super::DOT).rounded_full().bg(self.0.color))
            .child(self.0.name.clone())
    }
}

const FOLDERS: [(Nav, &str, Icon); 9] = [
    (Nav::Folder(Folder::Inbox), "Inbox", Icon::Inbox),
    (Nav::Starred, "Starred", Icon::Star),
    (Nav::Snoozed, "Snoozed", Icon::AlarmClock),
    (Nav::Folder(Folder::Sent), "Sent", Icon::Send),
    // Only while something is scheduled, like Gmail's.
    (Nav::Scheduled, "Scheduled", Icon::Clock),
    (Nav::Folder(Folder::Drafts), "Drafts", Icon::FilePen),
    (Nav::Folder(Folder::Archive), "Archive", Icon::Archive),
    (Nav::Folder(Folder::Spam), "Spam", Icon::ShieldAlert),
    (Nav::Folder(Folder::Trash), "Trash", Icon::Trash),
];

/// Leading visual of a nav row.
enum Marker {
    Icon(Icon),
    Dot(gpui::Hsla),
}

/// Gmail's unread counts for the Inbox, Spam and labels (FOL-2, LST-6),
/// which hold more than the cached threads. Each keeps the local count
/// from when it was read, so reading or marking unread here moves it at
/// once, until the next sync brings the server's again.
#[derive(Default)]
pub struct ServerCounts {
    /// By Gmail label id: (server unread, local unread then).
    by_label: HashMap<String, (usize, usize)>,
}

impl PigeonApp {
    /// The Gmail label whose count a view shows, if Gmail counts it.
    /// W4b: the Inbox and its categories count with W4a's numbers, under
    /// these keys.
    fn counted_label(&self, nav: Nav) -> Option<&str> {
        match nav {
            Nav::Folder(Folder::Inbox) => Some(category_key(Category::Primary)),
            Nav::Category(c) => Some(category_key(c)),
            Nav::Folder(Folder::Spam) => Some(gmail::SPAM),
            Nav::Label(ix) => self.data.labels.get(ix).map(|l| l.id.as_str()),
            _ => None,
        }
    }

    pub(crate) fn local_unread(&self, nav: Nav) -> usize {
        self.data
            .threads
            .iter()
            .filter(|t| t.unread && self.nav_contains(nav, t))
            .count()
    }

    /// Navs Gmail counts: the Inbox, Spam, the categories and the labels.
    fn counted_navs(&self) -> impl Iterator<Item = Nav> + use<> {
        [Nav::Folder(Folder::Inbox), Nav::Folder(Folder::Spam)]
            .into_iter()
            .chain(CATEGORIES.iter().map(|&(c, _)| Nav::Category(c)))
            .chain((0..self.data.labels.len()).map(Nav::Label))
    }

    /// Take Gmail's counts from the store, as of the threads loaded now.
    pub fn refresh_server_counts(&mut self, cx: &gpui::App) {
        let mut counts = self.backend.read(cx).label_counts();
        let categories = *self.backend.read(cx).category_counts();
        for c in Category::ALL {
            counts.insert(
                category_key(c).to_string(),
                (categories.unread(c).into(), 0),
            );
        }
        let local = NavCounts::local(&self.data.threads, &self.data.labels);
        let mut by_label = HashMap::new();
        for nav in self.counted_navs() {
            if let Some(label) = self.counted_label(nav)
                && let Some(&(unread, _)) = counts.get(label)
            {
                by_label.insert(label.to_string(), (unread as usize, local.unread(nav)));
            }
        }
        self.server_counts = ServerCounts { by_label };
    }

    /// `local` unread in a view, moved to Gmail's count where it has one.
    fn with_server_count(&self, nav: Nav, local: usize) -> usize {
        match self
            .counted_label(nav)
            .and_then(|l| self.server_counts.by_label.get(l))
        {
            Some(&(server, then)) => (server + local).saturating_sub(then),
            None => local,
        }
    }

    /// Every view's count, in one pass over the threads (MC-037): Gmail's
    /// count where it has one, moved by what changed here since, else the
    /// cached threads'. Built once per frame by `render`.
    pub fn nav_counts(&self) -> NavCounts {
        let mut counts = NavCounts::local(&self.data.threads, &self.data.labels);
        for nav in self.counted_navs() {
            let local = counts.unread(nav);
            counts.unread[slot(nav)] = self.with_server_count(nav, local);
        }
        counts
    }

    /// Whether the Labels section is folded away (MB-074).
    pub(crate) fn labels_collapsed(&self) -> bool {
        self.labels_ui.folded.contains(LABELS_SECTION)
    }

    fn toggle_labels_section(&mut self, cx: &mut Context<Self>) {
        if !self.labels_ui.folded.remove(LABELS_SECTION) {
            self.labels_ui.folded.insert(LABELS_SECTION.to_string());
        }
        // Saved with the folded labels (MC-048).
        cx.notify();
    }

    /// MB-010: mark every cached thread in a view as read. Gmail's count
    /// can hold more than the cache; those stay unread until the backend
    /// can mark a whole label.
    pub(crate) fn mark_all_read(
        &mut self,
        action: &MarkAllRead,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let targets: Vec<usize> = self
            .data
            .threads
            .iter()
            .enumerate()
            .filter(|(_, t)| t.unread && self.nav_contains(action.0, t))
            .map(|(ix, _)| ix)
            .collect();
        if !targets.is_empty() {
            self.set_unread(targets, false, cx);
        }
    }

    /// MB-010: a folder's context menu.
    fn show_folder_menu(
        &mut self,
        nav: Nav,
        position: gpui::Point<gpui::Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // Unavailable items are left out, so a view with nothing unread
        // has no menu.
        if self.local_unread(nav) == 0 {
            return;
        }
        NativeMenu::new()
            .menu("Mark All as Read", Box::new(MarkAllRead(nav)))
            .show(position, window, cx);
    }

    pub(crate) fn render_sidebar(
        &self,
        width: gpui::Pixels,
        counts: &NavCounts,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let theme = Theme::of(cx).clone();
        let account = Contact {
            name: self.data.account.name.clone(),
            email: self.data.account.email.clone(),
        };

        let categories =
            shown_categories(counts, self.backend.read(cx).category_counts(), self.nav);
        let mut folders = Vec::new();
        for &(nav, name, glyph) in &FOLDERS {
            if nav == Nav::Scheduled && counts.shown(nav) == 0 && self.nav != nav {
                continue;
            }
            folders.push(
                self.nav_item(nav, name.into(), Marker::Icon(glyph), 0, counts, &theme, cx)
                    .into_any_element(),
            );
            if nav == Nav::Folder(Folder::Inbox) {
                for &category in &categories {
                    let glyph = CATEGORIES
                        .iter()
                        .find(|(c, _)| *c == category)
                        .map_or(Icon::Inbox, |(_, g)| *g);
                    folders.push(
                        self.nav_item(
                            Nav::Category(category),
                            category.name().into(),
                            Marker::Icon(glyph),
                            1,
                            counts,
                            &theme,
                            cx,
                        )
                        .into_any_element(),
                    );
                }
            }
        }
        let collapsed = self.labels_collapsed();
        let (rows, hidden) = self.sidebar_labels_with(counts);
        let show_hidden = self.labels_ui.show_hidden && !hidden.is_empty();
        let mut labels: Vec<AnyElement> = rows
            .into_iter()
            .map(|row| self.label_item(row, true, counts, &theme, cx))
            .collect();
        if !hidden.is_empty() {
            let count = super::status::plural(hidden.len(), "label", "labels");
            let text = if show_hidden {
                format!("Hide {count}")
            } else {
                format!("{count} hidden")
            };
            labels.push(
                focusable_control(div().id("hidden-labels"), &theme)
                    .role(gpui::Role::Button)
                    .aria_label(text.clone())
                    .aria_expanded(show_hidden)
                    .h(CONTROL_MD)
                    .flex_none()
                    .px(SPACE_MD)
                    .flex()
                    .items_center()
                    .gap(SPACE_SM)
                    .rounded(RADIUS_MD)
                    .text_size(TEXT_SM)
                    .text_color(theme.text_faint)
                    .hover(|s| s.bg(theme.hover_bg))
                    .child(motion::turn(
                        "chevron",
                        icon(Icon::ChevronDown, ICON_MD, theme.text_faint),
                        show_hidden,
                        2.,
                    ))
                    .child(text)
                    .tooltip(|window, cx| {
                        tooltip("Show or leave out hidden labels")
                            .action(&ToggleHiddenLabels, None)
                            .build(window, cx)
                    })
                    .on_click(|_, window, cx| {
                        window.dispatch_action(Box::new(ToggleHiddenLabels), cx)
                    })
                    .into_any_element(),
            );
            if show_hidden {
                labels.extend(
                    hidden
                        .into_iter()
                        .map(|row| self.label_item(row, false, counts, &theme, cx)),
                );
            }
        }
        let account_label = format!("{}, {}", account.name, account.email);
        let demo = self.backend.read(cx).source() == Source::Demo;

        div()
            .relative()
            .w(width)
            .h_full()
            .flex_none()
            .flex()
            .flex_col()
            .bg(theme.sidebar_bg)
            .border_r_1()
            .border_color(theme.border)
            .child(
                window_drag_area(div().id("sidebar-header"))
                    .h(super::HEADER_MIN)
                    .min_h(HEADER_HEIGHT)
                    .flex_none()
                    .flex()
                    .items_center()
                    // MA-062: at the leading edge, where the list header
                    // shows it while the sidebar is hidden.
                    .justify_start()
                    .px(SPACE_MD)
                    .child(icon_button(
                        "toggle-sidebar",
                        Icon::PanelLeft,
                        "Hide sidebar",
                        Box::new(ToggleSidebar),
                        &theme,
                    )),
            )
            .child(
                // The account: who this mail is for. Opens Settings ›
                // Accounts (ME-029); the address is in its tooltip.
                focusable_control(div().id("account"), &theme)
                    .role(gpui::Role::Button)
                    .aria_label(account_label)
                    .aria_description("Opens Accounts settings")
                    .mx(SPACE_MD)
                    .mb(SPACE_SM)
                    .h(CONTROL_MD)
                    .px(SPACE_MD)
                    .flex()
                    .items_center()
                    .gap(SPACE_SM)
                    .rounded(RADIUS_MD)
                    .hover(|s| s.bg(theme.hover_bg))
                    .active(|s| s.bg(theme.selected_bg))
                    .on_click(|_, window, cx| {
                        window.dispatch_action(Box::new(OpenSettingsSection(Section::Accounts)), cx)
                    })
                    .child(account_avatar(
                        self.backend.read(cx).profile_photo(),
                        account.initials(),
                        theme.avatar_color(&account.email),
                        AVATAR_XS,
                        &theme,
                    ))
                    .child(
                        div()
                            .min_w_0()
                            .truncate()
                            .text_size(TEXT_BASE)
                            .font_weight(FontWeight::MEDIUM)
                            .child(account.name.clone()),
                    )
                    .tooltip({
                        let email = account.email.clone();
                        move |window, cx| tooltip(email.clone()).build(window, cx)
                    }),
            )
            .child(
                div()
                    .mx(SPACE_MD)
                    .mb(SPACE_SM)
                    .flex()
                    .flex_col()
                    .gap(SPACE_SM)
                    .child(if demo {
                        demo_row(&theme)
                    } else {
                        super::status::status_line(
                            self.backend.read(cx).status(),
                            super::status::now_ms(),
                            &theme,
                        )
                    })
                    .children(super::status::notifications_card(
                        self.backend.read(cx),
                        &theme,
                        cx,
                    )),
            )
            .child(
                div()
                    .id("sidebar-nav")
                    .role(gpui::Role::Navigation)
                    .aria_label("Mailboxes")
                    .flex_1()
                    .min_h_0()
                    .px(SPACE_MD)
                    .pb(SPACE_MD)
                    .flex()
                    .flex_col()
                    .gap(SPACE_2XS)
                    .overflow_y_scrollbar()
                    .children(folders)
                    .child(labels_header(collapsed, &theme, cx))
                    .children(labels),
            )
    }

    /// The labels the sidebar lists, and those under "N hidden". None
    /// while the Labels section is folded, so keyboard label commands
    /// skip it too.
    pub(crate) fn sidebar_labels(&self) -> (Vec<LabelRow>, Vec<LabelRow>) {
        self.sidebar_labels_with(&self.nav_counts())
    }

    fn sidebar_labels_with(&self, counts: &NavCounts) -> (Vec<LabelRow>, Vec<LabelRow>) {
        if self.labels_collapsed() {
            return Default::default();
        }
        let open = match self.nav {
            Nav::Label(ix) => Some(ix).filter(|&ix| {
                self.labels_ui.hidden_while_open.as_deref()
                    != self.data.labels.get(ix).map(|l| l.id.as_str())
            }),
            _ => None,
        };
        label_rows(
            &self.data.labels,
            &self.labels_ui.order,
            &self.labels_ui.folded,
            |ix| counts.unread(Nav::Label(ix)),
            open,
        )
    }

    /// W4b: a label row, nested and foldable, with its context menu.
    /// `movable` rows drag onto their siblings to rearrange them.
    fn label_item(
        &self,
        row: LabelRow,
        movable: bool,
        counts: &NavCounts,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let label = &self.data.labels[row.ix];
        let ix = row.ix;
        let dragged = DraggedLabel {
            id: label.id.clone(),
            name: label.name.clone(),
            color: label.color,
            parent: row.parent,
            pos: row.pos,
        };
        let (target_id, parent, pos) = (label.id.clone(), row.parent, row.pos);
        let target = target_id.clone();
        let accent = theme.accent;
        let context = self.labels_ui.context_label.as_deref() == Some(label.id.as_str());
        let ring = theme.context_ring;
        let item = self.nav_item(
            Nav::Label(ix),
            row.name.clone().into(),
            Marker::Dot(label.color),
            row.depth,
            counts,
            theme,
            cx,
        );
        item.when(row.children > 0, |this| {
            // MB-032: the triangle sits in the indent gutter before the
            // dot, a gutter wide and a row tall.
            let name = row.name.clone();
            let tip = format!(
                "{} sublabels of {name}",
                if row.folded { "Show" } else { "Hide" }
            );
            let faint = theme.text_faint;
            this.aria_expanded(!row.folded).child(
                div()
                    .id(("fold-label", ix))
                    .role(gpui::Role::DisclosureTriangle)
                    .aria_label(tip.clone())
                    .aria_expanded(!row.folded)
                    .absolute()
                    .top_0()
                    .bottom_0()
                    .left(SPACE_LG * row.depth as f32)
                    .w(SPACE_LG)
                    .flex()
                    .items_center()
                    .justify_center()
                    // MD-021: one glyph, turned.
                    .child(motion::turn(
                        "chevron",
                        icon(Icon::ChevronRight, ICON_SM, faint),
                        !row.folded,
                        1.,
                    ))
                    .tooltip(move |window, cx| tooltip(tip.clone()).build(window, cx))
                    // Not a drag of the row, nor a click on it.
                    .on_mouse_down(gpui::MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .on_click(cx.listener(move |this, _, _, cx| {
                        cx.stop_propagation();
                        this.toggle_label_folded(ix, cx);
                    })),
            )
        })
        // The label its context menu is about (MB-017).
        .when(context, |this| {
            this.child(
                div()
                    .absolute()
                    .inset_0()
                    .rounded(RADIUS_MD)
                    .border_1()
                    .border_color(ring),
            )
        })
        .when(movable, |this| {
            this.on_drag(dragged, |dragged, _, _, cx| {
                cx.new(|_| DragGhost(dragged.clone()))
            })
            .drag_over::<DraggedLabel>(move |style, dragged, _, _| {
                if dragged.parent != parent || dragged.id == target_id {
                    return style;
                }
                // Dropped on a sibling it takes that place: below it when
                // it comes from above, else above.
                if dragged.pos < pos {
                    style.border_b_2().border_color(accent)
                } else {
                    style.border_t_2().border_color(accent)
                }
            })
            .on_drop(cx.listener(move |this, dragged: &DraggedLabel, _, cx| {
                this.drop_label(&dragged.id, &target, cx)
            }))
        })
        .on_any_mouse_down(
            cx.listener(move |this, event: &gpui::MouseDownEvent, window, cx| {
                if is_secondary(event) {
                    this.show_label_menu(ix, event.position, window, cx)
                }
            }),
        )
        .into_any_element()
    }

    #[allow(clippy::too_many_arguments)]
    fn nav_item(
        &self,
        nav: Nav,
        name: gpui::SharedString,
        marker: Marker,
        depth: usize,
        counts: &NavCounts,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> gpui::Stateful<gpui::Div> {
        let active = self.nav == nav;
        let count = counts.shown(nav);
        let id = match nav {
            Nav::Folder(folder) => ("nav-folder", folder as usize),
            Nav::Starred => ("nav-starred", 0),
            Nav::Snoozed => ("nav-snoozed", 0),
            Nav::Scheduled => ("nav-scheduled", 0),
            Nav::Label(ix) => ("nav-label", ix),
            Nav::Category(c) => ("nav-category", c as usize),
        };
        // MA-048: the name carries the count it shows.
        let label = match (count, nav) {
            (0, _) => name.to_string(),
            (n, Nav::Folder(Folder::Drafts) | Nav::Scheduled) => {
                format!("{name}, {}", thousands(n))
            }
            (n, _) => format!("{name}, {} unread", thousands(n)),
        };
        let tip = name.clone();

        focusable_control(div().id(id), theme)
            .role(gpui::Role::Link)
            .aria_label(label)
            .aria_selected(active)
            .relative()
            .h(ROW_SM)
            .flex_none()
            .pl(SPACE_MD + SPACE_LG * depth as f32)
            .pr(SPACE_MD)
            .flex()
            .items_center()
            .gap(SPACE_SM)
            .rounded(RADIUS_MD)
            .text_size(TEXT_BASE)
            .map(|this| {
                if active {
                    this.bg(theme.nav_active_bg)
                        .text_color(theme.text)
                        .font_weight(FontWeight::MEDIUM)
                } else {
                    this.text_color(theme.text_muted)
                        .hover(|s| s.bg(theme.hover_bg))
                }
            })
            .child(match marker {
                Marker::Icon(glyph) => icon(
                    glyph,
                    ICON_LG,
                    if active {
                        theme.icon_active
                    } else {
                        theme.icon
                    },
                )
                .into_any_element(),
                Marker::Dot(color) => super::dot_marker(color).into_any_element(),
            })
            .child(div().flex_1().truncate().child(name))
            .when_some(nav_action(nav), |this, action| {
                this.tooltip(move |window, cx| {
                    tooltip(tip.clone())
                        .action(action.as_ref(), None)
                        .build(window, cx)
                })
            })
            // MA-087: conversations dropped here move, star, label or
            // recategorize; rows where that changes nothing don't light up.
            .drag_over::<DraggedThreads>({
                let app = cx.entity();
                let bg = theme.selected_bg;
                move |style, dragged, _, cx| {
                    if app.read(cx).accepts_drop(nav, dragged) {
                        style.bg(bg)
                    } else {
                        style
                    }
                }
            })
            .on_drop(
                cx.listener(move |this, dragged: &DraggedThreads, window, cx| {
                    this.drop_threads(nav, dragged, window, cx)
                }),
            )
            .when(!matches!(nav, Nav::Label(_)), |this| {
                this.on_any_mouse_down(cx.listener(
                    move |this, event: &gpui::MouseDownEvent, window, cx| {
                        if is_secondary(event) {
                            this.show_folder_menu(nav, event.position, window, cx)
                        }
                    },
                ))
            })
            .when(count > 0, |this| {
                // Unread counts (and Drafts' total); the Inbox's stands out.
                let inbox = nav == Nav::Folder(Folder::Inbox);
                this.child(
                    div()
                        .text_size(TEXT_SM)
                        .font_features(tabular_figures())
                        .font_weight(if inbox {
                            FontWeight::SEMIBOLD
                        } else {
                            FontWeight::NORMAL
                        })
                        .text_color(if inbox { theme.text } else { theme.text_muted })
                        .child(count_text(count)),
                )
            })
            .on_click(
                cx.listener(move |this, event: &gpui::ClickEvent, window, cx| {
                    // Control-click opens a label's menu instead.
                    if event.modifiers().control {
                        return;
                    }
                    this.set_nav(nav, cx);
                    // A click hands the keyboard back to the list; Enter on a
                    // focused row keeps focus on the row.
                    if !event.is_keyboard() {
                        window.focus(&this.focus_handle, cx);
                    }
                }),
            )
    }
}

/// MB-074: "Labels" folds the section away; the chevron shows on hover
/// and while folded.
fn labels_header(collapsed: bool, theme: &Theme, cx: &mut Context<PigeonApp>) -> impl IntoElement {
    const GROUP: &str = "labels-section";
    let tip = if collapsed {
        "Show labels"
    } else {
        "Hide labels"
    };
    div()
        .pt(SPACE_LG)
        .pb(SPACE_XS)
        .flex_none()
        .flex()
        .items_center()
        .gap(SPACE_XS)
        .child(
            focusable_control(div().id(GROUP), theme)
                .group(GROUP)
                .role(gpui::Role::Button)
                .aria_label("Labels")
                .aria_expanded(!collapsed)
                .flex_1()
                .min_w_0()
                .h(CONTROL_MD)
                .pl(SPACE_MD)
                .flex()
                .items_center()
                .gap(SPACE_XS)
                .rounded(RADIUS_MD)
                .text_size(TEXT_XS)
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(theme.text_faint)
                .child("Labels")
                .child(
                    div()
                        .flex()
                        .when(!collapsed, |this| {
                            this.opacity(0.).group_hover(GROUP, |s| s.opacity(1.))
                        })
                        .child(motion::turn(
                            "labels-chevron",
                            icon(Icon::ChevronRight, ICON_SM, theme.text_faint),
                            !collapsed,
                            1.,
                        )),
                )
                .tooltip(move |window, cx| tooltip(tip).build(window, cx))
                .on_click(cx.listener(|this, _, _, cx| this.toggle_labels_section(cx))),
        )
        .child(
            icon_button(
                "new-label",
                Icon::Plus,
                "New label",
                Box::new(NewLabel),
                theme,
            )
            .my(-SPACE_XS),
        )
}

/// ME-050: in demo data, the way out: "Demo data · Sign in…".
fn demo_row(theme: &Theme) -> AnyElement {
    focusable_control(div().id("demo-sign-in"), theme)
        .role(gpui::Role::Button)
        .aria_label("Demo data. Sign in…")
        .px(SPACE_MD)
        .py(SPACE_XS)
        .flex()
        .items_center()
        .gap(SPACE_XS)
        .rounded(RADIUS_MD)
        .text_size(TEXT_XS)
        .text_color(theme.text_faint)
        .hover(|s| s.bg(theme.hover_bg))
        .active(|s| s.bg(theme.selected_bg))
        .child("Demo data ·")
        .child(div().text_color(theme.link).child("Sign in…"))
        .tooltip(|window, cx| {
            tooltip("Sign in with Google")
                .action(&SignIn, None)
                .build(window, cx)
        })
        .on_click(|_, window, cx| window.dispatch_action(Box::new(SignIn), cx))
        .into_any_element()
}

fn category_key(category: Category) -> &'static str {
    match category {
        Category::Primary => "category:primary",
        Category::Social => "category:social",
        Category::Promotions => "category:promotions",
        Category::Updates => "category:updates",
        Category::Forums => "category:forums",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{app::nav_holds, data::Message};

    fn thread(id: &str, folder: Folder, labels: &[&str], unread: bool) -> Thread {
        let message = Message {
            label_ids: labels.iter().map(|l| l.to_string()).collect(),
            ..Message::new(Contact::default())
        };
        let mut t = Thread::local(id.into(), id.into(), folder, vec![message], None);
        t.unread = unread;
        t
    }

    #[test]
    fn the_inbox_shows_primary_and_the_categories_their_own() {
        let threads = [
            thread("p", Folder::Inbox, &["INBOX"], true),
            thread(
                "promo",
                Folder::Inbox,
                &["INBOX", "CATEGORY_PROMOTIONS"],
                true,
            ),
            thread("upd", Folder::Inbox, &["INBOX", "CATEGORY_UPDATES"], false),
            thread("old", Folder::Archive, &["CATEGORY_SOCIAL"], false),
        ];
        let shown = |nav| -> Vec<&str> {
            threads
                .iter()
                .filter(|t| nav_holds(nav, t, &[]))
                .map(|t| t.id.as_str())
                .collect()
        };
        assert_eq!(shown(Nav::Folder(Folder::Inbox)), ["p"]);
        assert_eq!(shown(Nav::Category(Category::Promotions)), ["promo"]);
        assert_eq!(shown(Nav::Category(Category::Updates)), ["upd"]);
        // Archived Social mail isn't in the Inbox's Social.
        assert!(shown(Nav::Category(Category::Social)).is_empty());
        let counts = crate::backend::categories::local_counts(&threads);
        assert_eq!(counts.unread(Category::Primary), 1);
        assert_eq!(counts.unread(Category::Promotions), 1);
        assert_eq!(counts.text(Category::Updates), None);
        // Updates has a thread though none unread; Social and Forums
        // have neither, unless open.
        let inbox = Nav::Folder(Folder::Inbox);
        let nav = NavCounts::local(&threads, &[]);
        assert_eq!(
            shown_categories(&nav, &counts, inbox),
            [Category::Promotions, Category::Updates]
        );
        assert_eq!(
            shown_categories(&nav, &counts, Nav::Category(Category::Forums)),
            [Category::Promotions, Category::Updates, Category::Forums]
        );
        // Unread on the server only (older than the cache).
        let promo_only = NavCounts::local(&threads[..1], &[]);
        assert_eq!(
            shown_categories(&promo_only, &counts, inbox),
            [Category::Promotions]
        );
    }

    #[test]
    fn nav_counts_match_nav_holds() {
        let mut starred = thread("s", Folder::Archive, &[], true);
        starred.starred = true;
        let mut labelled = thread("l", Folder::Inbox, &["INBOX"], true);
        labelled.labels = vec!["L0".into()];
        let mut spam = thread("x", Folder::Spam, &[], true);
        spam.labels = vec!["L0".into()];
        let mut snoozed = thread("z", Folder::Archive, &[], true);
        snoozed.later.snoozed_until = Some(1);
        let mut scheduled = thread("d3", Folder::Drafts, &[], false);
        scheduled.later.send_at = Some(1);
        let threads = [
            thread("p", Folder::Inbox, &["INBOX"], false),
            thread(
                "promo",
                Folder::Inbox,
                &["INBOX", "CATEGORY_PROMOTIONS"],
                true,
            ),
            thread("d1", Folder::Drafts, &[], false),
            thread("d2", Folder::Drafts, &[], true),
            starred,
            labelled,
            spam,
            snoozed,
            scheduled,
        ];
        let labels = [Label::new("L0".to_string(), "Work", gpui::black())];
        let counts = NavCounts::local(&threads, &labels);
        let navs = Folder::ALL
            .into_iter()
            .map(Nav::Folder)
            .chain([Nav::Starred, Nav::Snoozed, Nav::Label(0)])
            .chain(Category::ALL[1..].iter().map(|&c| Nav::Category(c)));
        for nav in navs {
            let expected = threads
                .iter()
                .filter(|t| t.unread && nav_holds(nav, t, &labels))
                .count();
            assert_eq!(counts.unread(nav), expected, "{nav:?}");
        }
        assert_eq!(counts.shown(Nav::Folder(Folder::Drafts)), 2);
        assert_eq!(counts.shown(Nav::Scheduled), 1);
        assert_eq!(counts.unread(Nav::Snoozed), 1);
        // The starred one only: snoozed conversations aren't in Archive.
        assert_eq!(counts.unread(Nav::Folder(Folder::Archive)), 1);
        assert_eq!(counts.shown(Nav::Starred), 0);
        assert_eq!(counts.unread(Nav::Label(0)), 1);
        assert_eq!(counts.unread(Nav::Label(7)), 0);
    }

    #[test]
    fn counts_have_separators_and_a_cap() {
        assert_eq!(count_text(12), "12");
        assert_eq!(count_text(1_234), "1,234");
        assert_eq!(count_text(9_999), "9,999");
        assert_eq!(count_text(12_345), "9,999+");
    }

    fn labels(spec: &[(&str, ListVisibility)]) -> Vec<Label> {
        spec.iter()
            .enumerate()
            .map(|(ix, (name, v))| Label {
                list_visibility: *v,
                ..Label::new(format!("L{ix}"), *name, gpui::black())
            })
            .collect()
    }

    #[test]
    fn label_rows_fold_and_follow_visibility() {
        use ListVisibility::*;
        let l = labels(&[
            ("Work", Show),
            ("Work/Clients", Show),
            ("Work/Clients/Acme", Show),
            ("Quiet", ShowIfUnread),
            ("Secret", Hide),
            ("Secret/Inner", Show),
        ]);
        let names = |rows: &[LabelRow]| -> Vec<(usize, String)> {
            rows.iter().map(|r| (r.depth, r.name.clone())).collect()
        };
        let none = HashSet::new();
        let (rows, hidden) = label_rows(&l, &[], &none, |_| 0, None);
        assert_eq!(
            names(&rows),
            [
                (0, "Work".into()),
                (1, "Clients".into()),
                (2, "Acme".into())
            ]
        );
        // Quiet (read), Secret and its child: apart, by full name.
        assert_eq!(
            names(&hidden),
            [
                (1, "Quiet".into()),
                (1, "Secret".into()),
                (1, "Secret/Inner".into())
            ]
        );
        // Unread shows "show if unread"; open shows a hidden one.
        let (rows, hidden) = label_rows(&l, &[], &none, |ix| (ix == 3) as usize, Some(4));
        let shown: Vec<usize> = rows.iter().map(|r| r.ix).collect();
        assert_eq!(shown, [3, 4, 5, 0, 1, 2]);
        assert!(hidden.is_empty());
        // Folding Work keeps it, without its children; not counted hidden.
        let folded: HashSet<String> = ["L0".to_string()].into();
        let (rows, hidden) = label_rows(&l, &[], &folded, |_| 0, None);
        assert_eq!(names(&rows), [(0, "Work".into())]);
        assert!(rows[0].folded);
        assert_eq!(rows[0].children, 1);
        assert_eq!(hidden.len(), 3);
        // The user's order: Secret before Work.
        let order = ["L4".to_string(), "L0".to_string()];
        let (rows, _) = label_rows(&l, &order, &none, |_| 0, Some(4));
        let shown: Vec<usize> = rows.iter().map(|r| r.ix).collect();
        assert_eq!(shown, [4, 5, 0, 1, 2]);
    }
}
