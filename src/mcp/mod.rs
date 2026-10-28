//! SYS-4: an MCP server, so AI agents on this Mac can read and change the
//! user's mail and calendar while Pigeon is open. Streamable HTTP on
//! 127.0.0.1 (`server`), a bearer token kept in the Keychain, and one switch
//! per capability in Settings › MCP. Tool calls run on the main thread
//! through the same backend ops the UI uses (`tools`).

pub mod server;
pub mod tools;

use std::{
    collections::HashMap,
    sync::{Arc, Mutex, mpsc},
    time::{Duration, Instant},
};

use gpui::{App, Global, Task};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use self::server::{Activity, Host, Server, Shared};
use crate::{
    app::shell,
    backend::{BackendEvent, OpId, auth},
    logging, settings,
};

/// The port Pigeon asks for first; the next free one if it's taken.
pub const DEFAULT_PORT: u16 = 6427;
const KEYCHAIN_SERVICE: &str = "Pigeon MCP";
/// How long a tool call may take on the main thread, a Gmail search or a
/// body download included.
const CALL_TIMEOUT: Duration = Duration::from_secs(30);
/// How much longer the server waits for an answer than a call may take to
/// start, so one that started just in time still answers.
const REPLY_GRACE: Duration = Duration::from_secs(5);
/// What an agent hears when its call wasn't run in time. True: a call past
/// its deadline is never run (`Call::in_time`).
const BUSY: &str = "Pigeon was busy and didn't run this; try again";
const READ_FAILED: &str = "Couldn't read the access token from the Keychain. Unlock the Keychain, then turn the server off and on again.";
const SAVE_FAILED: &str = "Couldn't save the access token to the Keychain, so agents need a new one after Pigeon restarts.";

/// What a tool needs switched on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Capability {
    ReadMail,
    OrganizeMail,
    WriteDrafts,
    SendMail,
    ReadCalendar,
    EditCalendar,
}

impl Capability {
    #[cfg(test)]
    pub const ALL: [Capability; 6] = [
        Capability::ReadMail,
        Capability::OrganizeMail,
        Capability::WriteDrafts,
        Capability::SendMail,
        Capability::ReadCalendar,
        Capability::EditCalendar,
    ];

    /// Its switch's label in Settings › MCP.
    pub fn title(self) -> &'static str {
        match self {
            Capability::ReadMail => "Read mail",
            Capability::OrganizeMail => "Organize mail",
            Capability::WriteDrafts => "Write drafts",
            Capability::SendMail => "Send mail",
            Capability::ReadCalendar => "Read calendar",
            Capability::EditCalendar => "Edit calendar",
        }
    }

    pub fn reads(self) -> bool {
        matches!(self, Capability::ReadMail | Capability::ReadCalendar)
    }
}

/// Settings › MCP, saved. Off until turned on; then reading is allowed and
/// changing isn't, until the user turns those on too.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct McpPrefs {
    pub enabled: bool,
    pub read_mail: bool,
    pub organize_mail: bool,
    pub write_drafts: bool,
    pub send_mail: bool,
    pub read_calendar: bool,
    pub edit_calendar: bool,
}

impl Default for McpPrefs {
    fn default() -> Self {
        Self {
            enabled: false,
            read_mail: true,
            organize_mail: false,
            write_drafts: false,
            send_mail: false,
            read_calendar: true,
            edit_calendar: false,
        }
    }
}

impl McpPrefs {
    fn flag(&mut self, capability: Capability) -> &mut bool {
        match capability {
            Capability::ReadMail => &mut self.read_mail,
            Capability::OrganizeMail => &mut self.organize_mail,
            Capability::WriteDrafts => &mut self.write_drafts,
            Capability::SendMail => &mut self.send_mail,
            Capability::ReadCalendar => &mut self.read_calendar,
            Capability::EditCalendar => &mut self.edit_calendar,
        }
    }

    pub fn allows(&self, capability: Capability) -> bool {
        let mut prefs = *self;
        *prefs.flag(capability)
    }
}

/// Where the server is, for Settings.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Status {
    Off,
    Starting,
    Listening(u16),
    Failed(String),
}

/// A tool call on its way to the main thread.
struct Call {
    tool: String,
    args: Value,
    /// After this the server has told the agent it didn't run, so it
    /// mustn't: the agent may try again.
    deadline: Instant,
    reply: mpsc::SyncSender<Result<Value, String>>,
}

impl Call {
    /// The call, if it may still run at `now`; else it's answered as not run.
    fn in_time(self, now: Instant) -> Option<Self> {
        if now > self.deadline {
            self.reply.try_send(Err(BUSY.into())).ok();
            return None;
        }
        Some(self)
    }
}

enum Msg {
    Call(Call),
    Changed,
}

/// The server's way to the main thread.
struct MainThread {
    tx: async_channel::Sender<Msg>,
    timeout: Duration,
    grace: Duration,
}

impl Host for MainThread {
    fn call(&self, tool: &str, args: Value) -> Result<Value, String> {
        let (reply, answer) = mpsc::sync_channel(1);
        self.tx
            .send_blocking(Msg::Call(Call {
                tool: tool.to_string(),
                args,
                deadline: Instant::now() + self.timeout,
                reply,
            }))
            .map_err(|_| "Pigeon is quitting".to_string())?;
        answer
            .recv_timeout(self.timeout + self.grace)
            .map_err(|_| BUSY.to_string())?
    }

    fn changed(&self) {
        let _ = self.tx.send_blocking(Msg::Changed);
    }
}

struct Mcp {
    shared: Arc<Mutex<Shared>>,
    host: Arc<dyn Host>,
    server: Option<Server>,
    status: Status,
    /// Why the token won't outlive this run, if saving it failed.
    warning: Option<String>,
    /// Local ids the backend replaced with Gmail's, so ids handed to an
    /// agent keep working.
    replaced: HashMap<String, String>,
    /// Sends an agent queued, by op, and their conversation: read back once
    /// they've gone.
    sends: Vec<(OpId, String)>,
    _start: Option<Task<()>>,
}

impl Global for Mcp {}

/// Once, at launch, after the shell: start the server if it's on.
pub fn init(cx: &mut App) {
    let (tx, rx) = async_channel::unbounded::<Msg>();
    cx.spawn(async move |cx| {
        while let Ok(msg) = rx.recv().await {
            cx.update(|cx| on_message(msg, cx));
        }
    })
    .detach();
    let backend = shell::backend_of(cx);
    cx.subscribe(&backend, |_, event, cx| on_backend_event(event, cx))
        .detach();
    let prefs = settings::get(cx).mcp;
    cx.set_global(Mcp {
        shared: Arc::new(Mutex::new(Shared::new(String::new(), prefs))),
        host: Arc::new(MainThread {
            tx,
            timeout: CALL_TIMEOUT,
            grace: REPLY_GRACE,
        }),
        server: None,
        status: Status::Off,
        warning: None,
        replaced: HashMap::new(),
        sends: Vec::new(),
        _start: None,
    });
    if prefs.enabled {
        start(cx);
    }
}

fn on_message(msg: Msg, cx: &mut App) {
    match msg {
        Msg::Call(call) => {
            let Some(Call {
                tool, args, reply, ..
            }) = call.in_time(Instant::now())
            else {
                return;
            };
            // Writes finish here, before the server can give up on them.
            match tools::run(&tool, args, cx) {
                tools::Run::Now(outcome) => {
                    reply.try_send(outcome).ok();
                }
                tools::Run::Later(task) => cx
                    .spawn(async move |_| {
                        reply.try_send(task.await).ok();
                    })
                    .detach(),
            }
        }
        Msg::Changed => cx.refresh_windows(),
    }
}

fn on_backend_event(event: &BackendEvent, cx: &mut App) {
    match event {
        BackendEvent::IdReplaced { old, new } => {
            cx.global_mut::<Mcp>()
                .replaced
                .insert(old.clone(), new.clone());
        }
        BackendEvent::OpCommitted(op) | BackendEvent::OpFailed { op, .. } => {
            let mcp = cx.global_mut::<Mcp>();
            let Some(ix) = mcp.sends.iter().position(|(o, _)| o == op) else {
                return;
            };
            let (_, thread) = mcp.sends.remove(ix);
            let thread = resolve(&thread, cx);
            // The sent message shows in its conversation and in Sent.
            cx.defer(move |cx| {
                shell::backend_of(cx).update(cx, |_, cx| {
                    cx.emit(BackendEvent::ThreadsChanged(vec![thread]))
                });
            });
        }
        _ => {}
    }
}

/// An id an agent was given, as the store has it now.
pub(crate) fn resolve(id: &str, cx: &App) -> String {
    let replaced = &cx.global::<Mcp>().replaced;
    let mut id = id.to_string();
    for _ in 0..4 {
        match replaced.get(&id) {
            Some(new) => id = new.clone(),
            None => break,
        }
    }
    id
}

pub(crate) fn track_send(op: OpId, thread: String, cx: &mut App) {
    cx.global_mut::<Mcp>().sends.push((op, thread));
}

/// The Keychain item's account: the data folder, so test copies on other
/// folders keep tokens of their own.
fn keychain_account() -> String {
    crate::backend::data_dir().to_string_lossy().into_owned()
}

fn new_token() -> String {
    format!("pgn_{}", auth::random_token(32))
}

/// Save `token`; why it won't outlive this run if that fails.
fn save_token(token: &str) -> Option<String> {
    let saved = auth::keychain_set(KEYCHAIN_SERVICE, &keychain_account(), token);
    saved.err().map(|e| {
        logging::info(&format!("mcp: couldn't save the token: {e:#}"));
        SAVE_FAILED.to_string()
    })
}

/// The saved token, or, only when there's none, a new one saved now, with
/// `save_token`'s warning. A Keychain that can't be read keeps the server
/// off: a new token would replace the one agents use. Off the main thread:
/// the Keychain is a process away.
fn load_token() -> Result<(String, Option<String>), String> {
    match auth::keychain_get(KEYCHAIN_SERVICE, &keychain_account()) {
        Ok(Some(token)) => Ok((token.expose().to_string(), None)),
        Ok(None) => {
            let token = new_token();
            let warning = save_token(&token);
            Ok((token, warning))
        }
        Err(e) => {
            logging::info(&format!("mcp: couldn't read the token: {e:#}"));
            Err(READ_FAILED.into())
        }
    }
}

fn start(cx: &mut App) {
    let mcp = cx.global_mut::<Mcp>();
    if mcp.server.is_some() || mcp.status == Status::Starting {
        return;
    }
    mcp.status = Status::Starting;
    let (shared, host) = (mcp.shared.clone(), mcp.host.clone());
    let started = cx.background_executor().spawn(async move {
        let (token, warning) = load_token()?;
        shared.lock().unwrap_or_else(|e| e.into_inner()).token = token;
        Server::start(shared, host, DEFAULT_PORT)
            .map(|server| (server, warning))
            .map_err(|e| format!("Couldn't start: {e}"))
    });
    let task = cx.spawn(async move |cx| {
        let started = started.await;
        cx.update(|cx| {
            let mcp = cx.global_mut::<Mcp>();
            mcp._start = None;
            let on = mcp
                .shared
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .prefs
                .enabled;
            match started {
                Ok((server, warning)) if on => {
                    logging::info(&format!("mcp: listening on 127.0.0.1:{}", server.port()));
                    mcp.status = Status::Listening(server.port());
                    mcp.server = Some(server);
                    mcp.warning = warning;
                }
                // Turned off while it started.
                Ok(_) => mcp.status = Status::Off,
                Err(e) => {
                    logging::info(&format!("mcp: couldn't start: {e}"));
                    mcp.status = Status::Failed(e);
                }
            }
            cx.refresh_windows();
        });
    });
    cx.global_mut::<Mcp>()._start = Some(task);
}

fn stop(cx: &mut App) {
    let mcp = cx.global_mut::<Mcp>();
    mcp.server = None;
    if mcp.status != Status::Starting {
        mcp.status = Status::Off;
    }
}

fn set_prefs(prefs: McpPrefs, cx: &mut App) {
    shell::change_app_settings(cx, |s| s.mcp = prefs);
    let mcp = cx.global::<Mcp>();
    mcp.shared.lock().unwrap_or_else(|e| e.into_inner()).prefs = prefs;
    cx.refresh_windows();
}

/// Settings › MCP › MCP server.
pub fn set_enabled(on: bool, cx: &mut App) {
    let mut prefs = settings::get(cx).mcp;
    prefs.enabled = on;
    set_prefs(prefs, cx);
    if on {
        start(cx);
    } else {
        stop(cx);
    }
}

/// One capability's switch.
pub fn set_capability(capability: Capability, on: bool, cx: &mut App) {
    let mut prefs = settings::get(cx).mcp;
    *prefs.flag(capability) = on;
    set_prefs(prefs, cx);
}

/// A new token; agents using the old one get 401 from now on.
pub fn regenerate_token(cx: &mut App) {
    let token = new_token();
    {
        let mut shared = cx
            .global::<Mcp>()
            .shared
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        shared.token = token.clone();
    }
    let save = cx
        .background_executor()
        .spawn(async move { save_token(&token) });
    cx.spawn(async move |cx| {
        let warning = save.await;
        cx.update(|cx| {
            cx.global_mut::<Mcp>().warning = warning;
            cx.refresh_windows();
        });
    })
    .detach();
    cx.refresh_windows();
}

/// Why the token won't outlive this run, if saving it failed.
pub fn warning(cx: &App) -> Option<String> {
    cx.try_global::<Mcp>()?.warning.clone()
}

pub fn status(cx: &App) -> Status {
    cx.try_global::<Mcp>()
        .map_or(Status::Off, |m| m.status.clone())
}

/// `http://127.0.0.1:<port>/mcp`, while it listens.
pub fn url(cx: &App) -> Option<String> {
    match status(cx) {
        Status::Listening(port) => Some(format!("http://127.0.0.1:{port}{}", server::PATH)),
        _ => None,
    }
}

/// The token, once it's been read or made.
pub fn token(cx: &App) -> Option<String> {
    let mcp = cx.try_global::<Mcp>()?;
    let token = mcp
        .shared
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .token
        .clone();
    (!token.is_empty()).then_some(token)
}

/// The token with all but its last four characters hidden.
pub fn masked(token: &str) -> String {
    let tail: String = token
        .chars()
        .rev()
        .take(4)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();
    format!("pgn_••••••••{tail}")
}

pub fn activity(cx: &App) -> Vec<Activity> {
    cx.try_global::<Mcp>().map_or_else(Vec::new, |m| {
        m.shared
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .activity
            .iter()
            .cloned()
            .collect()
    })
}

/// The config agents take: a server by URL with its Authorization header.
pub fn config_snippet(url: &str, token: &str) -> String {
    let config = serde_json::json!({
        "mcpServers": {
            "pigeon": {
                "type": "http",
                "url": url,
                "headers": {"Authorization": format!("Bearer {token}")},
            }
        }
    });
    serde_json::to_string_pretty(&config).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn reads_are_on_by_default_and_writes_off() {
        let prefs = McpPrefs::default();
        assert!(!prefs.enabled);
        for capability in Capability::ALL {
            assert_eq!(prefs.allows(capability), capability.reads());
        }
        let mut prefs = prefs;
        *prefs.flag(Capability::SendMail) = true;
        assert!(prefs.allows(Capability::SendMail));
    }

    #[test]
    fn the_snippet_carries_the_url_and_header() {
        let snippet = config_snippet("http://127.0.0.1:6427/mcp", "pgn_abc");
        let v: Value = serde_json::from_str(&snippet).unwrap();
        assert_eq!(
            v["mcpServers"]["pigeon"]["url"],
            "http://127.0.0.1:6427/mcp"
        );
        assert_eq!(
            v["mcpServers"]["pigeon"]["headers"]["Authorization"],
            "Bearer pgn_abc"
        );
        assert_eq!(masked("pgn_abcdefgh"), "pgn_••••••••efgh");
    }

    #[test]
    fn a_call_past_its_deadline_never_runs() {
        let (tx, rx) = async_channel::unbounded();
        let host = MainThread {
            tx,
            timeout: Duration::from_millis(10),
            grace: Duration::from_millis(10),
        };
        // Nobody runs it: the server gives up and says it didn't run.
        assert_eq!(host.call("send_mail", json!({})), Err(BUSY.to_string()));
        // The main thread gets to it later, and doesn't run it.
        let Ok(Msg::Call(call)) = rx.try_recv() else {
            panic!("the call wasn't queued");
        };
        assert!(call.in_time(Instant::now()).is_none());
        // One in time runs.
        let (reply, answer) = mpsc::sync_channel(1);
        let call = Call {
            tool: "send_mail".into(),
            args: json!({}),
            deadline: Instant::now() + Duration::from_secs(60),
            reply,
        };
        assert!(call.in_time(Instant::now()).is_some());
        assert!(answer.try_recv().is_err());
        // One late is answered as not run.
        let (reply, answer) = mpsc::sync_channel(1);
        let call = Call {
            tool: "send_mail".into(),
            args: json!({}),
            deadline: Instant::now(),
            reply,
        };
        assert!(
            call.in_time(Instant::now() + Duration::from_millis(1))
                .is_none()
        );
        assert_eq!(answer.try_recv().unwrap(), Err(BUSY.to_string()));
    }
}
