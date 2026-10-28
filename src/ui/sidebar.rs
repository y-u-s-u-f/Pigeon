//! Account card, system folders and labels.

use std::collections::{HashMap, HashSet};

use gpui::{
    AnyElement, AppContext as _, Context, FontWeight, InteractiveElement, IntoElement,
    ParentElement, StatefulInteractiveElement, Styled, Window, div, prelude::FluentBuilder as _,
    px,
};

use super::{
    HEADER_HEIGHT, account_avatar, icon, icon_button, icon_button_small, is_secondary, tooltip,
    window_drag_area,
};
use crate::ui::style::*;
use crate::{
    actions::{NewLabel, ToggleHiddenLabels, ToggleSidebar},
    app::{Nav, PigeonApp},
    assets::Icon,
    backend::{
        categories::{Category, CategoryCounts, in_category},
        gmail,
        labels::label_tree,
    },
    data::{Contact, Folder, Label, ListVisibility, Thread},
    theme::Theme,
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
pub fn shown_categories(threads: &[Thread], counts: &CategoryCounts, open: Nav) -> Vec<Category> {
    CATEGORIES
        .iter()
        .map(|&(c, _)| c)
        .filter(|&c| {
            open == Nav::Category(c)
                || counts.unread(c) > 0
                || threads
                    .iter()
                    .any(|t| t.folder == Folder::Inbox && in_category(t, c))
        })
        .collect()
}

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
            .child(div().size(px(8.)).rounded_full().bg(self.0.color))
            .child(self.0.name.clone())
    }
}

const FOLDERS: [(Nav, &str, Icon); 7] = [
    (Nav::Folder(Folder::Inbox), "Inbox", Icon::Inbox),
    (Nav::Starred, "Starred", Icon::Star),
    (Nav::Folder(Folder::Sent), "Sent", Icon::Send),
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

    fn local_unread(&self, nav: Nav) -> usize {
        self.data
            .threads
            .iter()
            .filter(|t| t.unread && self.nav_contains(nav, t))
            .count()
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
        let navs = [Nav::Folder(Folder::Inbox), Nav::Folder(Folder::Spam)]
            .into_iter()
            .chain(CATEGORIES.iter().map(|&(c, _)| Nav::Category(c)))
            .chain((0..self.data.labels.len()).map(Nav::Label));
        let mut by_label = HashMap::new();
        for nav in navs {
            if let Some(label) = self.counted_label(nav)
                && let Some(&(unread, _)) = counts.get(label)
            {
                by_label.insert(label.to_string(), (unread as usize, self.local_unread(nav)));
            }
        }
        self.server_counts = ServerCounts { by_label };
    }

    /// Unread threads in a view: Gmail's count where it has one, moved by
    /// what changed here since, else the cached threads'.
    pub fn view_unread(&self, nav: Nav) -> usize {
        let local = self.local_unread(nav);
        match self
            .counted_label(nav)
            .and_then(|l| self.server_counts.by_label.get(l))
        {
            Some(&(server, then)) => (server + local).saturating_sub(then),
            None => local,
        }
    }

    /// The number shown next to a nav item: total drafts, unread elsewhere,
    /// nothing for folders where an unread count means little.
    fn nav_count(&self, nav: Nav) -> usize {
        match nav {
            Nav::Folder(Folder::Drafts) => self
                .data
                .threads
                .iter()
                .filter(|t| self.nav_contains(nav, t))
                .count(),
            Nav::Folder(Folder::Sent | Folder::Archive | Folder::Trash) | Nav::Starred => 0,
            _ => self.view_unread(nav),
        }
    }

    pub(crate) fn render_sidebar(
        &self,
        width: gpui::Pixels,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let theme = Theme::of(cx).clone();
        let account = Contact {
            name: self.data.account.name.clone(),
            email: self.data.account.email.clone(),
        };

        let categories = shown_categories(
            &self.data.threads,
            self.backend.read(cx).category_counts(),
            self.nav,
        );
        let mut folders = Vec::new();
        for &(nav, name, glyph) in &FOLDERS {
            folders.push(
                self.nav_item(nav, name.into(), Marker::Icon(glyph), 0, &theme, cx)
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
                            &theme,
                            cx,
                        )
                        .into_any_element(),
                    );
                }
            }
        }
        let (rows, hidden) = self.sidebar_labels();
        let show_hidden = self.labels_ui.show_hidden && !hidden.is_empty();
        let mut labels: Vec<AnyElement> = rows
            .into_iter()
            .map(|row| self.label_item(row, true, &theme, cx))
            .collect();
        if !hidden.is_empty() {
            let text = if show_hidden {
                "Leave out hidden labels".to_string()
            } else {
                format!("{} hidden", hidden.len())
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
                    .child(icon(
                        if show_hidden {
                            Icon::ChevronUp
                        } else {
                            Icon::ChevronDown
                        },
                        ICON_MD,
                        theme.text_faint,
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
                        .map(|row| self.label_item(row, false, &theme, cx)),
                );
            }
        }

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
                    .h(HEADER_HEIGHT)
                    .flex_none()
                    .flex()
                    .items_center()
                    .justify_end()
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
                // The account: who this mail is for. Display only; the
                // address is in its tooltip.
                div()
                    .id("account")
                    .role(gpui::Role::Group)
                    .aria_label(format!("{}, {}", account.name, account.email))
                    .mx(SPACE_MD)
                    .mb(SPACE_SM)
                    .h(CONTROL_MD)
                    .px(SPACE_MD)
                    .flex()
                    .items_center()
                    .gap(SPACE_SM)
                    .rounded(RADIUS_MD)
                    .hover(|s| s.bg(theme.hover_bg))
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
                    .child(super::status::status_line(
                        self.backend.read(cx).status(),
                        chrono::Utc::now().timestamp_millis(),
                        &theme,
                    )),
            )
            .child(
                div()
                    .id("sidebar-nav")
                    .role(gpui::Role::Navigation)
                    .aria_label("Mailboxes")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .px(SPACE_MD)
                    .pb(SPACE_MD)
                    .flex()
                    .flex_col()
                    .gap(SPACE_2XS)
                    .children(folders)
                    .child(
                        div()
                            .pt(SPACE_LG)
                            .pb(SPACE_XS)
                            .pl(SPACE_MD)
                            .flex_none()
                            .flex()
                            .items_center()
                            .justify_between()
                            .text_size(TEXT_XS)
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(theme.text_faint)
                            .child("Labels")
                            .child(
                                icon_button(
                                    "new-label",
                                    Icon::Plus,
                                    "New label",
                                    Box::new(NewLabel),
                                    &theme,
                                )
                                .my(-SPACE_XS),
                            ),
                    )
                    .children(labels),
            )
    }

    /// The labels the sidebar lists, and those under "N hidden".
    pub(crate) fn sidebar_labels(&self) -> (Vec<LabelRow>, Vec<LabelRow>) {
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
            |ix| self.view_unread(Nav::Label(ix)),
            open,
        )
    }

    /// W4b: a label row, nested and foldable, with its context menu.
    /// `movable` rows drag onto their siblings to rearrange them.
    fn label_item(
        &self,
        row: LabelRow,
        movable: bool,
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
        let item = self.nav_item(
            Nav::Label(ix),
            row.name.clone().into(),
            Marker::Dot(label.color),
            row.depth,
            theme,
            cx,
        );
        item.when(row.children > 0, |this| {
            let name = row.name.clone();
            this.aria_expanded(!row.folded).child(
                icon_button_small(
                    ("fold-label", ix),
                    if row.folded {
                        Icon::ChevronRight
                    } else {
                        Icon::ChevronDown
                    },
                    format!(
                        "{} sublabels of {name}",
                        if row.folded { "Show" } else { "Hide" }
                    ),
                    theme,
                )
                .mr(-SPACE_SM)
                .on_click(cx.listener(move |this, _, _, cx| {
                    cx.stop_propagation();
                    this.toggle_label_folded(ix, cx);
                })),
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

    fn nav_item(
        &self,
        nav: Nav,
        name: gpui::SharedString,
        marker: Marker,
        depth: usize,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> gpui::Stateful<gpui::Div> {
        let active = self.nav == nav;
        let count = self.nav_count(nav);
        let id = match nav {
            Nav::Folder(folder) => ("nav-folder", folder as usize),
            Nav::Starred => ("nav-starred", 0),
            Nav::Label(ix) => ("nav-label", ix),
            Nav::Category(c) => ("nav-category", c as usize),
        };

        focusable_control(div().id(id), theme)
            .role(gpui::Role::Link)
            .aria_label(name.clone())
            .aria_selected(active)
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
                Marker::Dot(color) => div()
                    .size(ICON_LG)
                    .flex_none()
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(div().size(px(8.)).rounded_full().bg(color))
                    .into_any_element(),
            })
            .child(div().flex_1().truncate().child(name))
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
                        .child(count.to_string()),
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
        assert_eq!(
            shown_categories(&threads, &counts, inbox),
            [Category::Promotions, Category::Updates]
        );
        assert_eq!(
            shown_categories(&threads, &counts, Nav::Category(Category::Forums)),
            [Category::Promotions, Category::Updates, Category::Forums]
        );
        // Unread on the server only (older than the cache).
        let promo_only = &threads[..1];
        assert_eq!(
            shown_categories(promo_only, &counts, inbox),
            [Category::Promotions]
        );
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
