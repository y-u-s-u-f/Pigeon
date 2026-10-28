//! The user's labels (W4b): nesting by name (FOL-9), the colours on offer
//! and changes to them (FOL-6), which go to Gmail at once rather than
//! through the outbox: a new label needs Gmail's id before anything can
//! carry it, and a rename has to go parent first, then each child.
//!
//! - Nesting: `Work/Clients` sits under `Work` when there is a `Work`;
//!   otherwise it shows its whole name at the top.
//! - A rename renames every child too (Gmail doesn't): collisions are
//!   checked first; when a child fails, labels are read again and the
//!   message names the ones left as they were.
//! - Deleting never touches mail: Gmail takes the label off it.

use gpui::{Context, Task};

use super::{
    Backend, BackendEvent, Source,
    gmail::{self, GLabel, Gmail, GmailError, LabelList},
    outbox,
    store::Store,
};
use crate::data::{Label, ListVisibility};

/// Every colour Gmail takes for a label's text and background.
pub const GMAIL_PALETTE: [&str; 102] = [
    "#000000", "#434343", "#666666", "#999999", "#cccccc", "#efefef", "#f3f3f3", "#ffffff",
    "#fb4c2f", "#ffad47", "#fad165", "#16a766", "#43d692", "#4a86e8", "#a479e2", "#f691b3",
    "#f6c5be", "#ffe6c7", "#fef1d1", "#b9e4d0", "#c6f3de", "#c9daf8", "#e4d7f5", "#fcdee8",
    "#efa093", "#ffd6a2", "#fce8b3", "#89d3b2", "#a0eac9", "#a4c2f4", "#d0bcf1", "#fbc8d9",
    "#e66550", "#ffbc6b", "#fcda83", "#44b984", "#68dfa9", "#6d9eeb", "#b694e8", "#f7a7c0",
    "#cc3a21", "#eaa041", "#f2c960", "#149e60", "#3dc789", "#3c78d8", "#8e63ce", "#e07798",
    "#ac2b16", "#cf8933", "#d5ae49", "#0b804b", "#2a9c68", "#285bac", "#653e9b", "#b65775",
    "#822111", "#a46a21", "#aa8831", "#076239", "#1a764d", "#1c4587", "#41236d", "#83334c",
    "#464646", "#e7e7e7", "#0d3472", "#b6cff5", "#0d3b44", "#98d7e4", "#3d188e", "#e3d7ff",
    "#711a36", "#fbd3e0", "#8a1c0a", "#f2b2a8", "#7a2e0b", "#ffc8af", "#7a4706", "#ffdeb5",
    "#594c05", "#fbe983", "#684e07", "#fdedc1", "#0b4f30", "#b3efd3", "#04502e", "#a2dcc1",
    "#c2c2c2", "#4986e7", "#2da2bb", "#b99aff", "#994a64", "#f691b2", "#ff7537", "#ffad46",
    "#662e37", "#ebdbde", "#cca6ac", "#094228", "#42d692", "#16a765",
];

/// A label colour on offer: a background and text from Gmail's palette
/// that read well together.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ColorPair {
    pub name: &'static str,
    pub background: &'static str,
    pub text: &'static str,
}

pub const COLORS: [ColorPair; 9] = [
    pair("Red", "#e66550", "#ffffff"),
    pair("Orange", "#ffad47", "#000000"),
    pair("Yellow", "#fad165", "#000000"),
    pair("Green", "#16a766", "#ffffff"),
    pair("Teal", "#2da2bb", "#ffffff"),
    pair("Blue", "#4a86e8", "#ffffff"),
    pair("Purple", "#a479e2", "#ffffff"),
    pair("Pink", "#e07798", "#ffffff"),
    pair("Grey", "#999999", "#ffffff"),
];

const fn pair(name: &'static str, background: &'static str, text: &'static str) -> ColorPair {
    ColorPair {
        name,
        background,
        text,
    }
}

impl ColorPair {
    pub fn color(self) -> gpui::Hsla {
        crate::data::from_hex(self.background).expect("a valid colour")
    }

    /// Both colours are ones Gmail takes.
    pub fn is_valid(self) -> bool {
        GMAIL_PALETTE.contains(&self.background) && GMAIL_PALETTE.contains(&self.text)
    }
}

/// The pair a label has now, if it's one on offer.
pub fn color_of(label: &Label) -> Option<usize> {
    let hex = crate::data::to_hex(label.color);
    COLORS.iter().position(|c| c.background == hex)
}

/// A label in the tree, in the order the sidebar lists them: each parent,
/// then its children, by name.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LabelNode {
    /// Index into the labels.
    pub ix: usize,
    pub depth: usize,
    /// The name under its parent: `Clients` for `Work/Clients`.
    pub name: String,
    /// Index into the labels.
    pub parent: Option<usize>,
    pub children: usize,
}

/// The label whose name is the longest `<name>/` start of this one's.
fn parent_of(labels: &[Label], ix: usize) -> Option<usize> {
    let name = &labels[ix].name;
    labels
        .iter()
        .enumerate()
        .filter(|(p, l)| {
            *p != ix
                && name.len() > l.name.len() + 1
                && name.starts_with(l.name.as_str())
                && name.as_bytes()[l.name.len()] == b'/'
        })
        .max_by_key(|(_, l)| l.name.len())
        .map(|(p, _)| p)
}

/// The labels as a tree. Siblings go by their place in `order` (label ids,
/// the user's own arrangement), those not in it after, by name.
pub fn label_tree(labels: &[Label], order: &[String]) -> Vec<LabelNode> {
    let parents: Vec<Option<usize>> = (0..labels.len()).map(|ix| parent_of(labels, ix)).collect();
    let mut out = Vec::with_capacity(labels.len());
    fn walk(
        labels: &[Label],
        order: &[String],
        parents: &[Option<usize>],
        parent: Option<usize>,
        depth: usize,
        out: &mut Vec<LabelNode>,
    ) {
        let mut kids: Vec<usize> = (0..labels.len())
            .filter(|&ix| parents[ix] == parent)
            .collect();
        kids.sort_by_key(|&ix| {
            let place = order.iter().position(|id| *id == labels[ix].id);
            (place.unwrap_or(usize::MAX), labels[ix].name.to_lowercase())
        });
        for ix in kids {
            let name = match parent {
                Some(p) => labels[ix].name[labels[p].name.len() + 1..].to_string(),
                None => labels[ix].name.clone(),
            };
            let at = out.len();
            out.push(LabelNode {
                ix,
                depth,
                name,
                parent,
                children: 0,
            });
            walk(labels, order, parents, Some(ix), depth + 1, out);
            out[at].children = parents.iter().filter(|p| **p == Some(ix)).count();
        }
    }
    walk(labels, order, &parents, None, 0, &mut out);
    out
}

/// The new `order` after moving label `id` to the place `pick` chooses
/// among its siblings (ids in order, and its own place); `None` when
/// it doesn't move.
fn moved(
    labels: &[Label],
    order: &[String],
    id: &str,
    pick: impl FnOnce(&[&str], usize) -> Option<usize>,
) -> Option<Vec<String>> {
    let tree = label_tree(labels, order);
    let parent = tree.iter().find(|n| labels[n.ix].id == id)?.parent;
    let mut siblings: Vec<&str> = tree
        .iter()
        .filter(|n| n.parent == parent)
        .map(|n| labels[n.ix].id.as_str())
        .collect();
    let from = siblings.iter().position(|s| *s == id)?;
    let to = pick(&siblings, from).filter(|&to| to != from)?;
    let label = siblings.remove(from);
    siblings.insert(to, label);
    let mut next = siblings.into_iter();
    Some(
        tree.iter()
            .map(|n| match n.parent == parent {
                true => next.next().unwrap_or_default(),
                false => labels[n.ix].id.as_str(),
            })
            .map(String::from)
            .collect(),
    )
}

/// Drop label `id` on its sibling `target`: it takes that place.
pub fn drop_on(labels: &[Label], order: &[String], id: &str, target: &str) -> Option<Vec<String>> {
    moved(labels, order, id, |siblings, _| {
        siblings.iter().position(|s| *s == target)
    })
}

/// Move label `id` one place up or down among the siblings `listed`
/// accepts (those the sidebar shows).
pub fn nudge(
    labels: &[Label],
    order: &[String],
    id: &str,
    up: bool,
    listed: impl Fn(&str) -> bool,
) -> Option<Vec<String>> {
    moved(labels, order, id, |siblings, from| {
        let mut at = from;
        loop {
            at = if up { at.checked_sub(1)? } else { at + 1 };
            if listed(siblings.get(at)?) {
                return Some(at);
            }
        }
    })
}

/// Every label named under this one (`Work/…`), at any depth.
pub fn descendants(labels: &[Label], ix: usize) -> Vec<usize> {
    let prefix = format!("{}/", labels[ix].name);
    (0..labels.len())
        .filter(|&i| labels[i].name.starts_with(&prefix))
        .collect()
}

/// A name as Gmail takes it: trimmed, with no empty part between slashes.
pub fn check_name(name: &str) -> Result<String, String> {
    let name = name.trim();
    if name.is_empty() {
        return Err("A label needs a name".into());
    }
    if name.split('/').any(|part| part.trim().is_empty()) {
        return Err("A label name can't start or end with “/”, or have “//”".into());
    }
    if gmail::is_system_label(&name.to_uppercase()) {
        return Err(format!("“{name}” is one of Gmail's own names"));
    }
    Ok(name.split('/').map(str::trim).collect::<Vec<_>>().join("/"))
}

fn taken(labels: &[Label], name: &str, except: &[usize]) -> bool {
    labels
        .iter()
        .enumerate()
        .any(|(ix, l)| !except.contains(&ix) && l.name.eq_ignore_ascii_case(name))
}

/// A new label's name, unless a label has it already.
pub fn plan_create(labels: &[Label], name: &str) -> Result<String, String> {
    let name = check_name(name)?;
    if taken(labels, &name, &[]) {
        return Err(format!("There's a label called “{name}” already"));
    }
    Ok(name)
}

/// One label's new name.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Rename {
    pub id: String,
    pub old: String,
    pub new: String,
}

/// The label and each of its children renamed, parent first; refused when
/// any new name is taken by a label not being renamed.
pub fn plan_rename(labels: &[Label], ix: usize, name: &str) -> Result<Vec<Rename>, String> {
    let name = check_name(name)?;
    let old = labels[ix].name.clone();
    if name == old {
        return Ok(Vec::new());
    }
    if name.starts_with(&format!("{old}/")) {
        return Err("A label can't go inside itself".into());
    }
    let mut moving = vec![ix];
    moving.extend(descendants(labels, ix));
    let renames: Vec<Rename> = moving
        .iter()
        .map(|&i| Rename {
            id: labels[i].id.clone(),
            old: labels[i].name.clone(),
            new: format!("{name}{}", &labels[i].name[old.len()..]),
        })
        .collect();
    if let Some(clash) = renames.iter().find(|r| taken(labels, &r.new, &moving)) {
        return Err(format!("There's a label called “{}” already", clash.new));
    }
    Ok(renames)
}

/// A change to the user's labels.
#[derive(Clone, Debug, PartialEq)]
pub enum LabelEdit {
    Create {
        name: String,
    },
    /// From `plan_rename`: the label first, then its children.
    Rename(Vec<Rename>),
    Color {
        id: String,
        color: ColorPair,
    },
    ListVisibility {
        id: String,
        visibility: ListVisibility,
    },
    MessageVisibility {
        id: String,
        shown: bool,
    },
    /// By (id, name), parents before children.
    Delete(Vec<(String, String)>),
}

impl LabelEdit {
    fn patch(&self) -> Option<(&str, serde_json::Value)> {
        Some(match self {
            LabelEdit::Color { id, color } => (
                id,
                serde_json::json!({ "color": {
                    "backgroundColor": color.background,
                    "textColor": color.text,
                }}),
            ),
            LabelEdit::ListVisibility { id, visibility } => (
                id,
                serde_json::json!({ "labelListVisibility": visibility.gmail() }),
            ),
            LabelEdit::MessageVisibility { id, shown } => (
                id,
                serde_json::json!({
                    "messageListVisibility": if *shown { "show" } else { "hide" },
                }),
            ),
            _ => return None,
        })
    }
}

/// "“A” and “B”", "“A”, “B” and “C”".
fn names(names: &[String]) -> String {
    let quoted: Vec<String> = names.iter().map(|n| format!("“{n}”")).collect();
    match quoted.split_last() {
        None => String::new(),
        Some((last, [])) => last.clone(),
        Some((last, rest)) => format!("{} and {last}", rest.join(", ")),
    }
}

fn why(error: &GmailError) -> String {
    match error {
        GmailError::Network(_) => "Label changes need a connection".into(),
        e => e.to_string(),
    }
}

/// What a commit did: the threads it took a deleted label off, for the
/// app to reload.
#[derive(Debug, Default, PartialEq)]
pub struct Committed {
    pub threads: Vec<String>,
}

/// Read every label again after a change that went part way.
fn resync(gmail: &Gmail, store: &Store, account: &str) {
    match gmail.call::<LabelList>(&gmail::labels_list_request()) {
        Ok(list) => {
            store.replace_gmail_labels(account, &list.labels).ok();
        }
        Err(e) => eprintln!("Pigeon: couldn't read the labels again: {e}"),
    }
}

/// Make the change in Gmail, then in the store. Blocking.
pub fn commit(
    gmail: &Gmail,
    store: &Store,
    account: &str,
    edit: &LabelEdit,
) -> Result<Committed, String> {
    // Gmail's answer to a patch can leave out fields, and to a create
    // `type` and the visibilities, so a create is stored as the user label
    // it made, shown, and a change as what was asked, once Gmail took it.
    let created = |label: &GLabel| {
        let mut label = label.clone();
        if label.kind.is_empty() {
            label.kind = "user".into();
        }
        label
            .label_list_visibility
            .get_or_insert_with(|| ListVisibility::Show.gmail().into());
        label
            .message_list_visibility
            .get_or_insert_with(|| "show".into());
        store.put_gmail_label(account, &label).ok();
    };
    let changed = |edit: &LabelEdit| {
        apply_local(store, account, edit).ok();
    };
    match edit {
        LabelEdit::Create { name } => {
            let label: GLabel = gmail
                .call(&gmail::label_create_request(name))
                .map_err(|e| format!("Couldn't create “{name}”: {}", why(&e)))?;
            created(&label);
            Ok(Committed::default())
        }
        LabelEdit::Rename(renames) => {
            let Some((first, children)) = renames.split_first() else {
                return Ok(Committed::default());
            };
            let rename = |r: &Rename| {
                gmail.call::<GLabel>(&gmail::label_patch_request(
                    &r.id,
                    &serde_json::json!({ "name": r.new }),
                ))
            };
            rename(first).map_err(|e| format!("Couldn't rename “{}”: {}", first.old, why(&e)))?;
            changed(&LabelEdit::Rename(vec![first.clone()]));
            let mut failed = Vec::new();
            for child in children {
                match rename(child) {
                    Ok(_) => changed(&LabelEdit::Rename(vec![child.clone()])),
                    Err(_) => failed.push(child.old.clone()),
                }
            }
            if failed.is_empty() {
                return Ok(Committed::default());
            }
            resync(gmail, store, account);
            Err(format!(
                "Renamed “{}”, but not {}",
                first.old,
                names(&failed)
            ))
        }
        LabelEdit::Delete(labels) => {
            let mut done = Committed::default();
            let mut failed = Vec::new();
            let mut error = None;
            for (id, name) in labels {
                match gmail.call_empty(&gmail::label_delete_request(id)) {
                    Ok(()) => done
                        .threads
                        .extend(store.delete_label(account, id).unwrap_or_default()),
                    Err(e) => {
                        failed.push(name.clone());
                        error.get_or_insert(e);
                    }
                }
            }
            match error {
                None => Ok(done),
                Some(e) => {
                    resync(gmail, store, account);
                    Err(format!("Couldn't delete {}: {}", names(&failed), why(&e)))
                }
            }
        }
        edit => {
            let (id, fields) = edit.patch().expect("a patch");
            gmail
                .call::<GLabel>(&gmail::label_patch_request(id, &fields))
                .map_err(|e| format!("Couldn't change the label: {}", why(&e)))?;
            changed(edit);
            Ok(Committed::default())
        }
    }
}

/// Demo mode: the change, in the store only.
pub fn apply_local(store: &Store, account: &str, edit: &LabelEdit) -> Result<Committed, String> {
    let labels = store.load_labels(account).map_err(|e| e.to_string())?;
    let find = |id: &str| labels.iter().find(|l| l.id == id).cloned();
    let save = |label: Label| {
        store
            .put_labels(account, &[label])
            .map_err(|e| e.to_string())
    };
    match edit {
        LabelEdit::Create { name } => save(Label::new(
            format!("label-{}", uuid::Uuid::new_v4().simple()),
            name.clone(),
            COLORS[COLORS.len() - 1].color(),
        ))?,
        LabelEdit::Rename(renames) => {
            for r in renames {
                if let Some(label) = find(&r.id) {
                    save(Label {
                        name: r.new.clone(),
                        ..label
                    })?;
                }
            }
        }
        LabelEdit::Color { id, color } => {
            if let Some(label) = find(id) {
                save(Label {
                    color: color.color(),
                    ..label
                })?;
            }
        }
        LabelEdit::ListVisibility { id, visibility } => {
            if let Some(label) = find(id) {
                save(Label {
                    list_visibility: *visibility,
                    ..label
                })?;
            }
        }
        LabelEdit::MessageVisibility { id, shown } => {
            if let Some(label) = find(id) {
                save(Label {
                    in_message_list: *shown,
                    ..label
                })?;
            }
        }
        LabelEdit::Delete(labels) => {
            let mut done = Committed::default();
            for (id, _) in labels {
                done.threads
                    .extend(store.delete_label(account, id).map_err(|e| e.to_string())?);
            }
            return Ok(done);
        }
    }
    Ok(Committed::default())
}

const COLLAPSED_KEY: &str = "collapsed_labels";

impl Backend {
    /// Change the user's labels (FOL-6, FOL-10). Demo at once; Google at
    /// the user's priority, refused offline. Either way `LabelsChanged`
    /// follows, so the sidebar shows what the store has: on a failure,
    /// the labels as they were.
    pub fn edit_labels(
        &mut self,
        edit: LabelEdit,
        cx: &mut Context<Self>,
    ) -> Task<Result<(), String>> {
        let account = self.email().to_string();
        let store = self.store.clone();
        if self.source == Source::Demo {
            let result = apply_local(&store, &account, &edit);
            self.labels_edited(&result, cx);
            return Task::ready(result.map(|_| ()));
        }
        let Some(transport) = self.transport() else {
            return Task::ready(Err("Sign in to change labels".into()));
        };
        if transport.is_offline() {
            cx.emit(BackendEvent::LabelsChanged);
            return Task::ready(Err("Label changes need a connection".into()));
        }
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    let gmail = Gmail::new(std::sync::Arc::new(outbox::UserPriority(transport)));
                    commit(&gmail, &store, &account, &edit)
                })
                .await;
            this.update(cx, |this, cx| this.labels_edited(&result, cx))
                .ok();
            result.map(|_| ())
        })
    }

    fn labels_edited(&mut self, result: &Result<Committed, String>, cx: &mut Context<Self>) {
        cx.emit(BackendEvent::LabelsChanged);
        if let Ok(done) = result
            && !done.threads.is_empty()
        {
            cx.emit(BackendEvent::ThreadsChanged(done.threads.clone()));
        }
    }

    /// The labels whose children the sidebar and the picker fold away
    /// (FOL-9), by id. Kept with the settings, so demo mode keeps them too.
    pub fn collapsed_labels(&self) -> Vec<String> {
        self.prefs
            .load_settings()
            .ok()
            .and_then(|mut map| map.remove(COLLAPSED_KEY))
            .and_then(|value| serde_json::from_value(value).ok())
            .unwrap_or_default()
    }

    pub fn set_collapsed_labels(&self, ids: &[String]) {
        let mut map = serde_json::Map::new();
        map.insert(COLLAPSED_KEY.into(), serde_json::json!(ids));
        if let Err(e) = self.prefs.save_settings(&map) {
            eprintln!("Pigeon: couldn't save the folded labels: {e:#}");
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::backend::http::FakeTransport;

    fn labels(names: &[&str]) -> Vec<Label> {
        names
            .iter()
            .enumerate()
            .map(|(ix, n)| Label::new(format!("Label_{ix}"), *n, gpui::black()))
            .collect()
    }

    #[test]
    fn colour_pairs_are_from_gmails_palette() {
        assert_eq!(GMAIL_PALETTE.len(), 102);
        for pair in COLORS {
            assert!(pair.is_valid(), "{}", pair.name);
            assert!(crate::data::from_hex(pair.background).is_some());
        }
        assert!(!pair("Off", "#123456", "#ffffff").is_valid());
        assert!(!pair("Off", "#4a86e8", "#123456").is_valid());
        let mut label = Label::new("a", "A", COLORS[5].color());
        assert_eq!(color_of(&label), Some(5));
        label.color = gpui::black();
        assert_eq!(color_of(&label), None);
    }

    #[test]
    fn the_tree_nests_by_name() {
        let l = labels(&[
            "Work/Clients",
            "travel",
            "Work",
            "Work/Clients/Acme",
            "Orphan/Kid",
            "Work-Old",
        ]);
        let tree = label_tree(&l, &[]);
        let rows: Vec<(usize, &str, usize)> = tree
            .iter()
            .map(|n| (n.depth, n.name.as_str(), n.children))
            .collect();
        assert_eq!(
            rows,
            [
                (0, "Orphan/Kid", 0),
                (0, "travel", 0),
                (0, "Work", 1),
                (1, "Clients", 1),
                (2, "Acme", 0),
                (0, "Work-Old", 0),
            ]
        );
        assert_eq!(tree[3].parent, Some(2));
        let mut kids = descendants(&l, 2);
        kids.sort();
        assert_eq!(kids, [0, 3]);
    }

    #[test]
    fn siblings_can_be_rearranged() {
        // Label_0..: A, B, B/x, B/y, C
        let l = labels(&["A", "B", "B/x", "B/y", "C"]);
        let names = |order: &[String]| -> Vec<String> {
            label_tree(&l, order)
                .iter()
                .map(|n| l[n.ix].name.clone())
                .collect()
        };
        // C dropped on A takes its place.
        let order = drop_on(&l, &[], "Label_4", "Label_0").unwrap();
        assert_eq!(names(&order), ["C", "A", "B", "B/x", "B/y"]);
        // A dropped on C goes after it; B's children stay with B.
        let order = drop_on(&l, &[], "Label_0", "Label_4").unwrap();
        assert_eq!(names(&order), ["B", "B/x", "B/y", "C", "A"]);
        // Another parent's label, itself: nothing.
        assert!(drop_on(&l, &[], "Label_2", "Label_0").is_none());
        assert!(drop_on(&l, &[], "Label_0", "Label_0").is_none());
        // Up and down by one, within siblings; the ends stay put.
        let order = nudge(&l, &[], "Label_3", true, |_| true).unwrap();
        assert_eq!(names(&order), ["A", "B", "B/y", "B/x", "C"]);
        assert!(nudge(&l, &order, "Label_3", true, |_| true).is_none());
        assert!(nudge(&l, &order, "Label_2", false, |_| true).is_none());
        // Hidden siblings are skipped over; none left to jump to: nothing.
        let order = nudge(&l, &[], "Label_0", false, |id| id != "Label_1").unwrap();
        assert_eq!(names(&order), ["B", "B/x", "B/y", "C", "A"]);
        assert!(nudge(&l, &[], "Label_4", false, |_| true).is_none());
        assert!(nudge(&l, &[], "Label_0", false, |id| id == "Label_0").is_none());
    }

    #[test]
    fn names_are_checked() {
        assert_eq!(check_name("  Work / Clients ").unwrap(), "Work/Clients");
        assert!(check_name("").is_err());
        assert!(check_name("/Work").is_err());
        assert!(check_name("Work//x").is_err());
        assert!(check_name("inbox").is_err());
        let l = labels(&["Work"]);
        assert!(plan_create(&l, "work").is_err());
        assert_eq!(plan_create(&l, "Work/New").unwrap(), "Work/New");
    }

    #[test]
    fn a_rename_takes_the_children_along_unless_a_name_is_taken() {
        let l = labels(&[
            "Work",
            "Work/Clients",
            "Work/Clients/Acme",
            "Jobs/Clients",
            "Workshop",
        ]);
        let plan = plan_rename(&l, 0, "Jobs2").unwrap();
        let new: Vec<&str> = plan.iter().map(|r| r.new.as_str()).collect();
        assert_eq!(new, ["Jobs2", "Jobs2/Clients", "Jobs2/Clients/Acme"]);
        assert_eq!(plan[0].id, "Label_0");
        // `Jobs/Clients` exists: the child would collide.
        let clash = plan_rename(&l, 0, "Jobs").unwrap_err();
        assert!(clash.contains("Jobs/Clients"), "{clash}");
        // Another label's name, any case.
        assert!(plan_rename(&l, 0, "workshop").is_err());
        // Case only: the label itself doesn't count.
        assert_eq!(plan_rename(&l, 4, "WorkShop").unwrap().len(), 1);
        assert!(plan_rename(&l, 0, "Work/Inner").is_err());
        assert_eq!(plan_rename(&l, 0, "Work").unwrap(), []);
    }

    const LABELS: &str = "/gmail/v1/users/me/labels";

    fn store_with(labels: &[Label]) -> Store {
        let store = Store::open_in_memory().unwrap();
        store.put_labels("me@x.com", labels).unwrap();
        store
    }

    #[test]
    fn a_child_that_fails_to_rename_resyncs_and_is_named() {
        let l = labels(&["Work", "Work/A", "Work/B"]);
        let store = store_with(&l);
        let label =
            |id: &str, name: &str| format!(r#"{{"id":"{id}","name":"{name}","type":"user"}}"#);
        let fake = Arc::new(
            FakeTransport::new()
                .on(
                    "PATCH",
                    &format!("{LABELS}/Label_0"),
                    &[],
                    &[(200, &label("Label_0", "Job"))],
                )
                .on(
                    "PATCH",
                    &format!("{LABELS}/Label_1"),
                    &[],
                    &[(200, &label("Label_1", "Job/A"))],
                )
                .on(
                    "PATCH",
                    &format!("{LABELS}/Label_2"),
                    &[],
                    &[(
                        400,
                        r#"{"error":{"code":400,"message":"Invalid label name"}}"#,
                    )],
                )
                .on(
                    "GET",
                    LABELS,
                    &[],
                    &[(
                        200,
                        &format!(
                            r#"{{"labels":[{},{},{}]}}"#,
                            label("Label_0", "Job"),
                            label("Label_1", "Job/A"),
                            label("Label_2", "Work/B")
                        ),
                    )],
                ),
        );
        let plan = plan_rename(&l, 0, "Job").unwrap();
        let err = commit(
            &Gmail::new(fake.clone()),
            &store,
            "me@x.com",
            &LabelEdit::Rename(plan),
        )
        .unwrap_err();
        assert_eq!(err, "Renamed “Work”, but not “Work/B”");
        let mut names: Vec<String> = store
            .load_labels("me@x.com")
            .unwrap()
            .into_iter()
            .map(|l| l.name)
            .collect();
        names.sort();
        assert_eq!(names, ["Job", "Job/A", "Work/B"]);
        let requests = fake.requests();
        assert_eq!(requests.len(), 4);
        assert_eq!(
            String::from_utf8_lossy(&requests[0].body),
            r#"{"name":"Job"}"#
        );
        assert_eq!(requests[3].method, "GET");
    }

    #[test]
    fn a_failed_parent_rename_changes_nothing() {
        let l = labels(&["Work", "Work/A"]);
        let store = store_with(&l);
        let fake = Arc::new(FakeTransport::new().on(
            "PATCH",
            &format!("{LABELS}/Label_0"),
            &[],
            &[(0, "")],
        ));
        let plan = plan_rename(&l, 0, "Job").unwrap();
        let err = commit(
            &Gmail::new(fake.clone()),
            &store,
            "me@x.com",
            &LabelEdit::Rename(plan),
        )
        .unwrap_err();
        assert_eq!(
            err,
            "Couldn't rename “Work”: Label changes need a connection"
        );
        assert_eq!(fake.requests().len(), 1);
        assert_eq!(store.load_labels("me@x.com").unwrap()[0].name, "Work");
    }

    #[test]
    fn create_patch_and_delete_go_to_gmail() {
        let store = store_with(&labels(&["Old"]));
        let fake = Arc::new(
            FakeTransport::new()
                .on(
                    "POST",
                    LABELS,
                    &[],
                    // Without `type` or visibilities, as Gmail can answer.
                    &[(200, r#"{"id":"Label_9","name":"Pigeon Test"}"#)],
                )
                .on(
                    "PATCH",
                    &format!("{LABELS}/Label_9"),
                    &[],
                    // Partial answers: what was asked is what's stored.
                    &[
                        (200, r#"{"id":"Label_9","name":"Pigeon Test"}"#),
                        (200, r#"{"id":"Label_9","name":"Pigeon Test"}"#),
                        (200, r#"{"id":"Label_9","name":"Pigeon Test"}"#),
                    ],
                )
                .on("DELETE", &format!("{LABELS}/Label_0"), &[], &[(204, "")]),
        );
        let gmail = Gmail::new(fake.clone());
        let account = "me@x.com";
        commit(
            &gmail,
            &store,
            account,
            &LabelEdit::Create {
                name: "Pigeon Test".into(),
            },
        )
        .unwrap();
        commit(
            &gmail,
            &store,
            account,
            &LabelEdit::Color {
                id: "Label_9".into(),
                color: COLORS[5],
            },
        )
        .unwrap();
        // A new label lists at once, shown.
        let stored = store.load_labels(account).unwrap();
        assert_eq!(stored.len(), 2);
        assert_eq!(stored[1].list_visibility, ListVisibility::Show);
        assert!(stored[1].in_message_list);
        commit(
            &gmail,
            &store,
            account,
            &LabelEdit::ListVisibility {
                id: "Label_9".into(),
                visibility: ListVisibility::ShowIfUnread,
            },
        )
        .unwrap();
        commit(
            &gmail,
            &store,
            account,
            &LabelEdit::MessageVisibility {
                id: "Label_9".into(),
                shown: false,
            },
        )
        .unwrap();
        let done = commit(
            &gmail,
            &store,
            account,
            &LabelEdit::Delete(vec![("Label_0".into(), "Old".into())]),
        )
        .unwrap();
        assert_eq!(done, Committed::default());
        let stored = store.load_labels(account).unwrap();
        assert_eq!(stored.len(), 1);
        assert_eq!(stored[0].name, "Pigeon Test");
        assert_eq!(color_of(&stored[0]), Some(5));
        assert_eq!(stored[0].list_visibility, ListVisibility::ShowIfUnread);
        assert!(!stored[0].in_message_list);
        let requests = fake.requests();
        assert_eq!(
            String::from_utf8_lossy(&requests[1].body),
            r##"{"color":{"backgroundColor":"#4a86e8","textColor":"#ffffff"}}"##
        );
        assert_eq!(
            String::from_utf8_lossy(&requests[2].body),
            r#"{"labelListVisibility":"labelShowIfUnread"}"#
        );
        assert_eq!(requests[4].method, "DELETE");
    }

    #[test]
    fn deleting_locally_keeps_the_mail() {
        let store = Store::open_in_memory().unwrap();
        let account = crate::backend::demo::seed(&store).unwrap().email;
        let before = store.load_all_threads(&account).unwrap();
        let with_work = before
            .iter()
            .filter(|t| t.labels.contains(&"work".to_string()))
            .count();
        assert!(with_work > 0);
        let done = apply_local(
            &store,
            &account,
            &LabelEdit::Delete(vec![("work".into(), "Work".into())]),
        )
        .unwrap();
        assert_eq!(done.threads.len(), with_work);
        let after = store.load_all_threads(&account).unwrap();
        assert_eq!(after.len(), before.len());
        assert!(
            after
                .iter()
                .all(|t| !t.labels.contains(&"work".to_string()))
        );
        assert!(
            store
                .load_labels(&account)
                .unwrap()
                .iter()
                .all(|l| l.id != "work")
        );
    }

    #[test]
    fn visibility_changes_locally() {
        let store = store_with(&labels(&["A"]));
        apply_local(
            &store,
            "me@x.com",
            &LabelEdit::ListVisibility {
                id: "Label_0".into(),
                visibility: ListVisibility::Hide,
            },
        )
        .unwrap();
        let label = &store.load_labels("me@x.com").unwrap()[0];
        assert_eq!(label.list_visibility, ListVisibility::Hide);
        assert!(label.in_message_list);
    }
}

#[cfg(test)]
mod important_tests {
    use crate::backend::{MailOp, gmail, store::modify_thread};
    use crate::data::{Contact, Folder, Message, Thread};

    #[test]
    fn important_is_a_label_change_that_goes_at_once() {
        let op = MailOp::Modify {
            threads: vec!["t".into()],
            add: vec![gmail::IMPORTANT.into()],
            remove: Vec::new(),
        };
        assert!(!op.undoable());
        let mut thread = Thread::local(
            "t".into(),
            "S".into(),
            Folder::Inbox,
            vec![Message::new(Contact::default())],
            None,
        );
        modify_thread(
            &mut thread,
            &[gmail::IMPORTANT.into()],
            &[],
            &Default::default(),
        );
        assert!(thread.important);
        assert!(thread.labels.is_empty(), "a system label, not a user one");
        modify_thread(
            &mut thread,
            &[],
            &[gmail::IMPORTANT.into()],
            &Default::default(),
        );
        assert!(!thread.important);
    }
}
