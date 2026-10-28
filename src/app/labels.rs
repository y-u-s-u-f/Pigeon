//! W4b: the Inbox's category views (FOL-11), Important (FOL-12), and
//! managing labels from ⌘K, the sidebar and the keyboard: create, rename,
//! recolor, delete (FOL-6), fold (FOL-9) and visibility (FOL-10).

use std::collections::HashSet;

use gpui::{
    AppContext as _, Context, Div, Focusable as _, Pixels, Point, SharedString, Stateful, Window,
    component::native_menu::NativeMenu, prelude::*,
};

use super::{Nav, PigeonApp, ToastKind};
use crate::{
    actions::*,
    assets::Icon,
    backend::{
        categories::Category,
        gmail,
        labels::{self, COLORS, LabelEdit},
    },
    data::{Folder, Label, ListVisibility},
    ui::command_palette::{CommandPalette, Leading, PaletteCommand, PaletteItem},
};

#[derive(Default)]
pub struct LabelsUi {
    /// Labels whose children are folded away, by id; saved.
    pub folded: HashSet<String>,
    /// The sidebar lists hidden labels too.
    pub show_hidden: bool,
    /// The open label, by id, once hidden from the sidebar while open: it
    /// goes at once instead of staying for being open (`label_rows`).
    pub hidden_while_open: Option<String>,
    /// The user's arrangement of the labels, by id (`labels::label_tree`);
    /// kept in memory.
    pub order: Vec<String>,
    /// What the name being typed is for.
    prompt: Option<Prompt>,
}

enum Prompt {
    New,
    /// By label id.
    Rename(String),
}

/// What to do with a label picked from a list.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LabelTask {
    Rename,
    Color,
    Delete,
    Visibility,
    Fold,
}

/// The palette's label commands; labels by index into `MailData::labels`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum LabelCommand {
    Pick(usize, LabelTask),
    /// Index into `labels::COLORS`.
    Color(usize, usize),
    ListVisibility(usize, ListVisibility),
    MessageVisibility(usize, bool),
    Delete {
        label: usize,
        children: bool,
    },
    Cancel,
}

impl PigeonApp {
    pub(super) fn on_label_actions(
        &self,
        root: Stateful<Div>,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        let go = |category| {
            move |this: &mut Self, window: &mut Window, cx: &mut Context<Self>| {
                this.go_to(Nav::Category(category), window, cx)
            }
        };
        let (social, promotions, updates, forums) = (
            go(Category::Social),
            go(Category::Promotions),
            go(Category::Updates),
            go(Category::Forums),
        );
        root.on_action(cx.listener(move |this, _: &GoToSocial, w, cx| social(this, w, cx)))
            .on_action(cx.listener(move |this, _: &GoToPromotions, w, cx| promotions(this, w, cx)))
            .on_action(cx.listener(move |this, _: &GoToUpdates, w, cx| updates(this, w, cx)))
            .on_action(cx.listener(move |this, _: &GoToForums, w, cx| forums(this, w, cx)))
            .on_action(cx.listener(|this, _: &ToggleImportant, _, cx| {
                this.toggle_important(this.targets(), cx)
            }))
            .on_action(cx.listener(|this, _: &NewLabel, window, cx| {
                this.prompt_label_name(Prompt::New, "", window, cx)
            }))
            .on_action(cx.listener(|this, _: &RenameLabel, window, cx| {
                this.label_task(LabelTask::Rename, window, cx)
            }))
            .on_action(cx.listener(|this, _: &ChangeLabelColor, window, cx| {
                this.label_task(LabelTask::Color, window, cx)
            }))
            .on_action(cx.listener(|this, _: &DeleteLabel, window, cx| {
                this.label_task(LabelTask::Delete, window, cx)
            }))
            .on_action(cx.listener(|this, _: &ChangeLabelVisibility, window, cx| {
                this.label_task(LabelTask::Visibility, window, cx)
            }))
            .on_action(cx.listener(|this, _: &ToggleLabelFolded, window, cx| {
                this.label_task(LabelTask::Fold, window, cx)
            }))
            .on_action(cx.listener(|this, _: &ToggleHiddenLabels, _, cx| {
                this.labels_ui.show_hidden = !this.labels_ui.show_hidden;
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &MoveLabelUp, _, cx| this.nudge_label(true, cx)))
            .on_action(cx.listener(|this, _: &MoveLabelDown, _, cx| this.nudge_label(false, cx)))
            .on_action(
                cx.listener(|this, action: &SetLabelListVisibility, window, cx| {
                    if let Some(ix) = this.current_label() {
                        let command = LabelCommand::ListVisibility(ix, action.0);
                        this.run_label_command(command, window, cx);
                    }
                }),
            )
            .on_action(
                cx.listener(|this, action: &SetLabelMessageVisibility, window, cx| {
                    if let Some(ix) = this.current_label() {
                        let command = LabelCommand::MessageVisibility(ix, action.0);
                        this.run_label_command(command, window, cx);
                    }
                }),
            )
    }

    fn current_label(&self) -> Option<usize> {
        match self.nav {
            Nav::Label(ix) => Some(ix),
            _ => None,
        }
    }

    /// Move the open label one place up or down among the siblings the
    /// sidebar shows.
    fn nudge_label(&mut self, up: bool, cx: &mut Context<Self>) {
        let Some(ix) = self.current_label() else {
            return;
        };
        let (shown, _) = self.sidebar_labels();
        let listed: HashSet<&str> = shown
            .iter()
            .map(|row| self.data.labels[row.ix].id.as_str())
            .collect();
        let order = labels::nudge(
            &self.data.labels,
            &self.labels_ui.order,
            &self.data.labels[ix].id,
            up,
            |id| listed.contains(id),
        );
        if let Some(order) = order {
            self.labels_ui.order = order;
            cx.notify();
        }
    }

    /// Dropped: the label `id` takes the place of its sibling `target`.
    pub(crate) fn drop_label(&mut self, id: &str, target: &str, cx: &mut Context<Self>) {
        let order = labels::drop_on(&self.data.labels, &self.labels_ui.order, id, target);
        if let Some(order) = order {
            self.labels_ui.order = order;
            cx.notify();
        }
    }

    /// On the open label, else on one picked from a list.
    fn label_task(&mut self, task: LabelTask, window: &mut Window, cx: &mut Context<Self>) {
        let current = self
            .current_label()
            .filter(|&ix| task != LabelTask::Fold || has_children(&self.data.labels, ix));
        if let Some(ix) = current {
            return self.run_label_task(ix, task, window, cx);
        }
        let labels = &self.data.labels;
        let items: Vec<PaletteItem> = labels::label_tree(labels, &self.labels_ui.order)
            .into_iter()
            .filter(|n| task != LabelTask::Fold || n.children > 0)
            .map(|n| PaletteItem {
                command: PaletteCommand::Label(LabelCommand::Pick(n.ix, task)),
                title: labels[n.ix].name.clone().into(),
                subtitle: None,
                leading: Leading::Dot(labels[n.ix].color),
                binding: None,
                search_only: false,
            })
            .collect();
        let title = match task {
            LabelTask::Rename => "Rename which label?",
            LabelTask::Color => "Change the color of which label?",
            LabelTask::Delete => "Delete which label?",
            LabelTask::Visibility => "Change where which label shows?",
            LabelTask::Fold => "Fold or unfold which label?",
        };
        self.show_palette(items, "", title.into(), "No such label".into(), window, cx);
    }

    fn run_label_task(
        &mut self,
        ix: usize,
        task: LabelTask,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(label) = self.data.labels.get(ix).cloned() else {
            return;
        };
        match task {
            LabelTask::Rename => {
                self.prompt_label_name(Prompt::Rename(label.id.clone()), &label.name, window, cx)
            }
            LabelTask::Fold => self.toggle_label_folded(ix, cx),
            LabelTask::Color => {
                let current = labels::color_of(&label);
                let items = COLORS
                    .iter()
                    .enumerate()
                    .map(|(c, pair)| PaletteItem {
                        command: PaletteCommand::Label(LabelCommand::Color(ix, c)),
                        title: pair.name.into(),
                        subtitle: (current == Some(c)).then(|| "Current".into()),
                        leading: Leading::Dot(pair.color()),
                        binding: None,
                        search_only: false,
                    })
                    .collect();
                self.show_palette(
                    items,
                    "",
                    format!("Color for “{}”", label.name),
                    "No such color".into(),
                    window,
                    cx,
                );
            }
            LabelTask::Visibility => {
                let current = |on: bool| on.then(|| SharedString::from("Current"));
                let list = ListVisibility::ALL.into_iter().map(|v| PaletteItem {
                    command: PaletteCommand::Label(LabelCommand::ListVisibility(ix, v)),
                    title: format!("Label list: {}", v.title()).into(),
                    subtitle: current(label.list_visibility == v),
                    leading: Leading::Icon(Icon::PanelLeft),
                    binding: None,
                    search_only: false,
                });
                let rows = [true, false].into_iter().map(|shown| PaletteItem {
                    command: PaletteCommand::Label(LabelCommand::MessageVisibility(ix, shown)),
                    title: format!("Conversation list: {}", if shown { "Show" } else { "Hide" })
                        .into(),
                    subtitle: current(label.in_message_list == shown),
                    leading: Leading::Icon(Icon::Rows),
                    binding: None,
                    search_only: false,
                });
                self.show_palette(
                    list.chain(rows).collect(),
                    "",
                    format!("Where “{}” shows", label.name),
                    "No such option".into(),
                    window,
                    cx,
                );
            }
            LabelTask::Delete => {
                let children = labels::descendants(&self.data.labels, ix).len();
                let item = |command, title: String, subtitle: Option<&str>, icon| PaletteItem {
                    command: PaletteCommand::Label(command),
                    title: title.into(),
                    subtitle: subtitle.map(SharedString::from),
                    leading: Leading::Icon(icon),
                    binding: None,
                    search_only: false,
                };
                let mut items = Vec::new();
                let keeps_mail = Some("Its conversations stay where they are");
                if children > 0 {
                    let s = if children == 1 { "" } else { "s" };
                    items.push(item(
                        LabelCommand::Delete {
                            label: ix,
                            children: true,
                        },
                        format!("Delete “{}” and its {children} sublabel{s}", label.name),
                        keeps_mail,
                        Icon::Trash,
                    ));
                    items.push(item(
                        LabelCommand::Delete {
                            label: ix,
                            children: false,
                        },
                        format!("Delete only “{}”, keeping its sublabels", label.name),
                        keeps_mail,
                        Icon::Trash,
                    ));
                } else {
                    items.push(item(
                        LabelCommand::Delete {
                            label: ix,
                            children: false,
                        },
                        format!("Delete “{}”", label.name),
                        keeps_mail,
                        Icon::Trash,
                    ));
                }
                items.push(item(LabelCommand::Cancel, "Keep it".into(), None, Icon::X));
                self.show_palette(
                    items,
                    "",
                    format!("Delete “{}”?", label.name),
                    "No such option".into(),
                    window,
                    cx,
                );
            }
        }
    }

    /// The palette as a field for a label's name.
    fn prompt_label_name(
        &mut self,
        prompt: Prompt,
        initial: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let (placeholder, verb) = match prompt {
            Prompt::New => ("New label's name", "Create label"),
            Prompt::Rename(_) => ("Label's new name", "Rename to"),
        };
        self.labels_ui.prompt = Some(prompt);
        self.help_open = false;
        let palette = cx.new(|cx| {
            CommandPalette::prompt(
                initial,
                placeholder.into(),
                verb.into(),
                "Nest it with “/”: Work/Clients".into(),
                window,
                cx,
            )
        });
        let events = cx.subscribe_in(&palette, window, Self::on_palette_event);
        window.focus(&palette.focus_handle(cx), cx);
        self.palette = Some((palette, events));
        cx.notify();
    }

    /// Enter in the name prompt.
    pub(super) fn label_name_typed(&mut self, text: &str, _: &mut Window, cx: &mut Context<Self>) {
        match self.labels_ui.prompt.take() {
            Some(Prompt::New) => match labels::plan_create(&self.data.labels, text) {
                Ok(name) => {
                    let done = format!("Created “{name}”");
                    self.edit_labels(LabelEdit::Create { name }, done, cx);
                }
                Err(e) => self.show_error_toast(e, cx),
            },
            Some(Prompt::Rename(id)) => {
                let Some(ix) = self.data.labels.iter().position(|l| l.id == id) else {
                    return;
                };
                match labels::plan_rename(&self.data.labels, ix, text) {
                    Ok(renames) if renames.is_empty() => {}
                    Ok(renames) => {
                        for r in &renames {
                            if let Some(label) = self.data.labels.iter_mut().find(|l| l.id == r.id)
                            {
                                label.name = r.new.clone();
                            }
                        }
                        let done = format!("Renamed to “{}”", renames[0].new);
                        self.edit_labels(LabelEdit::Rename(renames), done, cx);
                    }
                    Err(e) => self.show_error_toast(e, cx),
                }
            }
            None => {}
        }
        cx.notify();
    }

    pub(super) fn run_label_command(
        &mut self,
        command: LabelCommand,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(label) = (match command {
            LabelCommand::Pick(ix, _)
            | LabelCommand::Color(ix, _)
            | LabelCommand::ListVisibility(ix, _)
            | LabelCommand::MessageVisibility(ix, _)
            | LabelCommand::Delete { label: ix, .. } => self.data.labels.get(ix).cloned(),
            LabelCommand::Cancel => None,
        }) else {
            return;
        };
        let id = label.id.clone();
        match command {
            LabelCommand::Pick(ix, task) => self.run_label_task(ix, task, window, cx),
            LabelCommand::Color(ix, c) => {
                let color = COLORS[c];
                self.data.labels[ix].color = color.color();
                let done = format!("“{}” is {}", label.name, color.name.to_lowercase());
                self.edit_labels(LabelEdit::Color { id, color }, done, cx);
            }
            LabelCommand::ListVisibility(ix, visibility) => {
                self.data.labels[ix].list_visibility = visibility;
                self.labels_ui.hidden_while_open = (self.current_label() == Some(ix)
                    && visibility != ListVisibility::Show)
                    .then(|| id.clone());
                let done = match visibility {
                    ListVisibility::Show => format!("“{}” shows in the sidebar", label.name),
                    ListVisibility::ShowIfUnread => {
                        format!("“{}” shows in the sidebar while unread", label.name)
                    }
                    ListVisibility::Hide => format!("“{}” is hidden from the sidebar", label.name),
                };
                self.edit_labels(LabelEdit::ListVisibility { id, visibility }, done, cx);
            }
            LabelCommand::MessageVisibility(ix, shown) => {
                self.data.labels[ix].in_message_list = shown;
                let done = if shown {
                    format!("Conversations show “{}”", label.name)
                } else {
                    format!("Conversations don't show “{}”", label.name)
                };
                self.edit_labels(LabelEdit::MessageVisibility { id, shown }, done, cx);
            }
            LabelCommand::Delete {
                label: ix,
                children,
            } => {
                let mut gone = vec![ix];
                if children {
                    gone.extend(labels::descendants(&self.data.labels, ix));
                }
                let ids: Vec<(String, String)> = gone
                    .iter()
                    .map(|&i| {
                        let l = &self.data.labels[i];
                        (l.id.clone(), l.name.clone())
                    })
                    .collect();
                let done = match ids.len() {
                    1 => format!("Deleted “{}”", label.name),
                    n => format!("Deleted “{}” and {} sublabels", label.name, n - 1),
                };
                let keep: Vec<String> = ids.iter().map(|(id, _)| id.clone()).collect();
                self.retain_labels(|l| !keep.contains(&l.id), cx);
                self.edit_labels(LabelEdit::Delete(ids), done, cx);
            }
            LabelCommand::Cancel => {}
        }
        cx.notify();
    }

    /// Drop labels from the sidebar now, keeping the view on its label.
    fn retain_labels(&mut self, keep: impl Fn(&Label) -> bool, cx: &mut Context<Self>) {
        let open = self
            .current_label()
            .map(|ix| self.data.labels[ix].id.clone());
        self.data.labels.retain(|l| keep(l));
        if let Some(id) = open {
            match self.data.labels.iter().position(|l| l.id == id) {
                Some(ix) => self.nav = Nav::Label(ix),
                None => self.set_nav(Nav::Folder(Folder::Inbox), cx),
            }
        }
    }

    /// The change goes to the backend; the labels reload from the store
    /// after (`LabelsChanged`), which puts them back if it failed.
    fn edit_labels(&mut self, edit: LabelEdit, done: String, cx: &mut Context<Self>) {
        let task = self.backend.update(cx, |b, cx| b.edit_labels(edit, cx));
        cx.spawn(async move |this, cx| {
            let result = task.await;
            this.update(cx, |this, cx| match result {
                Ok(()) => this.show_toast(ToastKind::Done, done, None, cx),
                Err(e) => this.show_error_toast(e, cx),
            })
            .ok();
        })
        .detach();
    }

    pub(crate) fn toggle_label_folded(&mut self, ix: usize, cx: &mut Context<Self>) {
        let Some(id) = self.data.labels.get(ix).map(|l| l.id.clone()) else {
            return;
        };
        if !self.labels_ui.folded.remove(&id) {
            self.labels_ui.folded.insert(id);
        }
        let mut ids: Vec<String> = self.labels_ui.folded.iter().cloned().collect();
        ids.sort();
        self.backend.read(cx).set_collapsed_labels(&ids);
        cx.notify();
    }

    /// Mark as important, or not when they all are already (FOL-12).
    pub(crate) fn toggle_important(&mut self, targets: Vec<usize>, cx: &mut Context<Self>) {
        if targets.is_empty() {
            return;
        }
        let important = targets.iter().any(|&ix| !self.data.threads[ix].important);
        let changed = self.ids_where(&targets, |t| t.important != important);
        for &ix in &targets {
            self.data.threads[ix].important = important;
        }
        let label = vec![gmail::IMPORTANT.to_string()];
        let (add, remove) = if important {
            (label, Vec::new())
        } else {
            (Vec::new(), label)
        };
        self.queue_labels(&targets, add.clone(), remove.clone(), cx);
        let (title, message) = if important {
            ("Mark as Important", "Marked as important")
        } else {
            ("Mark as Not Important", "Marked as not important")
        };
        self.push_label_change(title, changed, add, remove, Some(message.into()), cx);
        cx.notify();
    }

    /// Right-click on a label in the sidebar: open it, then its actions.
    pub(crate) fn show_label_menu(
        &mut self,
        ix: usize,
        position: Point<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.set_nav(Nav::Label(ix), cx);
        let label = &self.data.labels[ix];
        let mut list = NativeMenu::new();
        for v in ListVisibility::ALL {
            list = list.menu_with_check(
                v.title(),
                label.list_visibility == v,
                Box::new(SetLabelListVisibility(v)),
            );
        }
        let mut rows = NativeMenu::new();
        for shown in [true, false] {
            rows = rows.menu_with_check(
                if shown { "Show" } else { "Hide" },
                label.in_message_list == shown,
                Box::new(SetLabelMessageVisibility(shown)),
            );
        }
        let mut menu = NativeMenu::new()
            .menu("Rename…", Box::new(RenameLabel))
            .menu("Change Color…", Box::new(ChangeLabelColor))
            .submenu("In Label List", list)
            .submenu("In Conversation List", rows);
        if has_children(&self.data.labels, ix) {
            let folded = self.labels_ui.folded.contains(&label.id);
            menu = menu.menu(
                if folded {
                    "Unfold Sublabels"
                } else {
                    "Fold Sublabels"
                },
                Box::new(ToggleLabelFolded),
            );
        }
        menu.separator()
            .menu("Move Up", Box::new(MoveLabelUp))
            .menu("Move Down", Box::new(MoveLabelDown))
            .separator()
            .menu("New Label…", Box::new(NewLabel))
            .menu("Delete Label…", Box::new(DeleteLabel))
            .show(position, window, cx);
    }

    /// `l`: the labels as a tree, folded ones' children found by typing.
    pub(super) fn label_picker_items(&self, targets: &[usize]) -> Vec<PaletteItem> {
        let labels = &self.data.labels;
        let tree = labels::label_tree(labels, &self.labels_ui.order);
        let folded_away = |mut parent: Option<usize>| {
            while let Some(p) = parent {
                if self.labels_ui.folded.contains(&labels[p].id) {
                    return true;
                }
                parent = tree.iter().find(|n| n.ix == p).and_then(|n| n.parent);
            }
            false
        };
        tree.iter()
            .map(|n| {
                let label = &labels[n.ix];
                let on = targets
                    .iter()
                    .all(|&t| self.data.threads[t].labels.contains(&label.id));
                PaletteItem {
                    command: PaletteCommand::ToggleLabel(n.ix),
                    title: label.name.clone().into(),
                    subtitle: on.then(|| "Applied · ↵ removes it".into()),
                    leading: Leading::Nested(label.color, n.depth),
                    binding: None,
                    search_only: folded_away(n.parent),
                }
            })
            .collect()
    }
}

fn has_children(labels: &[Label], ix: usize) -> bool {
    !labels::descendants(labels, ix).is_empty()
}
