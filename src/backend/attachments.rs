//! Attachments and the raw message (W2): opening and saving files (ATT-4),
//! inline images (ATT-2), a forward's copies of the original's files
//! (ATT-7) and the source (RDR-8).
//!
//! Bodies come from `messages.get format=raw`, so there's no Gmail
//! `attachmentId`: a file's bytes are taken from the raw message by the
//! part id `mime::parse_message` gave it. Raw messages aren't kept; a file
//! opened once is, in a private cache:
//!
//! - `<data dir>/attachments/cache/<key>/<name>`: a generated key per
//!   file, the name sanitised, the quarantine attribute set. Capped at
//!   `CACHE_CAP`, least recently used out first.
//! - `<data dir>/attachments/held/<uuid>/<name>`: a forward's copy of a
//!   cached file, which eviction never touches, so the queued send still
//!   has it. Removed once the send went, or after `HELD_FOR`.

use std::{
    fs, io,
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, SystemTime},
};

use anyhow::anyhow;
use gpui::{Context, Task};

use super::{
    Backend, BackendEvent, Outgoing,
    gmail::{self, GMessage, Gmail, GmailError},
    images::{self, FetchedImage},
    mime, now_ms,
    outbox::UserPriority,
    store::{IMAGE_CACHE_CAP, Store},
};
use crate::data::{Attachment, Message};

/// How much the cache of opened files keeps.
pub const CACHE_CAP: u64 = 256 * 1024 * 1024;
/// How long a held copy waits for its send.
const HELD_FOR: Duration = Duration::from_secs(30 * 24 * 60 * 60);
/// How much of the source "Show Original" shows; Save as .eml has it all.
pub const ORIGINAL_SHOWN: usize = 1024 * 1024;
/// The placeholder data's files have no bytes.
pub const DEMO_NO_BYTES: &str = "Demo attachments have no content to open";

/// The files on disk.
#[derive(Clone, Debug)]
pub struct Files {
    root: PathBuf,
}

impl Files {
    pub fn new(root: PathBuf) -> Self {
        Self { root }
    }

    pub fn in_data_dir() -> Self {
        Self::new(super::data_dir().join("attachments"))
    }

    fn cache(&self) -> PathBuf {
        self.root.join("cache")
    }

    fn held(&self) -> PathBuf {
        self.root.join("held")
    }

    /// The cached file under `key`, marked as just used.
    pub fn cached(&self, key: &str) -> Option<PathBuf> {
        let file = only_file(&self.cache().join(key))?;
        touch(&file);
        Some(file)
    }

    /// Keep `bytes` under `key` as `name` (sanitised), quarantined, then
    /// drop the least recently used files until the cache fits in `cap`.
    pub fn put(&self, key: &str, name: &str, bytes: &[u8], cap: u64) -> io::Result<PathBuf> {
        let dir = self.cache().join(key);
        fs::create_dir_all(&dir)?;
        let path = dir.join(sanitize_name(name));
        let partial = dir.join(format!(".partial-{}", uuid::Uuid::new_v4().simple()));
        fs::write(&partial, bytes)?;
        fs::rename(&partial, &path)?;
        quarantine(&path);
        self.evict(cap, Some(key));
        self.prune_held(HELD_FOR);
        Ok(path)
    }

    /// Drop the least recently used files until the cache fits in `cap`;
    /// `keep` stays whatever its size.
    pub fn evict(&self, cap: u64, keep: Option<&str>) {
        let Ok(entries) = fs::read_dir(self.cache()) else {
            return;
        };
        let mut files: Vec<(PathBuf, u64, SystemTime)> = entries
            .flatten()
            .filter_map(|entry| {
                let file = only_file(&entry.path())?;
                let meta = fs::metadata(&file).ok()?;
                Some((entry.path(), meta.len(), meta.modified().ok()?))
            })
            .collect();
        files.sort_by_key(|f| std::cmp::Reverse(f.2));
        let mut total = 0;
        for (dir, len, _) in files {
            let kept = keep.is_some_and(|k| dir.file_name().is_some_and(|n| n == k));
            total += len;
            if total > cap && !kept {
                fs::remove_dir_all(&dir).ok();
            }
        }
    }

    /// A copy of `file` that eviction leaves alone, for a queued send.
    pub fn hold(&self, file: &Path) -> io::Result<PathBuf> {
        let dir = self.held().join(uuid::Uuid::new_v4().simple().to_string());
        fs::create_dir_all(&dir)?;
        let name = file
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let path = dir.join(sanitize_name(&name));
        fs::copy(file, &path)?;
        Ok(path)
    }

    fn is_held(&self, path: &Path) -> bool {
        path.starts_with(self.held())
    }

    /// The send went: its held copies go.
    pub fn release(&self, attachments: &[Attachment]) {
        for a in attachments {
            let path = Path::new(&a.path);
            if !a.path.is_empty()
                && self.is_held(path)
                && let Some(dir) = path.parent()
            {
                fs::remove_dir_all(dir).ok();
            }
        }
    }

    /// Held copies older than `age` go: their draft was discarded or left.
    fn prune_held(&self, age: Duration) {
        let Ok(entries) = fs::read_dir(self.held()) else {
            return;
        };
        for entry in entries.flatten() {
            let old = entry
                .metadata()
                .and_then(|m| m.modified())
                .ok()
                .and_then(|t| t.elapsed().ok())
                .is_some_and(|elapsed| elapsed > age);
            if old {
                fs::remove_dir_all(entry.path()).ok();
            }
        }
    }
}

/// The one file in `dir` (not a partial write).
fn only_file(dir: &Path) -> Option<PathBuf> {
    fs::read_dir(dir).ok()?.flatten().find_map(|entry| {
        let name = entry.file_name();
        (entry.file_type().is_ok_and(|t| t.is_file()) && !name.to_string_lossy().starts_with('.'))
            .then(|| entry.path())
    })
}

fn touch(path: &Path) {
    if let Ok(file) = fs::File::options().append(true).open(path) {
        file.set_modified(SystemTime::now()).ok();
    }
}

/// A key for a file's cache directory: no part of a name or id shows in
/// the path.
pub fn cache_key(account: &str, message: &str, part: &str) -> String {
    use sha2::Digest as _;
    let hash = sha2::Sha256::digest(format!("{account}\n{message}\n{part}").as_bytes());
    hash[..16].iter().map(|b| format!("{b:02x}")).collect()
}

/// A file name that's safe to write: no directories, no control or
/// direction-changing characters, not hidden, not too long.
pub fn sanitize_name(name: &str) -> String {
    let base = name.rsplit(['/', '\\']).next().unwrap_or_default();
    let cleaned: String = base
        .chars()
        .filter(|c| !matches!(c, '\u{200e}' | '\u{200f}' | '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}'))
        .map(|c| if c.is_control() || c == ':' { '_' } else { c })
        .collect();
    let trimmed = cleaned.trim_matches(|c: char| c == '.' || c.is_whitespace());
    if trimmed.is_empty() {
        return "attachment".into();
    }
    const MAX: usize = 200;
    if trimmed.len() <= MAX {
        return trimmed.to_string();
    }
    let (stem, ext) = match trimmed.rsplit_once('.') {
        Some((stem, ext)) if ext.len() <= 16 => (stem, format!(".{ext}")),
        _ => (trimmed, String::new()),
    };
    let mut cut = MAX - ext.len();
    while !stem.is_char_boundary(cut) {
        cut -= 1;
    }
    format!("{}{ext}", &stem[..cut])
}

/// `name` in `dir`, numbered ("name 2.pdf") when it's taken.
pub fn unique_path(dir: &Path, name: &str) -> PathBuf {
    let name = sanitize_name(name);
    let path = dir.join(&name);
    if !path.exists() {
        return path;
    }
    let (stem, ext) = match name.rsplit_once('.') {
        Some((stem, ext)) if !stem.is_empty() => (stem.to_string(), format!(".{ext}")),
        _ => (name.clone(), String::new()),
    };
    (2..)
        .map(|n| dir.join(format!("{stem} {n}{ext}")))
        .find(|p| !p.exists())
        .expect("some number is free")
}

/// Whether opening the file could run code: apps, installers, scripts and
/// binaries. Those are shown in Finder instead.
pub fn is_executable(name: &str, mime: &str, head: &[u8]) -> bool {
    const EXTENSIONS: &[&str] = &[
        "app",
        "action",
        "applescript",
        "bat",
        "bin",
        "bundle",
        "cmd",
        "com",
        "command",
        "cpl",
        "csh",
        "definition",
        "dylib",
        "exe",
        "fileloc",
        "hta",
        "inetloc",
        "jar",
        "jnlp",
        "js",
        "jse",
        "kext",
        "ksh",
        "lnk",
        "mpkg",
        "msi",
        "osax",
        "pkg",
        "pl",
        "plugin",
        "prefpane",
        "ps1",
        "py",
        "rb",
        "reg",
        "run",
        "scpt",
        "scptd",
        "scr",
        "service",
        "sh",
        "shortcut",
        "so",
        "terminal",
        "tool",
        "url",
        "vb",
        "vbe",
        "vbs",
        "webloc",
        "workflow",
        "ws",
        "wsf",
        "xpc",
        "zsh",
    ];
    const MIMES: &[&str] = &[
        "application/x-sh",
        "application/x-csh",
        "application/x-shellscript",
        "application/x-executable",
        "application/x-mach-binary",
        "application/x-msdownload",
        "application/x-msdos-program",
        "application/x-apple-diskimage",
        "application/java-archive",
        "application/x-java-jnlp-file",
        "application/vnd.apple.installer+xml",
        "text/x-shellscript",
        "text/x-python",
        "text/javascript",
        "application/javascript",
    ];
    let ext = name
        .rsplit_once('.')
        .map(|(_, ext)| ext.trim().to_ascii_lowercase())
        .unwrap_or_default();
    let magic = [
        &[0xfe, 0xed, 0xfa, 0xce][..],
        &[0xfe, 0xed, 0xfa, 0xcf],
        &[0xce, 0xfa, 0xed, 0xfe],
        &[0xcf, 0xfa, 0xed, 0xfe],
        &[0xca, 0xfe, 0xba, 0xbe],
        b"#!",
        b"MZ",
    ];
    EXTENSIONS.contains(&ext.as_str())
        || MIMES.contains(&mime.to_ascii_lowercase().as_str())
        || magic.iter().any(|m| head.starts_with(m))
}

/// Mark a file as downloaded, so Gatekeeper checks it when it's opened.
pub fn quarantine(path: &Path) {
    let secs = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    let value = format!("0081;{secs:08x};Pigeon;");
    let set = std::process::Command::new("/usr/bin/xattr")
        .args(["-w", "com.apple.quarantine", &value])
        .arg(path)
        .output();
    if let Err(e) = set {
        eprintln!("Pigeon: couldn't quarantine a file: {e}");
    }
}

/// Dev only (`PIGEON_DEV=1`): `PIGEON_SAVE_DIR` takes saves straight to
/// that folder without the save panel, so a tester can check them.
pub fn dev_save_dir() -> Option<PathBuf> {
    let dev = std::env::var("PIGEON_DEV").is_ok_and(|v| !v.is_empty() && v != "0");
    let dir = std::env::var_os("PIGEON_SAVE_DIR").filter(|d| !d.is_empty())?;
    dev.then(|| PathBuf::from(dir))
}

/// The user's Downloads folder, where the save panel starts.
pub fn downloads_dir() -> PathBuf {
    std::env::var_os("HOME")
        .map_or_else(|| PathBuf::from("."), PathBuf::from)
        .join("Downloads")
}

/// Where a file's bytes are.
#[derive(Clone, Debug, PartialEq)]
pub enum Source {
    /// On disk: picked here, or a held copy.
    File(PathBuf),
    /// A part of a Gmail message.
    Part { message: String, part: String },
    /// A file a Gmail message carries, by name: one read back from a draft.
    Named { message: String, name: String },
}

impl Source {
    /// Where `attachment`, shown on the message `message_id`, comes from.
    pub fn of(message_id: &str, attachment: &Attachment) -> Option<Source> {
        if !attachment.path.is_empty() && Path::new(&attachment.path).is_file() {
            return Some(Source::File(PathBuf::from(&attachment.path)));
        }
        let message = if attachment.message_id.is_empty() {
            message_id
        } else {
            &attachment.message_id
        };
        // Not in Gmail yet.
        if message.is_empty() || message.starts_with("local-") {
            return None;
        }
        Some(if attachment.part_id.is_empty() {
            Source::Named {
                message: message.to_string(),
                name: attachment.name.clone(),
            }
        } else {
            Source::Part {
                message: message.to_string(),
                part: attachment.part_id.clone(),
            }
        })
    }

    fn key(&self, account: &str) -> String {
        match self {
            Source::File(path) => cache_key(account, "file", &path.to_string_lossy()),
            Source::Part { message, part } => cache_key(account, message, part),
            Source::Named { message, name } => cache_key(account, message, &format!("name:{name}")),
        }
    }
}

/// A message's raw bytes from Gmail, at the user's priority.
pub fn fetch_raw(gmail: &Gmail, id: &str) -> Result<Vec<u8>, GmailError> {
    let message: GMessage = gmail.call(&gmail::message_raw_request(id))?;
    message
        .raw
        .as_deref()
        .and_then(mime::decode_raw)
        .ok_or_else(|| GmailError::Parse("the message couldn't be read".into()))
}

fn plain_error(e: GmailError) -> anyhow::Error {
    match e {
        GmailError::Network(_) => anyhow!("No connection"),
        GmailError::Status { status: 401, .. } => anyhow!("Sign in again to open it"),
        GmailError::Status { status: 404, .. } => anyhow!("The message is gone from Gmail"),
        GmailError::Status { message, .. } if !message.is_empty() => anyhow!(message),
        e => anyhow!("{e}"),
    }
}

/// A file ready to open or save.
#[derive(Clone, Debug)]
pub struct Ready {
    pub path: PathBuf,
    /// It could run code: reveal it rather than open it.
    pub executable: bool,
}

/// The file for `attachment`, from the cache or its source (then cached).
/// Blocking.
pub fn materialize(
    gmail: Option<&Gmail>,
    files: &Files,
    account: &str,
    source: &Source,
    attachment: &Attachment,
) -> anyhow::Result<Ready> {
    let key = source.key(account);
    let path = match files.cached(&key) {
        Some(path) => path,
        None => {
            let bytes = match source {
                Source::File(path) => fs::read(path)?,
                Source::Part { message, part } => {
                    let gmail = gmail.ok_or_else(|| anyhow!("Not signed in"))?;
                    let raw = fetch_raw(gmail, message).map_err(plain_error)?;
                    mime::part_bytes(&raw, part)
                        .ok_or_else(|| anyhow!("The file isn't in the message"))?
                }
                Source::Named { message, name } => {
                    let gmail = gmail.ok_or_else(|| anyhow!("Not signed in"))?;
                    let raw = fetch_raw(gmail, message).map_err(plain_error)?;
                    mime::attachment_bytes(&raw, name)
                        .ok_or_else(|| anyhow!("The file isn't in the message"))?
                }
            };
            files.put(&key, &attachment.name, &bytes, CACHE_CAP)?
        }
    };
    let mut head = Vec::new();
    if let Ok(file) = fs::File::open(&path) {
        use io::Read as _;
        file.take(8).read_to_end(&mut head).ok();
    }
    let executable = is_executable(&attachment.name, &attachment.mime, &head)
        || is_executable(
            &path.file_name().unwrap_or_default().to_string_lossy(),
            "",
            &[],
        );
    Ok(Ready { path, executable })
}

/// A cached image, ready to show.
fn cached_image(store: &Store, url: &str) -> Option<FetchedImage> {
    let (_, bytes) = store.image(url).ok()??;
    images::decode(bytes).ok()
}

/// The source of a demo message, made up from what it shows.
fn demo_raw(message: &Message, subject: &str) -> Vec<u8> {
    let out = Outgoing {
        from: message.from.clone(),
        to: message.to.clone(),
        cc: message.cc.clone(),
        subject: subject.to_string(),
        markdown: crate::html::plain_text(&crate::html::parse(message.html())),
        html: message.html().to_string(),
        ..Outgoing::default()
    };
    let id = if message.message_id.is_empty() {
        mime::new_message_id(&message.from.email)
    } else {
        message.message_id.clone()
    };
    mime::build_message(
        &out,
        &mime::Build {
            message_id: id,
            date: message.date.timestamp(),
            ..Default::default()
        },
    )
}

impl Backend {
    fn user_gmail(&self) -> Option<Gmail> {
        self.transport()
            .map(|t| Gmail::new(Arc::new(UserPriority(t))))
    }

    /// The file for an attachment of `message_id`, fetched off the main
    /// thread if it isn't cached (ATT-4). Demo files have no bytes.
    pub fn attachment_file(
        &self,
        message_id: &str,
        attachment: &Attachment,
        cx: &mut Context<Self>,
    ) -> Task<anyhow::Result<Ready>> {
        let source = Source::of(message_id, attachment);
        let source = match source {
            Some(Source::File(path)) => Source::File(path),
            _ if self.is_demo() => return Task::ready(Err(anyhow!(DEMO_NO_BYTES))),
            Some(source) => source,
            None => return Task::ready(Err(anyhow!("It isn't in Gmail yet"))),
        };
        let gmail = self.user_gmail();
        let account = self.email().to_string();
        let attachment = attachment.clone();
        cx.background_executor().spawn(async move {
            materialize(
                gmail.as_ref(),
                &Files::in_data_dir(),
                &account,
                &source,
                &attachment,
            )
        })
    }

    /// A forward's files that are cached already: copied out of the cache
    /// so its eviction can't take them before the send (ATT-7).
    pub fn hold_attachments(&self, message_id: &str, attachments: &mut [Attachment]) {
        let files = Files::in_data_dir();
        for a in attachments {
            if !a.path.is_empty() {
                continue;
            }
            let Some(source) = Source::of(message_id, a) else {
                continue;
            };
            if let Some(cached) = files.cached(&source.key(self.email()))
                && let Ok(held) = files.hold(&cached)
            {
                a.path = held.to_string_lossy().into_owned();
            }
        }
    }

    /// A message's whole source (RDR-8): from Gmail, or made up from the
    /// placeholder data.
    pub fn raw_message(
        &self,
        message: &Message,
        subject: &str,
        cx: &mut Context<Self>,
    ) -> Task<anyhow::Result<Vec<u8>>> {
        if self.is_demo() || message.id.starts_with("local-") {
            return Task::ready(Ok(demo_raw(message, subject)));
        }
        let Some(gmail) = self.user_gmail() else {
            return Task::ready(Err(anyhow!("Not signed in")));
        };
        let id = message.id.clone();
        cx.background_executor()
            .spawn(async move { fetch_raw(&gmail, &id).map_err(plain_error) })
    }

    /// A message's inline images (ATT-2), from the image cache. Ones that
    /// aren't there (evicted, or a body stored before ATT-2) come from the
    /// raw message once more, which also stores its body again.
    pub fn inline_images(
        &self,
        thread_id: &str,
        message_id: &str,
        urls: Vec<String>,
        cx: &mut Context<Self>,
    ) -> Task<Vec<(String, Option<FetchedImage>)>> {
        let store = self.store.clone();
        let gmail = self.user_gmail();
        let account = self.email().to_string();
        let (thread_id, message_id) = (thread_id.to_string(), message_id.to_string());
        cx.spawn(async move |this, cx| {
            let tid = thread_id.clone();
            let (found, changed) = cx
                .background_executor()
                .spawn(async move {
                    let mut found: Vec<_> = urls
                        .into_iter()
                        .map(|u| {
                            let image = cached_image(&store, &u);
                            (u, image)
                        })
                        .collect();
                    let mut changed = false;
                    if found.iter().any(|(_, i)| i.is_none())
                        && let Some(gmail) = gmail
                        && let Ok(raw) = fetch_raw(&gmail, &message_id)
                        && let Some(parsed) = mime::parse_message(&raw)
                    {
                        let stored_html = store
                            .load_threads(&account, std::slice::from_ref(&tid))
                            .ok()
                            .and_then(|t| t.into_iter().next())
                            .and_then(|t| {
                                t.messages
                                    .into_iter()
                                    .find(|m| m.id == message_id)
                                    .map(|m| m.html().to_string())
                            });
                        if stored_html.as_deref() != Some(parsed.html.as_str()) {
                            changed = store
                                .put_bodies(&account, &tid, vec![(message_id.clone(), parsed)])
                                .is_ok();
                        } else {
                            for image in &parsed.inline_images {
                                store
                                    .put_image(
                                        &image.url,
                                        image.mime,
                                        &image.bytes,
                                        now_ms(),
                                        IMAGE_CACHE_CAP,
                                    )
                                    .ok();
                            }
                        }
                        for (url, image) in &mut found {
                            if image.is_none() {
                                *image = cached_image(&store, url);
                            }
                        }
                    }
                    (found, changed)
                })
                .await;
            if changed {
                this.update(cx, |_, cx| {
                    cx.emit(BackendEvent::ThreadsChanged(vec![thread_id]))
                })
                .ok();
            }
            found
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::http::FakeTransport;

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "pigeon-att-{name}-{}",
            uuid::Uuid::new_v4().simple()
        ));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn names_are_sanitised() {
        assert_eq!(sanitize_name("report.pdf"), "report.pdf");
        assert_eq!(sanitize_name("../../etc/passwd"), "passwd");
        assert_eq!(sanitize_name("..\\..\\boot.ini"), "boot.ini");
        assert_eq!(sanitize_name("a/b/c.txt"), "c.txt");
        assert_eq!(sanitize_name(".."), "attachment");
        assert_eq!(sanitize_name(""), "attachment");
        assert_eq!(sanitize_name("   "), "attachment");
        assert_eq!(sanitize_name("dir/"), "attachment");
        assert_eq!(sanitize_name(".hidden"), "hidden");
        assert_eq!(sanitize_name("a:b\u{0}c.txt"), "a_b_c.txt");
        // Right-to-left override hiding ".exe" as "gpj.exe".
        assert_eq!(sanitize_name("photo\u{202e}gpj.exe"), "photogpj.exe");
        let long = format!("{}.pdf", "x".repeat(400));
        let short = sanitize_name(&long);
        assert!(short.len() <= 200 && short.ends_with(".pdf"));
    }

    #[test]
    fn taken_names_are_numbered() {
        let dir = temp_dir("unique");
        assert_eq!(unique_path(&dir, "a.pdf"), dir.join("a.pdf"));
        fs::write(dir.join("a.pdf"), b"1").unwrap();
        assert_eq!(unique_path(&dir, "a.pdf"), dir.join("a 2.pdf"));
        fs::write(dir.join("a 2.pdf"), b"2").unwrap();
        assert_eq!(unique_path(&dir, "a.pdf"), dir.join("a 3.pdf"));
        fs::write(dir.join("notes"), b"1").unwrap();
        assert_eq!(unique_path(&dir, "notes"), dir.join("notes 2"));
        assert_eq!(unique_path(&dir, "../x/notes"), dir.join("notes 2"));
        fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn executables_are_recognised() {
        assert!(is_executable("Setup.app", "application/octet-stream", b""));
        assert!(is_executable("install.SH", "text/plain", b""));
        assert!(is_executable(
            "tool",
            "application/octet-stream",
            &[0xcf, 0xfa, 0xed, 0xfe]
        ));
        assert!(is_executable("run", "", b"#!/bin/sh\n"));
        assert!(is_executable("invoice.pdf.exe", "", b""));
        assert!(is_executable("x", "application/x-sh", b""));
        assert!(!is_executable("report.pdf", "application/pdf", b"%PDF-1.7"));
        assert!(!is_executable("photo.jpg", "image/jpeg", &[0xff, 0xd8]));
        assert!(!is_executable("notes", "text/plain", b"hello"));
    }

    #[test]
    fn files_are_quarantined_and_keyed_without_names() {
        let files = Files::new(temp_dir("quarantine"));
        let key = cache_key("me@example.com", "m1", "3");
        assert_eq!(key.len(), 32);
        assert_ne!(key, cache_key("me@example.com", "m1", "4"));
        let path = files
            .put(&key, "../report.pdf", b"%PDF-", CACHE_CAP)
            .unwrap();
        assert_eq!(path, files.cache().join(&key).join("report.pdf"));
        assert_eq!(files.cached(&key), Some(path.clone()));
        let attr = std::process::Command::new("/usr/bin/xattr")
            .args(["-p", "com.apple.quarantine"])
            .arg(&path)
            .output()
            .unwrap();
        assert!(String::from_utf8_lossy(&attr.stdout).contains("Pigeon"));
        fs::remove_dir_all(&files.root).ok();
    }

    #[test]
    fn eviction_drops_the_least_recently_used() {
        let files = Files::new(temp_dir("evict"));
        let old = files.put("a", "a.bin", &[0; 40], 100).unwrap();
        let earlier = SystemTime::now() - Duration::from_secs(60);
        fs::File::options()
            .append(true)
            .open(&old)
            .unwrap()
            .set_modified(earlier)
            .unwrap();
        files.put("b", "b.bin", &[0; 40], 100).unwrap();
        files.put("c", "c.bin", &[0; 40], 100).unwrap();
        assert!(files.cached("a").is_none());
        assert!(files.cached("b").is_some() && files.cached("c").is_some());
        // The one just written stays even when it alone is over the cap.
        files.put("d", "d.bin", &[0; 200], 100).unwrap();
        assert!(files.cached("d").is_some());
        assert!(files.cached("b").is_none());
        fs::remove_dir_all(&files.root).ok();
    }

    #[test]
    fn a_held_copy_outlives_eviction_until_the_send() {
        let files = Files::new(temp_dir("held"));
        let cached = files.put("k", "plan.pdf", b"%PDF-plan", CACHE_CAP).unwrap();
        let held = files.hold(&cached).unwrap();
        // The forward waits in the outbox while the cache empties.
        files.evict(0, None);
        assert!(files.cached("k").is_none());
        assert_eq!(fs::read(&held).unwrap(), b"%PDF-plan");
        let attachment = Attachment {
            path: held.to_string_lossy().into_owned(),
            ..Attachment::from_file("plan.pdf".into(), 9)
        };
        // Someone else's file is never removed.
        let mine = files.root.join("mine.pdf");
        fs::write(&mine, b"x").unwrap();
        files.release(&[Attachment {
            path: mine.to_string_lossy().into_owned(),
            ..attachment.clone()
        }]);
        assert!(mine.exists());
        files.release(&[attachment]);
        assert!(!held.exists());
        fs::remove_dir_all(&files.root).ok();
    }

    fn raw_answer(eml: &[u8]) -> String {
        use base64::Engine as _;
        let raw = base64::engine::general_purpose::URL_SAFE.encode(eml);
        format!(r#"{{"id":"m1","raw":"{raw}"}}"#)
    }

    #[test]
    fn a_part_comes_from_the_raw_message_once() {
        let eml = include_bytes!("../../tests/fixtures/google/gmail/attachments.eml");
        let parsed = mime::parse_message(eml).unwrap();
        let pdf = parsed.attachments[0].clone();
        let fake = Arc::new(FakeTransport::new().on(
            "GET",
            "/gmail/v1/users/me/messages/m1",
            &[("format", "raw")],
            &[(200, &raw_answer(eml))],
        ));
        let gmail = Gmail::new(fake.clone());
        let files = Files::new(temp_dir("part"));
        let source = Source::of("m1", &pdf).unwrap();
        let ready = materialize(Some(&gmail), &files, "me@example.com", &source, &pdf).unwrap();
        assert_eq!(ready.path.file_name().unwrap(), "Q3 report.pdf");
        assert_eq!(fs::read(&ready.path).unwrap().len() as u64, pdf.size_bytes);
        assert!(!ready.executable);
        // Opened again: from the cache.
        materialize(Some(&gmail), &files, "me@example.com", &source, &pdf).unwrap();
        assert_eq!(fake.requests().len(), 1);
        fs::remove_dir_all(&files.root).ok();
    }

    #[test]
    fn sources() {
        let mut a = Attachment {
            part_id: "4".into(),
            ..Attachment::from_file("a.pdf".into(), 1)
        };
        assert_eq!(
            Source::of("m1", &a),
            Some(Source::Part {
                message: "m1".into(),
                part: "4".into()
            })
        );
        a.message_id = "orig".into();
        assert_eq!(
            Source::of("fwd", &a),
            Some(Source::Part {
                message: "orig".into(),
                part: "4".into()
            })
        );
        a.part_id.clear();
        a.message_id.clear();
        assert_eq!(
            Source::of("d1", &a),
            Some(Source::Named {
                message: "d1".into(),
                name: "a.pdf".into()
            })
        );
        assert_eq!(Source::of("local-1", &a), None);
    }

    #[test]
    fn demo_source_is_a_message() {
        let mut m = Message::new(crate::data::Contact {
            name: "Ana".into(),
            email: "ana@example.com".into(),
        });
        m.body = crate::data::Body::Loaded {
            html: "<p>Hello <b>there</b></p>".into(),
        };
        let raw = demo_raw(&m, "Hi");
        let text = String::from_utf8_lossy(&raw);
        assert!(text.contains("Subject: Hi"));
        assert!(
            mime::parse_message(&raw)
                .unwrap()
                .html
                .contains("<b>there</b>")
        );
    }
}
