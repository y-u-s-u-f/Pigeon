//! The MCP tools: their table (names, schemas, the switch each needs) and
//! what they do, on the main thread. Reads come from the backend's store
//! with queued changes shown as made, or from the open calendar. Writes are
//! the backend ops the UI queues, through the outbox, so undo windows,
//! offline queueing and sync work as for the user's own changes; the
//! windows then read the change back (`ThreadsChanged`) or, for the
//! calendar, are changed in place as the calendar's own edits are.

use chrono::{DateTime, Datelike, Duration, Local, NaiveDate, NaiveDateTime};
use gpui::{App, Entity, Task};
use serde_json::{Value, json};

use super::{Capability, resolve};
use crate::{
    app::{ComposeKind, shell},
    backend::{
        Backend, BackendEvent, CalOp, MailOp, Outgoing, SendUpdates,
        calendar::{Access, access, has_guests, is_organizer, refusal},
        gmail, mime,
    },
    calendar::{
        Attendee, CalendarData, Event, EventStatus, RsvpStatus, SeriesScope, occurrences,
        time::midnight,
    },
    data::{Body, Contact, Folder, MailData, Thread},
    search::{Query, Scope},
    settings,
    ui::{calendar::CalendarView, composer::parse_address},
};

pub struct ToolDef {
    pub name: &'static str,
    pub description: &'static str,
    pub capability: Capability,
    schema: fn() -> Value,
    destructive: bool,
    /// It reaches people beyond the user: mail, or guests' invitations.
    open_world: bool,
}

const IDS: &str = "Conversation ids, from search_mail.";

fn ids_schema(extra: Value) -> Value {
    let mut schema = json!({
        "type": "object",
        "properties": {"ids": {"type": "array", "items": {"type": "string"}, "minItems": 1, "description": IDS}},
        "required": ["ids"],
    });
    if let (Some(props), Value::Object(extra)) = (schema["properties"].as_object_mut(), extra) {
        props.extend(extra);
    }
    schema
}

fn message_schema(draft: bool) -> Value {
    let addresses = |what: &str| {
        json!({"type": "array", "items": {"type": "string"},
            "description": format!("{what}: \"Name <a@b.com>\" or a bare address.")})
    };
    let mut schema = json!({
        "type": "object",
        "properties": {
            "to": addresses("Recipients"),
            "cc": addresses("Cc"),
            "bcc": addresses("Bcc"),
            "subject": {"type": "string", "description": "A reply's defaults to \"Re: \" and the conversation's subject."},
            "body": {"type": "string", "description": "The message, in Markdown."},
            "reply_to_message_id": {"type": "string", "description": "A message id from read_conversation, to reply in its conversation. Recipients default to its sender."},
        },
    });
    if !draft {
        schema["properties"]["draft_id"] = json!({"type": "string", "description": "Send this draft as it is, from create_draft or Drafts; the other fields are then ignored."});
    }
    schema
}

pub const TOOLS: &[ToolDef] = &[
    ToolDef {
        name: "search_mail",
        description: "Search the user's mail with Gmail's syntax, as in Pigeon's search: free text, from:, to:, subject:, label:, is:unread, is:starred, has:attachment, in:inbox|sent|drafts|archive|spam|trash|anywhere, after:2026/10/01, newer_than:7d. Without in:, searches All Mail except Spam and Trash. Newest first; an empty query lists the newest conversations.",
        capability: Capability::ReadMail,
        schema: || {
            json!({"type": "object", "properties": {
                "query": {"type": "string"},
                "limit": {"type": "integer", "minimum": 1, "maximum": 100, "default": 20},
            }})
        },
        destructive: false,
        open_world: false,
    },
    ToolDef {
        name: "read_conversation",
        description: "Read a conversation: each message's headers, plain-text body and attachments (names and sizes).",
        capability: Capability::ReadMail,
        schema: || json!({"type": "object", "properties": {"id": {"type": "string", "description": "A conversation id from search_mail."}}, "required": ["id"]}),
        destructive: false,
        open_world: false,
    },
    ToolDef {
        name: "list_labels",
        description: "The user's Gmail labels: id and name.",
        capability: Capability::ReadMail,
        schema: || json!({"type": "object", "properties": {}}),
        destructive: false,
        open_world: false,
    },
    ToolDef {
        name: "archive",
        description: "Archive conversations in the Inbox. Goes after Pigeon's 5-second undo window.",
        capability: Capability::OrganizeMail,
        schema: || ids_schema(json!({})),
        destructive: false,
        open_world: false,
    },
    ToolDef {
        name: "trash",
        description: "Move conversations to Trash. Drafts are left alone. Goes after Pigeon's 5-second undo window.",
        capability: Capability::OrganizeMail,
        schema: || ids_schema(json!({})),
        destructive: true,
        open_world: false,
    },
    ToolDef {
        name: "mark_read",
        description: "Mark conversations read, or unread with read: false.",
        capability: Capability::OrganizeMail,
        schema: || ids_schema(json!({"read": {"type": "boolean", "default": true}})),
        destructive: false,
        open_world: false,
    },
    ToolDef {
        name: "star",
        description: "Star conversations, or unstar them with starred: false.",
        capability: Capability::OrganizeMail,
        schema: || ids_schema(json!({"starred": {"type": "boolean", "default": true}})),
        destructive: false,
        open_world: false,
    },
    ToolDef {
        name: "add_label",
        description: "Put a label on conversations.",
        capability: Capability::OrganizeMail,
        schema: || {
            let mut s = ids_schema(
                json!({"label": {"type": "string", "description": "A label's id or name, from list_labels."}}),
            );
            s["required"] = json!(["ids", "label"]);
            s
        },
        destructive: false,
        open_world: false,
    },
    ToolDef {
        name: "remove_label",
        description: "Take a label off conversations.",
        capability: Capability::OrganizeMail,
        schema: || {
            let mut s = ids_schema(
                json!({"label": {"type": "string", "description": "A label's id or name, from list_labels."}}),
            );
            s["required"] = json!(["ids", "label"]);
            s
        },
        destructive: false,
        open_world: false,
    },
    ToolDef {
        name: "create_draft",
        description: "Save a new message, or a reply, to Drafts, for the user to review and send.",
        capability: Capability::WriteDrafts,
        schema: || message_schema(true),
        destructive: false,
        open_world: false,
    },
    ToolDef {
        name: "send_mail",
        description: "Send a message or reply, or a draft by draft_id, with no confirmation. Goes after Pigeon's 5-second undo window, or when Pigeon is back online.",
        capability: Capability::SendMail,
        schema: || message_schema(false),
        destructive: true,
        open_world: true,
    },
    ToolDef {
        name: "list_events",
        description: "Events from start to end (default: the next 7 days), each repeat on its own, in the calendars the user shows, or in one calendar. Times are local wall-clock time.",
        capability: Capability::ReadCalendar,
        schema: || {
            json!({"type": "object", "properties": {
                "start": {"type": "string", "description": "2026-10-28 or 2026-10-28T09:00."},
                "end": {"type": "string", "description": "Exclusive."},
                "calendar": {"type": "string", "description": "A calendar's id or name."},
            }})
        },
        destructive: false,
        open_world: false,
    },
    ToolDef {
        name: "get_event",
        description: "One event in full: time, calendar, place, notes, guests and their answers, repeat rule, video link.",
        capability: Capability::ReadCalendar,
        schema: || json!({"type": "object", "properties": {"id": {"type": "string", "description": "An event id from list_events."}}, "required": ["id"]}),
        destructive: false,
        open_world: false,
    },
    ToolDef {
        name: "create_event",
        description: "Add an event. Times are local wall-clock time. All-day events take dates, end being the last day (inclusive).",
        capability: Capability::EditCalendar,
        schema: || {
            json!({"type": "object", "properties": {
                "title": {"type": "string"},
                "start": {"type": "string", "description": "2026-10-28T14:00, or a date for an all-day event."},
                "end": {"type": "string", "description": "Defaults to the user's default event length."},
                "all_day": {"type": "boolean"},
                "calendar": {"type": "string", "description": "A calendar's id or name; defaults to the user's default calendar."},
                "location": {"type": "string"},
                "description": {"type": "string"},
                "attendees": {"type": "array", "items": {"type": "string"}, "description": "Guests' addresses."},
                "notify_guests": {"type": "boolean", "description": "Email the guests an invitation. Default: true when there are guests."},
            }, "required": ["title", "start"]})
        },
        destructive: false,
        open_world: true,
    },
    ToolDef {
        name: "update_event",
        description: "Change an event the user can edit. Leave out what stays. A repeating event's title, place and notes change for the whole series; its time can't be changed here.",
        capability: Capability::EditCalendar,
        schema: || {
            json!({"type": "object", "properties": {
                "id": {"type": "string"},
                "title": {"type": "string"},
                "start": {"type": "string"},
                "end": {"type": "string"},
                "location": {"type": "string"},
                "description": {"type": "string"},
                "add_attendees": {"type": "array", "items": {"type": "string"}},
                "remove_attendees": {"type": "array", "items": {"type": "string"}},
                "notify_guests": {"type": "boolean", "description": "Email the guests the change. Default: true when there are guests."},
            }, "required": ["id"]})
        },
        destructive: false,
        open_world: true,
    },
    ToolDef {
        name: "delete_event",
        description: "Delete an event, or one occurrence of a repeating event. Someone else's event is declined and removed. Goes after Pigeon's 5-second undo window.",
        capability: Capability::EditCalendar,
        schema: || {
            json!({"type": "object", "properties": {
                "id": {"type": "string"},
                "occurrence_start": {"type": "string", "description": "For a repeating event: the occurrence's start, from list_events, to delete only it."},
                "notify_guests": {"type": "boolean", "description": "Email the guests. Default: true when there are guests."},
            }, "required": ["id"]})
        },
        destructive: true,
        open_world: true,
    },
    ToolDef {
        name: "respond_to_event",
        description: "Answer an invitation: accepted, tentative or declined.",
        capability: Capability::EditCalendar,
        schema: || {
            json!({"type": "object", "properties": {
                "id": {"type": "string"},
                "response": {"type": "string", "enum": ["accepted", "tentative", "declined"]},
            }, "required": ["id", "response"]})
        },
        destructive: false,
        open_world: true,
    },
];

pub fn find(name: &str) -> Option<&'static ToolDef> {
    TOOLS.iter().find(|t| t.name == name)
}

/// For `tools/list`.
pub fn list() -> Vec<Value> {
    TOOLS
        .iter()
        .map(|t| {
            json!({
                "name": t.name,
                "description": format!(
                    "{} Needs “{}” on in Pigeon's Settings › MCP.",
                    t.description,
                    t.capability.title()
                ),
                "inputSchema": (t.schema)(),
                "annotations": {
                    "readOnlyHint": t.capability.reads(),
                    "destructiveHint": t.destructive,
                    "openWorldHint": t.open_world,
                },
            })
        })
        .collect()
}

type Outcome = Result<Value, String>;

/// A tool's outcome: now, or for a read that waits on Gmail, later. Every
/// write is done by the time `run` returns.
pub enum Run {
    Now(Outcome),
    Later(Task<Outcome>),
}

/// Run tool `name`; its switch was checked already.
pub fn run(name: &str, args: Value, cx: &mut App) -> Run {
    match name {
        "search_mail" => Run::Later(search_mail(args, cx)),
        "read_conversation" => Run::Later(read_conversation(args, cx)),
        _ => Run::Now(match name {
            "list_labels" => list_labels(cx),
            "archive" => archive(&args, cx),
            "trash" => trash(&args, cx),
            "mark_read" => {
                let read = bool_arg(&args, "read", true);
                set_flag(&args, gmail::UNREAD, !read, cx)
            }
            "star" => {
                let starred = bool_arg(&args, "starred", true);
                set_flag(&args, gmail::STARRED, starred, cx)
            }
            "add_label" => label(&args, true, cx),
            "remove_label" => label(&args, false, cx),
            "create_draft" => create_draft(&args, cx),
            "send_mail" => send_mail(&args, cx),
            "list_events" => list_events(&args, cx),
            "get_event" => get_event(&args, cx),
            "create_event" => create_event(&args, cx),
            "update_event" => update_event(&args, cx),
            "delete_event" => delete_event(&args, cx),
            "respond_to_event" => respond_to_event(&args, cx),
            _ => Err(format!("Unknown tool: {name}")),
        }),
    }
}

// Arguments.

fn str_arg<'a>(args: &'a Value, key: &str) -> Option<&'a str> {
    args.get(key).and_then(Value::as_str)
}

fn required<'a>(args: &'a Value, key: &str) -> Result<&'a str, String> {
    str_arg(args, key)
        .filter(|s| !s.trim().is_empty())
        .map(str::trim)
        .ok_or_else(|| format!("{key} is required"))
}

fn bool_arg(args: &Value, key: &str, default: bool) -> bool {
    args.get(key).and_then(Value::as_bool).unwrap_or(default)
}

/// A list of strings, or one comma-separated string.
fn strings(args: &Value, key: &str) -> Vec<String> {
    match args.get(key) {
        Some(Value::Array(items)) => items
            .iter()
            .filter_map(Value::as_str)
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect(),
        Some(Value::String(s)) => s
            .split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect(),
        _ => Vec::new(),
    }
}

fn contacts(args: &Value, key: &str) -> Result<Vec<Contact>, String> {
    strings(args, key)
        .iter()
        .map(|s| parse_address(s).ok_or_else(|| format!("“{s}” isn't an email address")))
        .collect()
}

/// The years a date may be in: far enough from chrono's limits that adding
/// a week or an event's length can't overflow.
const YEARS: std::ops::RangeInclusive<i32> = 1900..=2200;

/// A date (all day) or a local date and time; a time with an offset is
/// moved to this Mac's zone.
fn parse_when(text: &str) -> Result<(NaiveDateTime, bool), String> {
    let t = text.trim();
    let refused = || format!("“{t}” isn't a date or time; use 2026-10-28 or 2026-10-28T14:30");
    let in_range = |(at, date_only): (NaiveDateTime, bool)| {
        YEARS
            .contains(&at.year())
            .then_some((at, date_only))
            .ok_or_else(refused)
    };
    if let Ok(at) = DateTime::parse_from_rfc3339(t) {
        return in_range((at.with_timezone(&Local).naive_local(), false));
    }
    for format in [
        "%Y-%m-%dT%H:%M:%S",
        "%Y-%m-%dT%H:%M",
        "%Y-%m-%d %H:%M:%S",
        "%Y-%m-%d %H:%M",
    ] {
        if let Ok(at) = NaiveDateTime::parse_from_str(t, format) {
            return in_range((at, false));
        }
    }
    NaiveDate::parse_from_str(t, "%Y-%m-%d")
        .map_err(|_| refused())
        .and_then(|d| in_range((midnight(d), true)))
}

fn time_text(at: NaiveDateTime) -> String {
    at.format("%Y-%m-%dT%H:%M").to_string()
}

// Mail.

fn backend(cx: &App) -> Entity<Backend> {
    shell::backend_of(cx)
}

fn person(c: &Contact) -> String {
    if c.name.is_empty() {
        c.email.clone()
    } else {
        format!("{} <{}>", c.name, c.email)
    }
}

fn people(list: &[Contact]) -> Vec<String> {
    list.iter().map(person).collect()
}

fn label_names(thread: &Thread, data: &MailData) -> Vec<String> {
    thread
        .labels
        .iter()
        .map(|id| {
            data.label(id)
                .map_or_else(|| id.clone(), |l| l.name.clone())
        })
        .collect()
}

fn summary(thread: &Thread, data: &MailData) -> Value {
    let last = thread.last_message();
    json!({
        "id": thread.id,
        "subject": thread.subject,
        "from": person(&last.from),
        "date": last.date.to_rfc3339(),
        "snippet": last.snippet,
        "messages": thread.messages.len(),
        "folder": thread.folder.key(),
        "labels": label_names(thread, data),
        "unread": thread.unread,
        "starred": thread.starred,
        "has_attachments": thread.has_attachments(),
    })
}

/// The threads `ids` names in `data`, newest first, at most `limit`.
fn summaries(data: &MailData, ids: &[String], limit: usize, note: Option<String>) -> Value {
    let mut threads: Vec<&Thread> = data
        .threads
        .iter()
        .filter(|t| ids.contains(&t.id))
        .collect();
    threads.sort_by_key(|t| std::cmp::Reverse(t.last_message().internal_date));
    let total = threads.len();
    let results: Vec<Value> = threads
        .into_iter()
        .take(limit)
        .map(|t| summary(t, data))
        .collect();
    let mut out = json!({"results": results, "more": total > limit});
    if let Some(note) = note {
        out["note"] = note.into();
    }
    out
}

fn search_mail(args: Value, cx: &mut App) -> Task<Outcome> {
    let text = str_arg(&args, "query").unwrap_or("").trim().to_string();
    let limit = args
        .get("limit")
        .and_then(Value::as_u64)
        .unwrap_or(20)
        .clamp(1, 100) as usize;
    let backend = backend(cx);
    let query = Query::parse(&text);
    let data = backend.read(cx).load_mail_heads();
    let scope = query.scope.unwrap_or(Scope::All);
    let local: Vec<String> = if query.gmail_only() {
        Vec::new()
    } else {
        data.threads
            .iter()
            .filter(|t| Query::in_scope(scope, t) && query.matches(t, &data))
            .map(|t| t.id.clone())
            .collect()
    };
    let gmail_query = (!text.is_empty() && backend.read(cx).can_search())
        .then(|| query.to_gmail_q(&data.labels, None))
        .flatten();
    let Some(gmail_query) = gmail_query else {
        let note = query
            .gmail_only()
            .then(|| "This query needs Gmail's own search, which isn't available now.".into());
        return Task::ready(Ok(summaries(&data, &local, limit, note)));
    };
    let search = backend.update(cx, |b, cx| b.search(&gmail_query, None, cx));
    cx.spawn(async move |cx| {
        let found = search.await;
        cx.update(|cx| {
            let (mut ids, note) = match found {
                Ok(found) => (found.thread_ids, None),
                Err(e) => (
                    Vec::new(),
                    Some(format!(
                        "Gmail's search failed ({e}); these are from Pigeon's cache only."
                    )),
                ),
            };
            for id in local {
                if !ids.contains(&id) {
                    ids.push(id);
                }
            }
            // Gmail's results are stored now.
            let data = backend.read(cx).load_mail_heads();
            Ok(summaries(&data, &ids, limit, note))
        })
    })
}

/// A message's text, at most this many characters.
const TEXT_MAX: usize = 20_000;

fn conversation(thread: &Thread, data: &MailData) -> Value {
    let messages: Vec<Value> = thread
        .messages
        .iter()
        .map(|m| {
            let text = match &m.body {
                Body::Loaded { html } => {
                    let text = crate::html::plain_text(&crate::html::parse(html));
                    let text = text.trim();
                    if text.chars().count() > TEXT_MAX {
                        let cut: String = text.chars().take(TEXT_MAX).collect();
                        format!("{cut}\n[…cut]")
                    } else {
                        text.to_string()
                    }
                }
                Body::Failed(why) => format!("(Couldn't download this message: {why})"),
                Body::NotLoaded | Body::Loading => "(Not downloaded yet; try again.)".into(),
            };
            let mut out = json!({
                "id": m.id,
                "from": person(&m.from),
                "to": people(&m.to),
                "date": m.date.to_rfc3339(),
                "text": text,
                "attachments": m.attachments.iter().map(|a| json!({
                    "name": a.name, "mime": a.mime, "size_bytes": a.size_bytes,
                })).collect::<Vec<_>>(),
            });
            if !m.cc.is_empty() {
                out["cc"] = people(&m.cc).into();
            }
            if !m.bcc.is_empty() {
                out["bcc"] = people(&m.bcc).into();
            }
            out
        })
        .collect();
    json!({
        "id": thread.id,
        "subject": thread.subject,
        "folder": thread.folder.key(),
        "labels": label_names(thread, data),
        "unread": thread.unread,
        "starred": thread.starred,
        "messages": messages,
    })
}

fn bodies_missing(thread: &Thread) -> bool {
    thread
        .messages
        .iter()
        .any(|m| matches!(m.body, Body::NotLoaded | Body::Loading))
}

fn read_conversation(args: Value, cx: &mut App) -> Task<Outcome> {
    let id = match required(&args, "id") {
        Ok(id) => resolve(id, cx),
        Err(e) => return Task::ready(Err(e)),
    };
    let backend = backend(cx);
    let fetch = id.clone();
    let reader = backend.clone();
    let read = move |cx: &App| -> Result<(Value, bool), String> {
        let b = reader.read(cx);
        let thread = b
            .load_threads(std::slice::from_ref(&id))
            .pop()
            .ok_or_else(|| format!("No conversation with id {id}"))?;
        let data = MailData {
            account: Default::default(),
            labels: b.load_labels(),
            threads: Vec::new(),
        };
        Ok((conversation(&thread, &data), bodies_missing(&thread)))
    };
    let (value, missing) = match read(cx) {
        Ok(found) => found,
        Err(e) => return Task::ready(Err(e)),
    };
    if !missing || !backend.read(cx).can_search() {
        return Task::ready(Ok(value));
    }
    backend.update(cx, |b, cx| b.fetch_body(&fetch, cx));
    cx.spawn(async move |cx| {
        // The bodies, once the backend stored them; what's there after 15 s.
        for _ in 0..60 {
            cx.background_executor()
                .timer(std::time::Duration::from_millis(250))
                .await;
            let (value, missing) = cx.update(|cx| read(cx))?;
            if !missing {
                return Ok(value);
            }
        }
        cx.update(|cx| read(cx)).map(|(value, _)| value)
    })
}

fn list_labels(cx: &App) -> Outcome {
    let labels: Vec<Value> = backend(cx)
        .read(cx)
        .load_labels()
        .iter()
        .map(|l| json!({"id": l.id, "name": l.name}))
        .collect();
    Ok(json!({"labels": labels}))
}

/// The conversations `ids` names, all of them, or which one isn't there.
fn threads(args: &Value, cx: &App) -> Result<Vec<Thread>, String> {
    let ids: Vec<String> = strings(args, "ids")
        .iter()
        .map(|id| resolve(id, cx))
        .collect();
    if ids.is_empty() {
        return Err("ids is required".into());
    }
    let threads = backend(cx).read(cx).load_threads(&ids);
    if let Some(id) = ids.iter().find(|id| !threads.iter().any(|t| &t.id == *id)) {
        return Err(format!("No conversation with id {id}"));
    }
    Ok(threads)
}

/// Queue `ops`, as the menu bar panel does, and have the windows read the
/// threads back with the change shown.
fn queue_mail(ops: Vec<MailOp>, ids: Vec<String>, cx: &mut App) {
    backend(cx).update(cx, |b, cx| {
        for op in ops {
            b.mail(op, cx);
        }
        cx.emit(BackendEvent::ThreadsChanged(ids));
    });
}

fn ids_of(threads: &[&Thread]) -> Vec<String> {
    threads.iter().map(|t| t.id.clone()).collect()
}

fn skipped(threads: &[&Thread], why: &str) -> Vec<Value> {
    threads
        .iter()
        .map(|t| json!({"id": t.id, "reason": why}))
        .collect()
}

fn archive(args: &Value, cx: &mut App) -> Outcome {
    let threads = threads(args, cx)?;
    let (inbox, rest): (Vec<&Thread>, Vec<&Thread>) =
        threads.iter().partition(|t| t.folder == Folder::Inbox);
    let ids = ids_of(&inbox);
    if !ids.is_empty() {
        let ops = MailOp::moves(ids.clone(), Folder::Inbox, Folder::Archive);
        queue_mail(ops, ids.clone(), cx);
    }
    Ok(json!({"archived": ids, "skipped": skipped(&rest, "not in the Inbox")}))
}

fn trash(args: &Value, cx: &mut App) -> Outcome {
    let threads = threads(args, cx)?;
    let (go, rest): (Vec<&Thread>, Vec<&Thread>) = threads
        .iter()
        .partition(|t| !matches!(t.folder, Folder::Drafts | Folder::Trash));
    let mut ops = Vec::new();
    for from in Folder::ALL {
        let ids = ids_of(
            &go.iter()
                .copied()
                .filter(|t| t.folder == from)
                .collect::<Vec<_>>(),
        );
        if !ids.is_empty() {
            ops.extend(MailOp::moves(ids, from, Folder::Trash));
        }
    }
    let ids = ids_of(&go);
    if !ops.is_empty() {
        queue_mail(ops, ids.clone(), cx);
    }
    Ok(json!({"trashed": ids, "skipped": skipped(&rest, "a draft, or in Trash already")}))
}

fn modify(threads: &[Thread], label: &str, on: bool, cx: &mut App) -> Vec<String> {
    let ids: Vec<String> = threads.iter().map(|t| t.id.clone()).collect();
    let (add, remove) = if on {
        (vec![label.to_string()], Vec::new())
    } else {
        (Vec::new(), vec![label.to_string()])
    };
    let op = MailOp::Modify {
        threads: ids.clone(),
        add,
        remove,
    };
    queue_mail(vec![op], ids.clone(), cx);
    ids
}

fn set_flag(args: &Value, label: &str, on: bool, cx: &mut App) -> Outcome {
    let threads = threads(args, cx)?;
    let ids = modify(&threads, label, on, cx);
    Ok(json!({"changed": ids}))
}

fn label(args: &Value, on: bool, cx: &mut App) -> Outcome {
    let name = required(args, "label")?;
    let labels = backend(cx).read(cx).load_labels();
    let label = labels
        .iter()
        .find(|l| l.id == name)
        .or_else(|| labels.iter().find(|l| l.name.eq_ignore_ascii_case(name)))
        .ok_or_else(|| format!("No label “{name}”; list_labels gives their names"))?;
    let threads = threads(args, cx)?;
    let ids = modify(&threads, &label.id, on, cx);
    Ok(json!({"changed": ids, "label": label.name}))
}

/// "Re: " once.
fn re(subject: &str) -> String {
    if subject
        .get(..3)
        .is_some_and(|p| p.eq_ignore_ascii_case("re:"))
    {
        subject.to_string()
    } else {
        format!("Re: {subject}")
    }
}

/// The message the arguments describe, from the user's default address
/// (or, for a reply, the one the mail was sent to), with their signature
/// when Settings › Mail › Signatures is on.
fn compose(args: &Value, cx: &App) -> Result<Outgoing, String> {
    let b = backend(cx).read(cx);
    let account = b.account().cloned().unwrap_or_default();
    if account.email.is_empty() {
        return Err("Nobody is signed in to Pigeon".into());
    }
    let mut out = Outgoing {
        to: contacts(args, "to")?,
        cc: contacts(args, "cc")?,
        bcc: contacts(args, "bcc")?,
        subject: str_arg(args, "subject").unwrap_or("").trim().to_string(),
        markdown: str_arg(args, "body").unwrap_or("").to_string(),
        ..Default::default()
    };
    let mut from = String::new();
    if let Some(answers) = str_arg(args, "reply_to_message_id").filter(|s| !s.is_empty()) {
        let data = b.load_mail_heads();
        let (thread, message) = data
            .threads
            .iter()
            .find_map(|t| t.messages.iter().find(|m| m.id == answers).map(|m| (t, m)))
            .ok_or_else(|| format!("No message with id {answers}"))?;
        out.kind = ComposeKind::Reply;
        out.reply_to = Some(thread.id.clone());
        out.answers = message.id.clone();
        if out.subject.is_empty() {
            out.subject = re(&thread.subject);
        }
        let mine = account.is_me(&message.from.email);
        if out.to.is_empty() {
            out.to = if mine {
                message.to.clone()
            } else if !message.reply_to.is_empty() {
                message.reply_to.clone()
            } else {
                vec![message.from.clone()]
            };
        }
        from = if mine {
            message.from.email.clone()
        } else {
            message
                .to
                .iter()
                .chain(&message.cc)
                .find(|c| account.is_me(&c.email))
                .map(|c| c.email.clone())
                .unwrap_or_default()
        };
    }
    let identity = account.identity(&from);
    out.from = account.sender(&identity);
    out.reply_to_address = identity.reply_to.clone();
    let mut html = crate::markdown::to_html(&out.markdown);
    if settings::get(cx).signatures && !identity.signature.trim().is_empty() {
        html.push_str(&mime::signature_block(&identity.signature));
        out.signature = identity.signature.clone();
    }
    out.html = html;
    Ok(out)
}

fn create_draft(args: &Value, cx: &mut App) -> Outcome {
    let draft = compose(args, cx)?;
    let id = format!("draft-local-{}", uuid::Uuid::new_v4().simple());
    let saved = backend(cx).update(cx, |b, cx| {
        let saved = b.save_draft(id.clone(), draft, 1, 0, cx);
        if saved {
            cx.emit(BackendEvent::ThreadsChanged(vec![id.clone()]));
        }
        saved
    });
    if !saved {
        return Err("Couldn't save the draft".into());
    }
    Ok(json!({"draft_id": id, "note": "Saved to Drafts."}))
}

/// A draft from Drafts, as it would be sent.
fn stored_draft(id: &str, cx: &App) -> Result<Outgoing, String> {
    let open = shell::main_window(cx)
        .and_then(|m| m.app.upgrade())
        .is_some_and(|app| app.read(cx).draft_is_open(id, cx));
    if open {
        return Err("That draft is open in Pigeon; send it there, or close it first".into());
    }
    let thread = backend(cx)
        .read(cx)
        .load_threads(&[id.to_string()])
        .pop()
        .filter(|t| t.folder == Folder::Drafts)
        .ok_or_else(|| format!("No draft with id {id}"))?;
    if bodies_missing(&thread) {
        return Err(
            "The draft hasn't downloaded yet; read it with read_conversation, then try again"
                .into(),
        );
    }
    let m = thread.last_message();
    let origin = thread.origin.clone();
    let kind = origin.as_ref().map_or(ComposeKind::New, |o| o.kind);
    // `html` has the signature already; the plain-text part takes it from here.
    let signature = origin
        .as_ref()
        .filter(|o| o.signature)
        .and_then(|_| backend(cx).read(cx).account().cloned())
        .map(|account| account.identity(&m.from.email).signature)
        .unwrap_or_default();
    let mut html = m.html().to_string();
    if let Some(quote) = origin.as_ref().and_then(|o| o.quoted.clone()) {
        let quoted = crate::ui::composer::Quoted::new(quote.header, quote.html);
        html.push_str(&crate::ui::composer::quote_html(kind, &quoted));
    }
    Ok(Outgoing {
        kind,
        from: m.from.clone(),
        to: m.to.clone(),
        cc: m.cc.clone(),
        bcc: m.bcc.clone(),
        subject: thread.subject.clone(),
        markdown: origin
            .as_ref()
            .map(|o| o.markdown.clone())
            .unwrap_or_else(|| crate::html::plain_text(&crate::html::parse(m.html()))),
        html,
        attachments: m.attachments.clone(),
        reply_to: origin.as_ref().and_then(|o| o.reply_to.clone()),
        answers: origin
            .as_ref()
            .map(|o| o.answers.clone())
            .unwrap_or_default(),
        quoted: origin.and_then(|o| o.quoted),
        reply_to_address: m
            .reply_to
            .first()
            .map(|c| c.email.clone())
            .unwrap_or_default(),
        signature,
        ..Default::default()
    })
}

fn send_mail(args: &Value, cx: &mut App) -> Outcome {
    let draft_id = str_arg(args, "draft_id")
        .filter(|s| !s.trim().is_empty())
        .map(|id| resolve(id.trim(), cx));
    let draft = match &draft_id {
        Some(id) => stored_draft(id, cx)?,
        None => compose(args, cx)?,
    };
    if draft.to.is_empty() && draft.cc.is_empty() && draft.bcc.is_empty() {
        return Err("Add at least one recipient".into());
    }
    let backend = backend(cx);
    let thread_id = draft
        .reply_to
        .clone()
        .filter(|id| {
            !backend
                .read(cx)
                .load_threads(std::slice::from_ref(id))
                .is_empty()
        })
        .unwrap_or_else(|| format!("sent-local-{}", uuid::Uuid::new_v4().simple()));
    let op = backend.update(cx, |b, cx| {
        let op = b.mail(
            MailOp::Send {
                draft,
                local_id: draft_id.clone(),
                thread_id: thread_id.clone(),
            },
            cx,
        );
        if let Some(id) = &draft_id {
            cx.emit(BackendEvent::ThreadsChanged(vec![id.clone()]));
        }
        op
    });
    super::track_send(op, thread_id.clone(), cx);
    Ok(json!({
        "status": "queued",
        "conversation_id": thread_id,
        "note": "Sends after Pigeon's 5-second undo window, or once Pigeon is back online.",
    }))
}

// Calendar.

/// The calendar view, while the main window is open: it holds edits the
/// store doesn't have yet.
fn calendar_view(cx: &App) -> Option<Entity<CalendarView>> {
    let app = shell::main_window(cx)?.app.upgrade()?;
    Some(app.read(cx).calendar.clone())
}

/// `f` on the calendar as the user sees it.
fn with_calendar<R>(cx: &App, f: impl FnOnce(&CalendarData) -> R) -> R {
    match calendar_view(cx) {
        Some(view) => f(&view.read(cx).data),
        None => f(&backend(cx).read(cx).load_calendar()),
    }
}

/// An agent may not change an event the user is changing in Pigeon: one
/// of the two changes would be lost.
fn not_editing(id: &str, cx: &App) -> Result<(), String> {
    match calendar_view(cx) {
        Some(view) if view.read(cx).user_is_editing(id) => Err(
            "The user is editing this event in Pigeon right now; try again once they're done"
                .into(),
        ),
        _ => Ok(()),
    }
}

/// What a calendar write says when no window shows the calendar.
const UNSEEN: &str = "Pigeon's window is closed, so list_events and get_event show this once Pigeon has saved it (about a second online; offline, once it's back).";

/// Make the change in the open calendar, as its own edits are, and queue
/// `ops`; with no window open, just queue them. `id`, changed or gone
/// (wholly or at `removed`), then shows in the open panel as after a
/// sync. Whether a window showed the change.
fn change_calendar(
    cx: &mut App,
    id: Option<&str>,
    removed: Option<NaiveDateTime>,
    change: impl FnOnce(&mut Vec<Event>),
    ops: Vec<CalOp>,
) -> bool {
    let Some(view) = calendar_view(cx) else {
        backend(cx).update(cx, |b, cx| {
            for op in ops {
                b.calendar(op, cx);
            }
        });
        return false;
    };
    view.update(cx, |c, cx| {
        change(&mut c.data.events);
        for op in ops {
            c.queue(op, cx);
        }
        cx.notify();
    });
    if let (Some(id), Some(main)) = (id, shell::main_window(cx)) {
        main.update(cx, |app, window, cx| {
            app.calendar
                .update(cx, |c, cx| c.changed_elsewhere(id, removed, window, cx))
        });
    }
    true
}

/// `out`, saying when reads will show the change if no window showed it.
fn noted(mut out: Value, shown: bool) -> Value {
    if !shown {
        out["note"] = UNSEEN.into();
    }
    out
}

fn find_calendar<'a>(
    data: &'a CalendarData,
    name: &str,
) -> Result<&'a crate::calendar::Calendar, String> {
    data.calendar(name)
        .or_else(|| {
            data.calendars
                .iter()
                .find(|c| c.name.eq_ignore_ascii_case(name.trim()))
        })
        .ok_or_else(|| format!("No calendar “{name}”"))
}

fn find_event<'a>(data: &'a CalendarData, id: &str) -> Result<&'a Event, String> {
    data.event_ix(id)
        .map(|ix| &data.events[ix])
        .ok_or_else(|| format!("No event with id {id}"))
}

fn event_access(data: &CalendarData, event: &Event) -> (Access, Option<String>) {
    let calendar = data.calendar(&event.calendar);
    let role = calendar.map(|c| c.access_role).unwrap_or_default();
    let access = access(event, role, &data.account.email);
    (
        access,
        refusal(access, role, calendar.map_or("", |c| c.name.as_str())),
    )
}

fn when(event: &Event, start: NaiveDateTime, end: NaiveDateTime) -> Value {
    if event.all_day {
        json!({
            "start": start.date().to_string(),
            "end": (end - Duration::days(1)).date().max(start.date()).to_string(),
            "all_day": true,
        })
    } else {
        json!({"start": time_text(start), "end": time_text(end), "all_day": false})
    }
}

fn calendar_name(data: &CalendarData, event: &Event) -> String {
    data.calendar(&event.calendar)
        .map_or_else(|| event.calendar.clone(), |c| c.name.clone())
}

fn list_events(args: &Value, cx: &App) -> Outcome {
    let from = match str_arg(args, "start") {
        Some(s) => parse_when(s)?.0,
        None => midnight(Local::now().date_naive()),
    };
    let to = match str_arg(args, "end") {
        Some(s) => parse_when(s)?.0,
        None => from + Duration::days(7),
    };
    if to <= from {
        return Err("end must be after start".into());
    }
    if to - from > Duration::days(366) {
        return Err("Ask for a year or less at a time".into());
    }
    with_calendar(cx, |data| {
        let calendar = str_arg(args, "calendar")
            .map(|c| find_calendar(data, c).map(|c| c.id.clone()))
            .transpose()?;
        let include = |e: &Event| {
            e.status != EventStatus::Cancelled
                && match &calendar {
                    Some(id) => &e.calendar == id,
                    None => data.is_visible(e),
                }
        };
        let found = occurrences(&data.events, include, from, to);
        const MAX: usize = 200;
        let events: Vec<Value> = found
            .iter()
            .take(MAX)
            .map(|o| {
                let e = &data.events[o.event];
                let mut out = json!({
                    "id": e.id,
                    "title": e.display_title(),
                    "calendar": calendar_name(data, e),
                    "repeats": e.is_series(),
                });
                if let (Value::Object(out), Value::Object(when)) =
                    (&mut out, when(e, o.start, o.end))
                {
                    out.extend(when);
                }
                if e.is_series() {
                    out["occurrence_start"] = time_text(o.start).into();
                }
                if !e.location.is_empty() {
                    out["location"] = e.location.clone().into();
                }
                if let Some(status) = e.my_status(&data.account) {
                    out["my_response"] = json!(status);
                }
                out
            })
            .collect();
        Ok(json!({
            "time_zone": crate::calendar::time::LocalZone::system().name,
            "events": events,
            "more": found.len() > MAX,
        }))
    })
}

fn get_event(args: &Value, cx: &App) -> Outcome {
    let id = required(args, "id")?;
    with_calendar(cx, |data| {
        let e = find_event(data, id)?;
        let (access, _) = event_access(data, e);
        let mut out = json!({
            "id": e.id,
            "title": e.display_title(),
            "calendar": {"id": e.calendar, "name": calendar_name(data, e)},
            "location": e.location,
            "description": e.description,
            "organizer": person(&e.organizer),
            "attendees": e.attendees.iter().map(|a| json!({
                "name": a.name, "email": a.email, "response": a.status,
            })).collect::<Vec<_>>(),
            "can_edit": access == Access::Edit,
            "busy": e.busy,
        });
        if let (Value::Object(out), Value::Object(when)) = (&mut out, when(e, e.start, e.end)) {
            out.extend(when);
        }
        if let Some(rule) = &e.recurrence {
            out["repeats"] = rule.label().into();
        }
        let video = [&e.conference_uri, &e.video_link]
            .into_iter()
            .find(|l| !l.is_empty());
        if let Some(video) = video {
            out["video_link"] = video.clone().into();
        }
        if !e.html_link.is_empty() {
            out["link"] = e.html_link.clone().into();
        }
        if let Some(status) = e.my_status(&data.account) {
            out["my_response"] = json!(status);
        }
        Ok(out)
    })
}

/// Email the guests: as asked, else when there are any.
fn send_updates(args: &Value, guests: bool) -> SendUpdates {
    if bool_arg(args, "notify_guests", guests) {
        SendUpdates::All
    } else {
        SendUpdates::None
    }
}

fn attendees(list: Vec<Contact>) -> Vec<Attendee> {
    list.into_iter()
        .map(|c| Attendee {
            name: c.name,
            email: c.email,
            status: RsvpStatus::NeedsAction,
        })
        .collect()
}

/// The event's new times: all-day ones take dates, the end the last day.
fn times(
    all_day: bool,
    start: NaiveDateTime,
    end: Option<&str>,
    length: Duration,
) -> Result<(NaiveDateTime, NaiveDateTime), String> {
    if all_day {
        let first = start.date();
        let last = match end {
            Some(end) => parse_when(end)?.0.date(),
            None => first,
        };
        if last < first {
            return Err("end must not be before start".into());
        }
        return Ok((midnight(first), midnight(last + Duration::days(1))));
    }
    let end = match end {
        Some(end) => parse_when(end)?.0,
        None => start + length,
    };
    if end <= start {
        return Err("end must be after start".into());
    }
    Ok((start, end))
}

fn create_event(args: &Value, cx: &mut App) -> Outcome {
    let title = required(args, "title")?.to_string();
    let (start, date_only) = parse_when(required(args, "start")?)?;
    let all_day = bool_arg(args, "all_day", date_only);
    let minutes = calendar_view(cx)
        .map(|v| v.read(cx).event_minutes())
        .or(settings::get(cx).event_minutes)
        .unwrap_or(60);
    let (start, end) = times(
        all_day,
        start,
        str_arg(args, "end"),
        Duration::minutes(minutes as i64),
    )?;
    let guests = attendees(contacts(args, "attendees")?);
    let default = settings::get(cx).default_calendar.clone();
    let event = with_calendar(cx, |data| -> Result<Event, String> {
        let calendar = match str_arg(args, "calendar") {
            Some(name) => {
                let c = find_calendar(data, name)?;
                if !c.writable() {
                    return Err(format!("“{}” is read-only", c.name));
                }
                c
            }
            None => crate::calendar::new_event_calendar(&data.calendars, default.as_deref())
                .filter(|c| c.writable())
                .ok_or("There's no calendar to add events to")?,
        };
        Ok(Event {
            id: uuid::Uuid::new_v4().simple().to_string(),
            calendar: calendar.id.clone(),
            title,
            start,
            end,
            all_day,
            location: str_arg(args, "location").unwrap_or("").to_string(),
            description: str_arg(args, "description").unwrap_or("").to_string(),
            attendees: guests,
            organizer: data.me(),
            organizer_self: true,
            reminder_minutes: calendar.default_reminder,
            reminder_use_default: true,
            busy: true,
            ..Event::default()
        })
    })?;
    let send = send_updates(args, !event.attendees.is_empty());
    let out = json!({"id": event.id, "calendar": event.calendar});
    let op = CalOp::Insert {
        event: event.clone(),
        send_updates: send,
    };
    let shown = change_calendar(cx, None, None, |events| events.push(event), vec![op]);
    Ok(noted(out, shown))
}

fn update_event(args: &Value, cx: &mut App) -> Outcome {
    let id = required(args, "id")?.to_string();
    not_editing(&id, cx)?;
    let add = attendees(contacts(args, "add_attendees")?);
    let remove = contacts(args, "remove_attendees")?;
    let (before, after, me) = with_calendar(cx, |data| -> Result<_, String> {
        let e = find_event(data, &id)?;
        if let (access, Some(why)) = event_access(data, e)
            && access != Access::Edit
        {
            return Err(why);
        }
        let mut after = e.clone();
        if let Some(title) = str_arg(args, "title") {
            after.title = title.to_string();
        }
        if let Some(location) = str_arg(args, "location") {
            after.location = location.to_string();
        }
        if let Some(description) = str_arg(args, "description") {
            after.description = description.to_string();
        }
        let start = str_arg(args, "start");
        let end = str_arg(args, "end");
        if start.is_some() || end.is_some() {
            if e.is_series() {
                return Err(
                    "A repeating event's time can't be changed over MCP; change it in Pigeon"
                        .into(),
                );
            }
            let new_start = match start {
                Some(s) => parse_when(s)?.0,
                None => e.start,
            };
            let end = end.map(str::to_string).or_else(|| {
                // Keep the length; an all-day event's last day.
                let end = new_start + e.duration();
                Some(if e.all_day {
                    (end - Duration::days(1)).date().to_string()
                } else {
                    time_text(end)
                })
            });
            (after.start, after.end) = times(e.all_day, new_start, end.as_deref(), e.duration())?;
        }
        for guest in add {
            if !after
                .attendees
                .iter()
                .any(|a| a.email.eq_ignore_ascii_case(&guest.email))
            {
                after.attendees.push(guest);
            }
        }
        after.attendees.retain(|a| {
            !remove
                .iter()
                .any(|r| r.email.eq_ignore_ascii_case(&a.email))
        });
        Ok((e.clone(), after, data.account.email.clone()))
    })?;
    if before == after {
        return Ok(json!({"id": id, "changed": false}));
    }
    let guests = has_guests(&after, &me) || has_guests(&before, &me);
    let op = CalOp::Patch {
        id: id.clone(),
        before,
        after: after.clone(),
        scope: SeriesScope::All,
        send_updates: send_updates(args, guests),
    };
    let shown = change_calendar(
        cx,
        Some(&id),
        None,
        |events| {
            if let Some(e) = events.iter_mut().find(|e| e.id == id) {
                *e = after;
            }
        },
        vec![op],
    );
    Ok(noted(json!({"id": id, "changed": true}), shown))
}

fn delete_event(args: &Value, cx: &mut App) -> Outcome {
    let id = required(args, "id")?.to_string();
    not_editing(&id, cx)?;
    let asked = str_arg(args, "occurrence_start")
        .map(|s| parse_when(s).map(|w| w.0))
        .transpose()?;
    let (occurrence, scope, guests, title) = with_calendar(cx, |data| -> Result<_, String> {
        let e = find_event(data, &id)?;
        if let (Access::ReadOnly, Some(why)) = event_access(data, e) {
            return Err(why);
        }
        let me = &data.account.email;
        let (occurrence, scope) = match asked {
            Some(at) if e.is_series() => {
                let found = occurrences(
                    std::slice::from_ref(e),
                    |_| true,
                    at,
                    at + Duration::minutes(1),
                );
                if !found.iter().any(|o| o.start == at) {
                    return Err(format!(
                        "“{}” has no occurrence starting {}",
                        e.display_title(),
                        time_text(at)
                    ));
                }
                (at, SeriesScope::This)
            }
            _ => (e.start, SeriesScope::All),
        };
        let guests = is_organizer(e, me) && has_guests(e, me);
        Ok((occurrence, scope, guests, e.display_title().to_string()))
    })?;
    let op = CalOp::Delete {
        id: id.clone(),
        occurrence: Some(occurrence),
        scope,
        send_updates: send_updates(args, guests),
    };
    let shown = change_calendar(
        cx,
        Some(&id),
        Some(occurrence),
        |events| match scope {
            SeriesScope::All => events
                .retain(|e| e.id != id && e.recurring_event_id.as_deref() != Some(id.as_str())),
            _ => {
                if let Some(series) = events.iter_mut().find(|e| e.id == id) {
                    series.exceptions.push(occurrence);
                }
            }
        },
        vec![op],
    );
    Ok(noted(
        json!({"deleted": title, "occurrence_only": scope == SeriesScope::This}),
        shown,
    ))
}

fn respond_to_event(args: &Value, cx: &mut App) -> Outcome {
    let id = required(args, "id")?.to_string();
    let status = match required(args, "response")?.to_ascii_lowercase().as_str() {
        "accepted" | "accept" | "yes" => RsvpStatus::Accepted,
        "tentative" | "maybe" => RsvpStatus::Tentative,
        "declined" | "decline" | "no" => RsvpStatus::Declined,
        other => return Err(format!("“{other}” isn't accepted, tentative or declined")),
    };
    not_editing(&id, cx)?;
    let account = with_calendar(cx, |data| -> Result<_, String> {
        let e = find_event(data, &id)?;
        if !e.attendees.iter().any(|a| data.account.is_me(&a.email)) {
            return Err(format!("You aren't a guest of “{}”", e.display_title()));
        }
        Ok(data.account.clone())
    })?;
    let shown = change_calendar(
        cx,
        Some(&id),
        None,
        |events| {
            if let Some(e) = events.iter_mut().find(|e| e.id == id) {
                for a in &mut e.attendees {
                    if account.is_me(&a.email) {
                        a.status = status;
                    }
                }
            }
        },
        vec![CalOp::Rsvp {
            id: id.clone(),
            status,
        }],
    );
    Ok(noted(json!({"id": id, "response": status}), shown))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_tool_has_an_object_schema_and_a_unique_name() {
        for tool in TOOLS {
            assert_eq!((tool.schema)()["type"], "object", "{}", tool.name);
            assert_eq!(TOOLS.iter().filter(|t| t.name == tool.name).count(), 1);
        }
        assert!(find("send_mail").is_some_and(|t| t.capability == Capability::SendMail));
    }

    #[test]
    fn dates_and_times() {
        let (at, date_only) = parse_when("2026-10-28").unwrap();
        assert!(date_only);
        assert_eq!(time_text(at), "2026-10-28T00:00");
        let (at, date_only) = parse_when("2026-10-28T14:30").unwrap();
        assert!(!date_only);
        assert_eq!(time_text(at), "2026-10-28T14:30");
        assert!(parse_when("tomorrow").is_err());
        let start = parse_when("2026-10-28").unwrap().0;
        let (s, e) = times(true, start, Some("2026-10-29"), Duration::hours(1)).unwrap();
        assert_eq!(
            (time_text(s), time_text(e)),
            ("2026-10-28T00:00".into(), "2026-10-30T00:00".into())
        );
        let (_, e) = times(false, start, None, Duration::minutes(30)).unwrap();
        assert_eq!(time_text(e), "2026-10-28T00:30");
        assert!(times(false, start, Some("2026-10-27T10:00"), Duration::zero()).is_err());
    }

    #[test]
    fn dates_far_out_are_refused_not_panicked_on() {
        for text in [
            "+262142-12-31",
            "+262142-12-31T23:59",
            "-262143-01-01",
            "9999-12-31",
            "1899-12-31",
            "2201-01-01T00:00",
            "+262142-12-31T23:59:59Z",
        ] {
            assert!(parse_when(text).is_err(), "{text}");
        }
        assert!(parse_when("1900-01-01").is_ok());
        let (last, _) = parse_when("2200-12-31T23:59").unwrap();
        // What the tools add to a date can't overflow.
        let (s, e) = times(true, last, None, Duration::days(366)).unwrap();
        assert!(e > s);
        assert!(times(false, last, None, Duration::days(366)).is_ok());
    }

    #[test]
    fn addresses_and_lists() {
        let args =
            json!({"to": ["Ana <ana@x.com>", "bo@y.org"], "cc": "c@z.io, d@z.io", "bcc": ["nope"]});
        let to = contacts(&args, "to").unwrap();
        assert_eq!(to[0].name, "Ana");
        assert_eq!(to[1].email, "bo@y.org");
        assert_eq!(contacts(&args, "cc").unwrap().len(), 2);
        assert!(contacts(&args, "bcc").is_err());
        assert_eq!(re("Re: lunch"), "Re: lunch");
        assert_eq!(re("lunch"), "Re: lunch");
    }
}
