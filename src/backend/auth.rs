//! Signing in to Google and staying signed in (docs/BACKEND_SPRINT_1.md,
//! 2.8): the OAuth client from `google_client.json`, the browser sign-in
//! over a loopback address with PKCE, token exchange, refresh and revoke,
//! the refresh token in the Keychain and the account in `accounts.json`.
//!
//! Secrets (`Secret`) never print: not in logs, panics or `Debug`.

use std::{
    collections::HashMap,
    io::{Read as _, Write as _},
    net::{TcpListener, TcpStream},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

use anyhow::{Context as _, anyhow};
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};

use super::http::{Authorizer, Clock, Request, Transport, TransportError};

/// What Pigeon asks for: mail, calendar, contacts, and who the user is.
pub const SCOPES: [&str; 8] = [
    "openid",
    "https://www.googleapis.com/auth/userinfo.email",
    "https://www.googleapis.com/auth/userinfo.profile",
    "https://www.googleapis.com/auth/gmail.modify",
    "https://www.googleapis.com/auth/calendar",
    "https://www.googleapis.com/auth/contacts.readonly",
    "https://www.googleapis.com/auth/contacts.other.readonly",
    "https://www.googleapis.com/auth/directory.readonly",
];

pub const GMAIL_SCOPE: &str = "https://www.googleapis.com/auth/gmail.modify";
pub const CALENDAR_SCOPE: &str = "https://www.googleapis.com/auth/calendar";
pub const CONTACTS_SCOPE: &str = "https://www.googleapis.com/auth/contacts.readonly";

/// The Keychain item's service; its account is the email.
pub const KEYCHAIN_SERVICE: &str = "Pigeon Google";

const REVOKE_URI: &str = "https://oauth2.googleapis.com/revoke";

/// How long the browser has to come back.
pub const SIGN_IN_TIMEOUT: Duration = Duration::from_secs(5 * 60);

/// A refresh starts this long before the access token expires.
const REFRESH_EARLY: Duration = Duration::from_secs(60);

/// A token, a code or the client secret.
#[derive(Clone, PartialEq, Eq, Deserialize)]
#[serde(transparent)]
pub struct Secret(String);

impl Secret {
    #[cfg(test)]
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Debug for Secret {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("***")
    }
}

// The OAuth client.

/// The `installed` half of `google_client.json`.
#[derive(Clone, Debug, Deserialize)]
pub struct ClientConfig {
    pub client_id: String,
    pub client_secret: Secret,
    pub auth_uri: String,
    pub token_uri: String,
}

impl ClientConfig {
    pub fn parse(json: &str) -> anyhow::Result<Self> {
        #[derive(Deserialize)]
        struct File {
            installed: ClientConfig,
        }
        Ok(serde_json::from_str::<File>(json)
            .context("google_client.json has no \"installed\" client")?
            .installed)
    }

    /// `google_client.json` in `dir`.
    pub fn load(dir: &Path) -> anyhow::Result<Self> {
        let path = dir.join("google_client.json");
        let json = std::fs::read_to_string(&path)
            .with_context(|| format!("no google_client.json in {}", dir.display()))?;
        Self::parse(&json)
    }
}

// The account file.

/// Who is signed in: one entry of `accounts.json`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AccountRecord {
    pub email: String,
    #[serde(default)]
    pub name: String,
    /// What Google granted (ACC-6).
    #[serde(default)]
    pub scopes: Vec<String>,
}

impl AccountRecord {
    pub fn has_scope(&self, scope: &str) -> bool {
        self.scopes.iter().any(|s| s == scope)
    }
}

fn accounts_path(dir: &Path) -> PathBuf {
    dir.join("accounts.json")
}

pub fn load_accounts(dir: &Path) -> Vec<AccountRecord> {
    std::fs::read_to_string(accounts_path(dir))
        .ok()
        .and_then(|json| serde_json::from_str(&json).ok())
        .unwrap_or_default()
}

/// Written 0600, in a 0700 folder.
pub fn save_accounts(dir: &Path, accounts: &[AccountRecord]) -> anyhow::Result<()> {
    use std::os::unix::fs::{OpenOptionsExt as _, PermissionsExt as _};
    std::fs::create_dir_all(dir)?;
    std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700)).ok();
    let path = accounts_path(dir);
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(&path)?;
    file.write_all(serde_json::to_string_pretty(accounts)?.as_bytes())?;
    Ok(())
}

// Where the refresh token lives.

pub trait TokenStore: Send + Sync {
    fn get(&self, email: &str) -> anyhow::Result<Option<Secret>>;
    fn set(&self, email: &str, token: &Secret) -> anyhow::Result<()>;
    fn delete(&self, email: &str) -> anyhow::Result<()>;
}

/// The login Keychain, through `/usr/bin/security`, which the item already
/// trusts, so nothing prompts however Pigeon is signed (6.4 item 3). The
/// token goes on stdin, never in argv.
pub struct Keychain;

const SECURITY: &str = "/usr/bin/security";
/// `security`'s exit code for an item that isn't there.
const NOT_FOUND: i32 = 44;

impl Keychain {
    /// Runs `command` in `security -i`, read from stdin.
    fn interactive(command: &str) -> anyhow::Result<std::process::ExitStatus> {
        let mut child = Command::new(SECURITY)
            .arg("-i")
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .context("couldn't run /usr/bin/security")?;
        let mut stdin = child.stdin.take().expect("piped");
        stdin.write_all(command.as_bytes())?;
        stdin.write_all(b"\n")?;
        drop(stdin);
        Ok(child.wait()?)
    }
}

/// Quoted for `security -i`, which splits words like a shell.
fn quote(value: &str) -> anyhow::Result<String> {
    if value.contains(['"', '\\', '\n']) {
        return Err(anyhow!("can't pass that to the Keychain"));
    }
    Ok(format!("\"{value}\""))
}

impl TokenStore for Keychain {
    fn get(&self, email: &str) -> anyhow::Result<Option<Secret>> {
        let output = Command::new(SECURITY)
            .args([
                "find-generic-password",
                "-s",
                KEYCHAIN_SERVICE,
                "-a",
                email,
                "-w",
            ])
            .stdin(Stdio::null())
            .stderr(Stdio::null())
            .output()
            .context("couldn't run /usr/bin/security")?;
        if output.status.code() == Some(NOT_FOUND) {
            return Ok(None);
        }
        if !output.status.success() {
            return Err(anyhow!("the Keychain refused ({})", output.status));
        }
        let token = String::from_utf8(output.stdout)
            .map_err(|_| anyhow!("the Keychain item isn't text"))?
            .trim()
            .to_string();
        Ok((!token.is_empty()).then_some(Secret(token)))
    }

    fn set(&self, email: &str, token: &Secret) -> anyhow::Result<()> {
        let command = format!(
            "add-generic-password -U -s {} -a {} -w {}",
            quote(KEYCHAIN_SERVICE)?,
            quote(email)?,
            quote(token.expose())?
        );
        let status = Self::interactive(&command)?;
        if !status.success() {
            return Err(anyhow!("couldn't save to the Keychain ({status})"));
        }
        Ok(())
    }

    fn delete(&self, email: &str) -> anyhow::Result<()> {
        let command = format!(
            "delete-generic-password -s {} -a {}",
            quote(KEYCHAIN_SERVICE)?,
            quote(email)?
        );
        let status = Self::interactive(&command)?;
        if !status.success() && status.code() != Some(NOT_FOUND) {
            return Err(anyhow!("couldn't delete from the Keychain ({status})"));
        }
        Ok(())
    }
}

/// Tests' Keychain.
#[cfg(test)]
#[derive(Default)]
pub struct MemoryTokens(Mutex<HashMap<String, Secret>>);

#[cfg(test)]
impl TokenStore for MemoryTokens {
    fn get(&self, email: &str) -> anyhow::Result<Option<Secret>> {
        Ok(self.0.lock().unwrap().get(email).cloned())
    }

    fn set(&self, email: &str, token: &Secret) -> anyhow::Result<()> {
        self.0.lock().unwrap().insert(email.into(), token.clone());
        Ok(())
    }

    fn delete(&self, email: &str) -> anyhow::Result<()> {
        self.0.lock().unwrap().remove(email);
        Ok(())
    }
}

/// The stored account and its refresh token, when both are there.
#[cfg(test)]
pub fn signed_in(dir: &Path, tokens: &dyn TokenStore) -> Option<(AccountRecord, Secret)> {
    let account = load_accounts(dir).into_iter().next()?;
    match tokens.get(&account.email) {
        Ok(Some(token)) => Some((account, token)),
        Ok(None) => None,
        Err(e) => {
            eprintln!("Pigeon: {e:#}");
            None
        }
    }
}

// PKCE (RFC 7636) and the auth URL.

/// `bytes` random bytes, base64url: 64 make an 86-character verifier.
pub fn random_token(bytes: usize) -> String {
    let mut buf = vec![0u8; bytes];
    security_framework::random::SecRandom::default()
        .copy_bytes(&mut buf)
        .expect("the system's random source works");
    URL_SAFE_NO_PAD.encode(buf)
}

/// The S256 challenge for `verifier`.
pub fn challenge(verifier: &str) -> String {
    URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()))
}

use super::http::encode;

/// Percent-decoded, `+` as a space.
pub fn decode(value: &str) -> String {
    let bytes = value.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut ix = 0;
    let hex = |b: u8| (b as char).to_digit(16).map(|d| d as u8);
    while ix < bytes.len() {
        match bytes[ix] {
            b'%' if ix + 2 < bytes.len() => match (hex(bytes[ix + 1]), hex(bytes[ix + 2])) {
                (Some(hi), Some(lo)) => {
                    out.push(hi << 4 | lo);
                    ix += 3;
                    continue;
                }
                _ => out.push(b'%'),
            },
            b'+' => out.push(b' '),
            b => out.push(b),
        }
        ix += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn form(pairs: &[(&str, &str)]) -> String {
    pairs
        .iter()
        .map(|(k, v)| format!("{k}={}", encode(v)))
        .collect::<Vec<_>>()
        .join("&")
}

/// Google's consent page, asking for `SCOPES` and a refresh token.
pub fn auth_url(
    client: &ClientConfig,
    redirect_uri: &str,
    challenge: &str,
    state: &str,
    login_hint: Option<&str>,
) -> String {
    let scope = SCOPES.join(" ");
    let mut pairs = vec![
        ("client_id", client.client_id.as_str()),
        ("redirect_uri", redirect_uri),
        ("response_type", "code"),
        ("scope", scope.as_str()),
        ("code_challenge", challenge),
        ("code_challenge_method", "S256"),
        ("state", state),
        ("access_type", "offline"),
        ("prompt", "consent"),
        ("include_granted_scopes", "true"),
    ];
    if let Some(hint) = login_hint {
        pairs.push(("login_hint", hint));
    }
    format!("{}?{}", client.auth_uri, form(&pairs))
}

// The loopback listener.

#[derive(Debug, PartialEq, Eq)]
pub enum CallbackError {
    /// Not our sign-in: a stale tab, or the browser asking for a favicon.
    WrongState,
    /// The user said no, or Google refused (`error=`).
    Denied(String),
    Malformed,
}

/// The code in a redirect's request line (`GET /?code=…&state=… HTTP/1.1`),
/// if its `state` is ours.
pub fn parse_callback(request_line: &str, state: &str) -> Result<Secret, CallbackError> {
    let target = request_line
        .split_whitespace()
        .nth(1)
        .ok_or(CallbackError::Malformed)?;
    let query = target.split_once('?').map(|(_, q)| q).unwrap_or_default();
    let params: HashMap<String, String> = query
        .split('&')
        .filter_map(|p| p.split_once('='))
        .map(|(k, v)| (decode(k), decode(v)))
        .collect();
    if params.get("state").map(String::as_str) != Some(state) {
        return Err(CallbackError::WrongState);
    }
    if let Some(error) = params.get("error") {
        return Err(CallbackError::Denied(error.clone()));
    }
    params
        .get("code")
        .filter(|c| !c.is_empty())
        .map(|c| Secret(c.clone()))
        .ok_or(CallbackError::Malformed)
}

#[derive(Debug, PartialEq, Eq)]
pub enum SignInError {
    Cancelled,
    TimedOut,
    Denied,
    /// Shown as is.
    Failed(String),
}

impl std::fmt::Display for SignInError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SignInError::Cancelled => write!(f, "Sign-in cancelled."),
            SignInError::TimedOut => write!(f, "The browser didn't come back in 5 minutes."),
            SignInError::Denied => write!(f, "Google sign-in was cancelled in the browser."),
            SignInError::Failed(message) => write!(f, "{message}"),
        }
    }
}

/// 127.0.0.1 on a free port.
pub fn listen() -> std::io::Result<(TcpListener, u16)> {
    let listener = TcpListener::bind("127.0.0.1:0")?;
    let port = listener.local_addr()?.port();
    listener.set_nonblocking(true)?;
    Ok((listener, port))
}

const PAGE_DONE: &str = "Signed in to Pigeon. You can close this tab.";
const PAGE_DENIED: &str = "Sign-in was cancelled. You can close this tab and go back to Pigeon.";
const PAGE_STALE: &str = "This sign-in link is out of date. Go back to Pigeon and try again.";

fn reply(stream: &mut TcpStream, status: &str, text: &str) {
    let body = format!(
        "<!doctype html><meta charset=utf-8><title>Pigeon</title>\
         <body style=\"font:15px -apple-system,sans-serif;margin:4em;text-align:center\">\
         <p>{text}</p></body>"
    );
    let response = format!(
        "HTTP/1.1 {status}\r\nContent-Type: text/html; charset=utf-8\r\n\
         Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    stream.write_all(response.as_bytes()).ok();
}

/// The request's first line, read off `stream` (at most 8 KB).
fn read_request_line(stream: &mut TcpStream) -> Option<String> {
    stream.set_nonblocking(false).ok()?;
    stream.set_read_timeout(Some(Duration::from_secs(5))).ok()?;
    let mut buf = Vec::new();
    let mut chunk = [0u8; 1024];
    while !buf.windows(2).any(|w| w == b"\r\n") && buf.len() < 8192 {
        let n = stream.read(&mut chunk).ok()?;
        if n == 0 {
            break;
        }
        buf.extend_from_slice(&chunk[..n]);
    }
    let text = String::from_utf8_lossy(&buf);
    text.lines().next().map(str::to_string)
}

/// Waits on `listener` for the browser to come back with our `state`;
/// answers anything else and keeps waiting. Gives up on `cancel` or after
/// `timeout`.
pub fn wait_for_code(
    listener: &TcpListener,
    state: &str,
    cancel: &AtomicBool,
    timeout: Duration,
) -> Result<Secret, SignInError> {
    let started = Instant::now();
    loop {
        if cancel.load(Ordering::Relaxed) {
            return Err(SignInError::Cancelled);
        }
        if started.elapsed() > timeout {
            return Err(SignInError::TimedOut);
        }
        match listener.accept() {
            Ok((mut stream, _)) => {
                let Some(line) = read_request_line(&mut stream) else {
                    continue;
                };
                match parse_callback(&line, state) {
                    Ok(code) => {
                        reply(&mut stream, "200 OK", PAGE_DONE);
                        return Ok(code);
                    }
                    Err(CallbackError::Denied(_)) => {
                        reply(&mut stream, "200 OK", PAGE_DENIED);
                        return Err(SignInError::Denied);
                    }
                    Err(_) => reply(&mut stream, "400 Bad Request", PAGE_STALE),
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                std::thread::sleep(Duration::from_millis(100));
            }
            Err(e) => {
                eprintln!("Pigeon: sign-in listener: {e}");
                return Err(SignInError::Failed(
                    "Sign-in stopped: the browser's answer didn't arrive. Try again.".into(),
                ));
            }
        }
    }
}

// Tokens.

/// What the token endpoint answers.
#[derive(Debug)]
pub struct Tokens {
    pub access_token: Secret,
    pub expires_in: Duration,
    /// Only on the first exchange (with `prompt=consent`).
    pub refresh_token: Option<Secret>,
    /// Granted, which may be fewer than asked (granular consent).
    pub scopes: Vec<String>,
    pub id_token: Option<Secret>,
}

#[derive(Debug, PartialEq, Eq)]
pub enum TokenError {
    /// The refresh token was revoked or expired: sign in again.
    InvalidGrant,
    Network(String),
    Other(String),
}

impl std::fmt::Display for TokenError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TokenError::InvalidGrant => write!(f, "Google no longer accepts this sign-in"),
            TokenError::Network(e) => write!(f, "Couldn't reach Google: {e}"),
            TokenError::Other(e) => write!(f, "Google refused the sign-in: {e}"),
        }
    }
}

pub fn parse_token_response(status: u16, body: &[u8]) -> Result<Tokens, TokenError> {
    #[derive(Deserialize)]
    struct Body {
        access_token: Option<Secret>,
        expires_in: Option<u64>,
        refresh_token: Option<Secret>,
        scope: Option<String>,
        id_token: Option<Secret>,
        error: Option<String>,
    }
    let parsed: Body = serde_json::from_slice(body)
        .map_err(|_| TokenError::Other(format!("unreadable answer ({status})")))?;
    if let Some(error) = parsed.error {
        return Err(if error == "invalid_grant" {
            TokenError::InvalidGrant
        } else {
            TokenError::Other(error)
        });
    }
    let access_token = parsed
        .access_token
        .filter(|_| (200..300).contains(&status))
        .ok_or_else(|| TokenError::Other(format!("no access token ({status})")))?;
    Ok(Tokens {
        access_token,
        expires_in: Duration::from_secs(parsed.expires_in.unwrap_or(3600)),
        refresh_token: parsed.refresh_token,
        scopes: parsed
            .scope
            .unwrap_or_default()
            .split_whitespace()
            .map(str::to_string)
            .collect(),
        id_token: parsed.id_token,
    })
}

fn post_token(
    transport: &dyn Transport,
    client: &ClientConfig,
    pairs: &[(&str, &str)],
) -> Result<Tokens, TokenError> {
    let request = Request {
        method: "POST",
        url: client.token_uri.clone(),
        headers: vec![(
            "Content-Type".into(),
            "application/x-www-form-urlencoded".into(),
        )],
        body: form(pairs).into_bytes(),
        cost: 0,
    };
    let response = transport.send(&request).map_err(|e| match e {
        TransportError::Network(e) => TokenError::Network(e),
        TransportError::AuthExpired => TokenError::InvalidGrant,
        TransportError::TooLarge => TokenError::Other(e.to_string()),
    })?;
    if response.status >= 500 {
        return Err(TokenError::Network(format!("status {}", response.status)));
    }
    parse_token_response(response.status, &response.body)
}

pub fn exchange_code(
    transport: &dyn Transport,
    client: &ClientConfig,
    code: &Secret,
    verifier: &str,
    redirect_uri: &str,
) -> Result<Tokens, TokenError> {
    post_token(
        transport,
        client,
        &[
            ("grant_type", "authorization_code"),
            ("code", code.expose()),
            ("code_verifier", verifier),
            ("redirect_uri", redirect_uri),
            ("client_id", &client.client_id),
            ("client_secret", client.client_secret.expose()),
        ],
    )
}

pub fn refresh(
    transport: &dyn Transport,
    client: &ClientConfig,
    refresh_token: &Secret,
) -> Result<Tokens, TokenError> {
    post_token(
        transport,
        client,
        &[
            ("grant_type", "refresh_token"),
            ("refresh_token", refresh_token.expose()),
            ("client_id", &client.client_id),
            ("client_secret", client.client_secret.expose()),
        ],
    )
}

/// Ends the grant at Google. Best effort: signing out goes on regardless.
pub fn revoke(transport: &dyn Transport, token: &Secret) -> Result<(), TransportError> {
    let request = Request {
        method: "POST",
        url: REVOKE_URI.into(),
        headers: vec![(
            "Content-Type".into(),
            "application/x-www-form-urlencoded".into(),
        )],
        body: form(&[("token", token.expose())]).into_bytes(),
        cost: 0,
    };
    transport.send(&request).map(|_| ())
}

/// Who signed in, from the ID token. It came straight from Google's token
/// endpoint over TLS, so its signature isn't checked (Google's guidance for
/// that case).
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Claims {
    pub email: String,
    pub name: String,
}

pub fn id_token_claims(id_token: &str) -> Option<Claims> {
    #[derive(Deserialize)]
    struct Payload {
        email: Option<String>,
        name: Option<String>,
    }
    let payload = id_token.split('.').nth(1)?;
    let bytes = URL_SAFE_NO_PAD.decode(payload.trim_end_matches('=')).ok()?;
    let payload: Payload = serde_json::from_slice(&bytes).ok()?;
    Some(Claims {
        email: payload.email?,
        name: payload.name.unwrap_or_default(),
    })
}

/// After the browser came back: the tokens, then the account saved, its
/// refresh token in `tokens` and its entry the only one in `accounts.json`.
/// Another account still set up here (its refresh token kept) stays, and
/// the new grant is revoked: switching takes signing out first (MB-056).
/// One whose token is gone already is forgotten.
pub fn finish_sign_in(
    transport: &dyn Transport,
    client: &ClientConfig,
    code: &Secret,
    verifier: &str,
    redirect_uri: &str,
    tokens: &dyn TokenStore,
    dir: &Path,
) -> Result<(AccountRecord, Secret), SignInError> {
    let granted = exchange_code(transport, client, code, verifier, redirect_uri)
        .map_err(|e| SignInError::Failed(e.to_string()))?;
    let refresh_token = granted.refresh_token.clone().ok_or_else(|| {
        SignInError::Failed("Google didn't give Pigeon offline access. Try again.".into())
    })?;
    let claims = granted
        .id_token
        .as_ref()
        .and_then(|t| id_token_claims(t.expose()))
        .ok_or_else(|| SignInError::Failed("Google didn't say who signed in.".into()))?;
    let account = AccountRecord {
        email: claims.email,
        name: claims.name,
        scopes: granted.scopes,
    };
    let others: Vec<AccountRecord> = load_accounts(dir)
        .into_iter()
        .filter(|old| !old.email.eq_ignore_ascii_case(&account.email))
        .collect();
    if let Some(old) = others
        .iter()
        .find(|old| tokens.get(&old.email).is_ok_and(|t| t.is_some()))
    {
        if let Err(e) = revoke(transport, &refresh_token) {
            eprintln!("Pigeon: couldn't revoke the sign-in: {e}");
        }
        return Err(SignInError::Failed(format!(
            "You signed in as {}. Pigeon is set up for {}. Sign out first to switch accounts.",
            account.email, old.email
        )));
    }
    for old in &others {
        tokens.delete(&old.email).ok();
    }
    tokens.set(&account.email, &refresh_token).map_err(|e| {
        eprintln!("Pigeon: keychain: {e:#}");
        SignInError::Failed("Couldn't save the sign-in to the Keychain. Try again.".into())
    })?;
    save_accounts(dir, std::slice::from_ref(&account)).map_err(|e| {
        eprintln!("Pigeon: accounts.json: {e:#}");
        SignInError::Failed("Couldn't save the account. Try again.".into())
    })?;
    Ok((account, refresh_token))
}

/// Signing out, past the cache: revoke at Google (best effort), then drop
/// the Keychain item and the `accounts.json` entry.
pub fn forget(
    transport: &dyn Transport,
    refresh_token: Option<&Secret>,
    email: &str,
    tokens: &dyn TokenStore,
    dir: &Path,
) -> anyhow::Result<()> {
    if let Some(token) = refresh_token
        && let Err(e) = revoke(transport, token)
    {
        eprintln!("Pigeon: couldn't revoke the sign-in: {e}");
    }
    tokens.delete(email)?;
    let rest: Vec<_> = load_accounts(dir)
        .into_iter()
        .filter(|a| a.email != email)
        .collect();
    save_accounts(dir, &rest)
}

// Access tokens, refreshed as needed.

/// Hands out access tokens for one account. One refresh at a time: callers
/// that need one while it runs wait for its result.
pub struct Auth {
    client: ClientConfig,
    refresh_token: Secret,
    /// The token endpoint is called through this, unauthenticated.
    transport: Arc<dyn Transport>,
    clock: Arc<dyn Clock>,
    access: Mutex<Option<(Secret, Instant)>>,
    refreshing: Mutex<()>,
    expired: AtomicBool,
}

impl Auth {
    pub fn new(
        client: ClientConfig,
        refresh_token: Secret,
        transport: Arc<dyn Transport>,
        clock: Arc<dyn Clock>,
    ) -> Self {
        Self {
            client,
            refresh_token,
            transport,
            clock,
            access: Mutex::new(None),
            refreshing: Mutex::new(()),
            expired: AtomicBool::new(false),
        }
    }

    pub fn refresh_token(&self) -> &Secret {
        &self.refresh_token
    }

    /// `invalid_grant` came back: "Sign in again".
    pub fn is_expired(&self) -> bool {
        self.expired.load(Ordering::Relaxed)
    }

    fn cached(&self) -> Option<String> {
        let access = self.access.lock().unwrap_or_else(|e| e.into_inner());
        access
            .as_ref()
            .filter(|(_, expiry)| self.clock.now() + REFRESH_EARLY < *expiry)
            .map(|(token, _)| token.expose().to_string())
    }
}

impl Authorizer for Auth {
    fn access_token(&self) -> Result<String, TransportError> {
        if self.is_expired() {
            return Err(TransportError::AuthExpired);
        }
        if let Some(token) = self.cached() {
            return Ok(token);
        }
        let _refreshing = self.refreshing.lock().unwrap_or_else(|e| e.into_inner());
        // Someone else's refresh may have finished while this one waited.
        if let Some(token) = self.cached() {
            return Ok(token);
        }
        if self.is_expired() {
            return Err(TransportError::AuthExpired);
        }
        match refresh(&*self.transport, &self.client, &self.refresh_token) {
            Ok(tokens) => {
                let token = tokens.access_token.expose().to_string();
                *self.access.lock().unwrap_or_else(|e| e.into_inner()) =
                    Some((tokens.access_token, self.clock.now() + tokens.expires_in));
                Ok(token)
            }
            Err(TokenError::InvalidGrant) => {
                self.expired.store(true, Ordering::Relaxed);
                Err(TransportError::AuthExpired)
            }
            Err(TokenError::Network(e)) => Err(TransportError::Network(e)),
            // A refusal other than invalid_grant (a bad client, say) won't
            // fix itself, but retrying is harmless and keeps the cache.
            Err(TokenError::Other(e)) => Err(TransportError::Network(e)),
        }
    }

    fn invalidate(&self, token: &str) {
        let mut access = self.access.lock().unwrap_or_else(|e| e.into_inner());
        if access.as_ref().is_some_and(|(t, _)| t.expose() == token) {
            *access = None;
        }
    }
}

impl std::fmt::Debug for Auth {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Auth")
            .field("client_id", &self.client.client_id)
            .field("refresh_token", &self.refresh_token)
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Barrier;

    use super::*;
    use crate::backend::http::{FakeTransport, SystemClock};

    const TOKEN_PATH: &str = "/token";

    fn client() -> ClientConfig {
        ClientConfig::parse(
            r#"{"installed":{"client_id":"id.apps.example.com","client_secret":"shh-secret",
            "auth_uri":"https://accounts.google.com/o/oauth2/auth",
            "token_uri":"https://oauth2.googleapis.com/token","redirect_uris":["http://localhost"]}}"#,
        )
        .unwrap()
    }

    /// A JWT-shaped ID token with `claims` as its payload.
    fn id_token(claims: &str) -> String {
        format!("e30.{}.sig", URL_SAFE_NO_PAD.encode(claims))
    }

    fn scratch_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "pigeon-test-{name}-{}-{}",
            std::process::id(),
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn pkce_matches_rfc_7636() {
        // Appendix B.
        assert_eq!(
            challenge("dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk"),
            "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM"
        );
        let verifier = random_token(64);
        assert_eq!(verifier.len(), 86);
        assert!((43..=128).contains(&verifier.len()));
        assert_ne!(verifier, random_token(64));
    }

    #[test]
    fn auth_url_asks_for_offline_consent_with_pkce() {
        let url = auth_url(
            &client(),
            "http://127.0.0.1:5000",
            "CHALLENGE",
            "STATE",
            Some("me@example.com"),
        );
        let request = Request::get(url.clone(), 0);
        let query: HashMap<String, String> = request
            .query()
            .into_iter()
            .map(|(k, v)| (k.to_string(), decode(v)))
            .collect();
        assert!(url.starts_with("https://accounts.google.com/o/oauth2/auth?"));
        assert_eq!(query["client_id"], "id.apps.example.com");
        assert_eq!(query["redirect_uri"], "http://127.0.0.1:5000");
        assert_eq!(query["response_type"], "code");
        assert_eq!(query["code_challenge"], "CHALLENGE");
        assert_eq!(query["code_challenge_method"], "S256");
        assert_eq!(query["state"], "STATE");
        assert_eq!(query["access_type"], "offline");
        assert_eq!(query["prompt"], "consent");
        assert_eq!(query["include_granted_scopes"], "true");
        assert_eq!(query["login_hint"], "me@example.com");
        assert_eq!(query["scope"], SCOPES.join(" "));
        assert!(!url.contains("shh-secret"));
    }

    #[test]
    fn loopback_takes_only_our_state() {
        assert_eq!(
            parse_callback(
                "GET /?state=abc&code=4%2F0Ab-x&scope=openid HTTP/1.1",
                "abc"
            )
            .unwrap()
            .expose(),
            "4/0Ab-x"
        );
        assert_eq!(
            parse_callback("GET /?state=other&code=x HTTP/1.1", "abc").unwrap_err(),
            CallbackError::WrongState
        );
        assert_eq!(
            parse_callback("GET /favicon.ico HTTP/1.1", "abc").unwrap_err(),
            CallbackError::WrongState
        );
        assert_eq!(
            parse_callback("GET /?error=access_denied&state=abc HTTP/1.1", "abc").unwrap_err(),
            CallbackError::Denied("access_denied".into())
        );
        assert_eq!(
            parse_callback("GET /?state=abc HTTP/1.1", "abc").unwrap_err(),
            CallbackError::Malformed
        );
        assert_eq!(
            parse_callback("", "abc").unwrap_err(),
            CallbackError::Malformed
        );
    }

    #[test]
    fn loopback_listener_serves_one_matching_request() {
        let (listener, port) = listen().unwrap();
        let cancel = AtomicBool::new(false);
        let browser = std::thread::spawn(move || {
            let send = |path: &str| {
                let mut stream = TcpStream::connect(("127.0.0.1", port)).unwrap();
                write!(stream, "GET {path} HTTP/1.1\r\nHost: localhost\r\n\r\n").unwrap();
                let mut answer = String::new();
                stream.read_to_string(&mut answer).ok();
                answer
            };
            let stale = send("/?state=old&code=zzz");
            let done = send("/?state=s1&code=c1");
            (stale, done)
        });
        let code = wait_for_code(&listener, "s1", &cancel, Duration::from_secs(10)).unwrap();
        assert_eq!(code.expose(), "c1");
        let (stale, done) = browser.join().unwrap();
        assert!(stale.starts_with("HTTP/1.1 400"));
        assert!(done.contains("You can close this tab"));
    }

    #[test]
    fn loopback_gives_up_on_cancel() {
        let (listener, _) = listen().unwrap();
        let cancel = AtomicBool::new(true);
        assert_eq!(
            wait_for_code(&listener, "s", &cancel, Duration::from_secs(10)).unwrap_err(),
            SignInError::Cancelled
        );
    }

    #[test]
    fn token_response_reads_granted_scopes() {
        let body = format!(
            r#"{{"access_token":"fake-access","expires_in":3599,"refresh_token":"fake-refresh",
            "scope":"openid https://www.googleapis.com/auth/gmail.modify","token_type":"Bearer",
            "id_token":"{}"}}"#,
            id_token(r#"{"email":"me@example.com","name":"Me Example"}"#)
        );
        let tokens = parse_token_response(200, body.as_bytes()).unwrap();
        assert_eq!(tokens.access_token.expose(), "fake-access");
        assert_eq!(tokens.expires_in, Duration::from_secs(3599));
        assert_eq!(tokens.refresh_token.unwrap().expose(), "fake-refresh");
        assert_eq!(tokens.scopes, ["openid", GMAIL_SCOPE]);
        let claims = id_token_claims(tokens.id_token.unwrap().expose()).unwrap();
        assert_eq!(claims.email, "me@example.com");
        assert_eq!(claims.name, "Me Example");

        assert_eq!(
            parse_token_response(400, br#"{"error":"invalid_grant","error_description":"x"}"#)
                .unwrap_err(),
            TokenError::InvalidGrant
        );
        assert!(matches!(
            parse_token_response(401, br#"{"error":"invalid_client"}"#),
            Err(TokenError::Other(_))
        ));
    }

    #[test]
    fn secrets_never_print() {
        let client = client();
        let printed = format!("{client:?} {:?}", Secret::new("fake-refresh"));
        assert!(!printed.contains("shh-secret"));
        assert!(!printed.contains("fake-refresh"));
        assert!(printed.contains("***"));
        let auth = Auth::new(
            client,
            Secret::new("fake-refresh"),
            Arc::new(FakeTransport::new()),
            Arc::new(SystemClock),
        );
        assert!(!format!("{auth:?}").contains("fake-refresh"));
    }

    const REFRESHED: &str = r#"{"access_token":"fake-access-1","expires_in":3600}"#;

    #[test]
    fn refresh_runs_once_for_many_callers() {
        let fake = Arc::new(FakeTransport::new().on("POST", TOKEN_PATH, &[], &[(200, REFRESHED)]));
        let auth = Arc::new(Auth::new(
            client(),
            Secret::new("fake-refresh"),
            fake.clone(),
            Arc::new(SystemClock),
        ));
        let barrier = Arc::new(Barrier::new(8));
        let threads: Vec<_> = (0..8)
            .map(|_| {
                let (auth, barrier) = (auth.clone(), barrier.clone());
                std::thread::spawn(move || {
                    barrier.wait();
                    auth.access_token().unwrap()
                })
            })
            .collect();
        for thread in threads {
            assert_eq!(thread.join().unwrap(), "fake-access-1");
        }
        let requests = fake.requests();
        assert_eq!(requests.len(), 1);
        let body = String::from_utf8(requests[0].body.clone()).unwrap();
        assert!(body.contains("grant_type=refresh_token"));
        assert!(body.contains("refresh_token=fake-refresh"));

        // A 401 with the current token refreshes again; a stale one doesn't.
        auth.invalidate("some-older-token");
        auth.access_token().unwrap();
        assert_eq!(fake.requests().len(), 1);
        auth.invalidate("fake-access-1");
        auth.access_token().unwrap();
        assert_eq!(fake.requests().len(), 2);
    }

    #[test]
    fn invalid_grant_means_sign_in_again() {
        let fake = Arc::new(FakeTransport::new().on(
            "POST",
            TOKEN_PATH,
            &[],
            &[(400, r#"{"error":"invalid_grant"}"#)],
        ));
        let auth = Auth::new(
            client(),
            Secret::new("fake-refresh"),
            fake.clone(),
            Arc::new(SystemClock),
        );
        assert_eq!(auth.access_token(), Err(TransportError::AuthExpired));
        assert!(auth.is_expired());
        // It doesn't ask again.
        assert_eq!(auth.access_token(), Err(TransportError::AuthExpired));
        assert_eq!(fake.requests().len(), 1);
    }

    #[test]
    fn sign_in_saves_and_sign_out_forgets() {
        let dir = scratch_dir("accounts");
        let tokens = MemoryTokens::default();
        // Someone else is set up here.
        save_accounts(
            &dir,
            &[AccountRecord {
                email: "old@example.com".into(),
                ..Default::default()
            }],
        )
        .unwrap();
        tokens
            .set("old@example.com", &Secret::new("old-refresh"))
            .unwrap();

        let answer = format!(
            r#"{{"access_token":"fake-access","expires_in":3600,"refresh_token":"fake-refresh",
            "scope":"openid {CALENDAR_SCOPE}","id_token":"{}"}}"#,
            id_token(r#"{"email":"me@example.com","name":"Me Example"}"#)
        );
        let fake = FakeTransport::new()
            .on("POST", TOKEN_PATH, &[], &[(200, answer.as_str())])
            .on("POST", "/revoke", &[], &[(200, "")]);
        let sign_in = || {
            finish_sign_in(
                &fake,
                &client(),
                &Secret::new("code-1"),
                "verifier",
                "http://127.0.0.1:1",
                &tokens,
                &dir,
            )
        };
        // MB-056: not over them; the new grant goes, the old one stays.
        let Err(SignInError::Failed(message)) = sign_in() else {
            panic!("signed in over another account");
        };
        assert!(message.contains("me@example.com") && message.contains("old@example.com"));
        assert!(message.contains("Sign out first"));
        assert_eq!(fake.requests().last().unwrap().path(), "/revoke");
        assert!(tokens.get("old@example.com").unwrap().is_some());
        assert_eq!(load_accounts(&dir)[0].email, "old@example.com");

        // Its sign-in already gone (only the entry left): it's forgotten.
        tokens.delete("old@example.com").unwrap();
        let before = fake.requests().len();
        let (account, refresh) = sign_in().unwrap();
        assert_eq!(account.email, "me@example.com");
        assert_eq!(account.name, "Me Example");
        assert!(account.has_scope(CALENDAR_SCOPE));
        assert!(!account.has_scope(GMAIL_SCOPE));
        assert_eq!(refresh.expose(), "fake-refresh");
        let body = String::from_utf8(fake.requests()[before].body.clone()).unwrap();
        assert!(body.contains("grant_type=authorization_code"));
        assert!(body.contains("code_verifier=verifier"));
        assert_eq!(load_accounts(&dir), std::slice::from_ref(&account));
        assert!(tokens.get("old@example.com").unwrap().is_none());
        let (found, token) = signed_in(&dir, &tokens).unwrap();
        assert_eq!(found, account);
        assert_eq!(token, refresh);
        use std::os::unix::fs::PermissionsExt as _;
        let mode = std::fs::metadata(dir.join("accounts.json"))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600);

        forget(&fake, Some(&refresh), &account.email, &tokens, &dir).unwrap();
        assert!(signed_in(&dir, &tokens).is_none());
        assert!(load_accounts(&dir).is_empty());
        assert!(tokens.get("me@example.com").unwrap().is_none());
        let revoke = fake.requests().pop().unwrap();
        assert_eq!(revoke.path(), "/revoke");
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn denied_consent_reads_plainly() {
        assert_eq!(
            SignInError::Denied.to_string(),
            "Google sign-in was cancelled in the browser."
        );
    }

    #[test]
    fn percent_coding_round_trips() {
        let text = "a b/c?d=e&f~g+ü";
        assert_eq!(decode(&encode(text)), text);
        assert_eq!(decode("a+b%2"), "a b%2");
    }
}
