-- Pigeon's cache. Numbered blocks, applied in order and recorded in
-- PRAGMA user_version. Never edit a block once merged: add a new one with
-- the number reserved for your workstream (docs/BACKEND_SPRINT_1.md, 4.2).
-- Account-scoped tables start with `account` (the account's email), so
-- signing out is one DELETE per table.

-- migration: 001
CREATE TABLE settings (
    key TEXT PRIMARY KEY,
    value TEXT NOT NULL              -- JSON
);

CREATE TABLE labels (
    account TEXT NOT NULL,
    id TEXT NOT NULL,
    name TEXT NOT NULL,
    type TEXT NOT NULL DEFAULT 'user',   -- 'system' or 'user'
    bg TEXT,                             -- '#rrggbb'
    fg TEXT,
    threads_unread INTEGER,
    threads_total INTEGER,
    PRIMARY KEY (account, id)
);

CREATE TABLE threads (
    account TEXT NOT NULL,
    id TEXT NOT NULL,
    history_id INTEGER,
    subject TEXT NOT NULL,
    snippet TEXT NOT NULL DEFAULT '',
    last_date INTEGER NOT NULL,          -- newest message's internal_date, ms
    folder TEXT NOT NULL,                -- data::Folder::key
    unread INTEGER NOT NULL DEFAULT 0,
    starred INTEGER NOT NULL DEFAULT 0,
    has_attachments INTEGER NOT NULL DEFAULT 0,
    label_ids TEXT NOT NULL DEFAULT '[]', -- JSON array, union over messages
    search_text TEXT NOT NULL DEFAULT '', -- Thread::search_text (DAT-6)
    in_window INTEGER NOT NULL DEFAULT 1, -- inside the sync window (2.6)
    loaded TEXT NOT NULL DEFAULT 'metadata', -- 'metadata' or 'full'
    PRIMARY KEY (account, id)
);
CREATE INDEX threads_by_date ON threads (account, last_date DESC);

CREATE TABLE messages (
    account TEXT NOT NULL,
    id TEXT NOT NULL,
    thread_id TEXT NOT NULL,
    history_id INTEGER,
    internal_date INTEGER NOT NULL,      -- ms
    date TEXT NOT NULL,                  -- the Date header, RFC 3339
    label_ids TEXT NOT NULL DEFAULT '[]',
    "from" TEXT NOT NULL,                -- JSON Contact
    "to" TEXT NOT NULL DEFAULT '[]',     -- JSON [Contact]
    cc TEXT NOT NULL DEFAULT '[]',
    bcc TEXT NOT NULL DEFAULT '[]',
    reply_to TEXT NOT NULL DEFAULT '[]',
    subject TEXT NOT NULL DEFAULT '',
    snippet TEXT NOT NULL DEFAULT '',
    message_id TEXT NOT NULL DEFAULT '',
    in_reply_to TEXT NOT NULL DEFAULT '',
    "references" TEXT NOT NULL DEFAULT '[]',
    size_estimate INTEGER NOT NULL DEFAULT 0,
    attachments TEXT NOT NULL DEFAULT '[]', -- JSON [Attachment]
    PRIMARY KEY (account, id)
);
CREATE INDEX messages_by_thread ON messages (account, thread_id, internal_date);

-- Bodies are fetched on demand (RDR-6) and evicted least recently opened
-- first; eviction drops only these rows.
CREATE TABLE bodies (
    account TEXT NOT NULL,
    message_id TEXT NOT NULL,            -- messages.id
    html TEXT NOT NULL,
    plain TEXT,
    invite_uid TEXT,
    invite_method TEXT,                  -- 'request' or 'cancel'
    invite_event_id TEXT,                -- resolved Event id, when known
    bytes INTEGER NOT NULL DEFAULT 0,
    last_opened INTEGER,
    PRIMARY KEY (account, message_id)
);

CREATE TABLE drafts (
    account TEXT NOT NULL,
    local_id TEXT NOT NULL,              -- the Drafts thread's id
    draft_id TEXT,                       -- Gmail's draft id, once saved there
    message_id TEXT,
    thread_id TEXT,                      -- the thread a reply answers
    markdown TEXT,
    origin TEXT,                         -- JSON DraftOrigin
    updated_at INTEGER NOT NULL,
    PRIMARY KEY (account, local_id)
);

CREATE TABLE calendars (
    account TEXT NOT NULL,
    id TEXT NOT NULL,
    summary TEXT NOT NULL,
    bg TEXT,
    fg TEXT,
    selected INTEGER NOT NULL DEFAULT 1,
    hidden INTEGER NOT NULL DEFAULT 0,
    access_role TEXT NOT NULL DEFAULT 'owner',
    is_primary INTEGER NOT NULL DEFAULT 0,
    time_zone TEXT,
    default_reminder INTEGER,
    sync_token TEXT,
    PRIMARY KEY (account, id)
);

CREATE TABLE events (
    account TEXT NOT NULL,
    calendar_id TEXT NOT NULL,
    id TEXT NOT NULL,
    etag TEXT,
    ical_uid TEXT,
    status TEXT,
    start TEXT NOT NULL,                 -- wall clock, local
    "end" TEXT NOT NULL,
    all_day INTEGER NOT NULL DEFAULT 0,
    recurring_event_id TEXT,
    original_start TEXT,
    updated TEXT,
    json TEXT,                           -- Google's event, to write back without losing fields
    model TEXT NOT NULL,                 -- calendar::Event as JSON
    PRIMARY KEY (account, calendar_id, id)
);
CREATE INDEX events_by_uid ON events (account, ical_uid);

CREATE TABLE contacts (
    account TEXT NOT NULL,
    resource_name TEXT NOT NULL,
    email TEXT NOT NULL,
    source TEXT NOT NULL,                -- 'connection' or 'other'
    name TEXT NOT NULL DEFAULT '',
    etag TEXT,
    PRIMARY KEY (account, resource_name, email)
);

CREATE TABLE sync_state (
    account TEXT NOT NULL,
    key TEXT NOT NULL,                   -- gmail_history_id, last_sync, …
    value TEXT NOT NULL,
    PRIMARY KEY (account, key)
);

CREATE TABLE outbox (
    id INTEGER PRIMARY KEY,
    account TEXT NOT NULL,
    kind TEXT NOT NULL,                  -- 'mail.modify', 'calendar.patch', …
    resource TEXT NOT NULL DEFAULT '',   -- thread, event or draft id, for ordering
    payload TEXT NOT NULL,               -- JSON backend::Op
    not_before INTEGER NOT NULL,         -- ms
    state TEXT NOT NULL DEFAULT 'pending', -- pending, inflight, failed, done
    attempts INTEGER NOT NULL DEFAULT 0,
    next_attempt INTEGER,
    error TEXT,
    created_at INTEGER NOT NULL
);

CREATE TABLE images (
    url_hash TEXT PRIMARY KEY,
    url TEXT NOT NULL,
    mime TEXT NOT NULL,
    bytes BLOB NOT NULL,
    fetched_at INTEGER NOT NULL
);

-- migration: 090
-- W10 (CAL-20): what an invitation's iCalendar part says (JSON
-- InviteDetails), for a read-only card when its event isn't stored.
ALTER TABLE bodies ADD COLUMN invite_details TEXT;

-- migration: 100
-- Sprint 2 W1 (NTF-1): messages already notified about, so a retry or a
-- relaunch doesn't notify again. Bounded; written with the history id.
CREATE TABLE notified (
    account TEXT NOT NULL,
    message_id TEXT NOT NULL,
    at INTEGER NOT NULL,                 -- ms
    PRIMARY KEY (account, message_id)
);

-- migration: 130
-- W5: every default reminder of a calendar (CAL-33, JSON [Reminder]), the
-- conference types it allows (CAL-34, JSON [String]), and the reminders
-- already shown, so none is shown twice.
ALTER TABLE calendars ADD COLUMN default_reminders TEXT;
ALTER TABLE calendars ADD COLUMN conference_types TEXT;
CREATE TABLE fired_reminders (
    account TEXT NOT NULL,
    calendar_id TEXT NOT NULL,
    event_id TEXT NOT NULL,
    instance_start TEXT NOT NULL,        -- wall clock, local
    offset_minutes INTEGER NOT NULL,
    fired_at INTEGER NOT NULL,           -- ms
    PRIMARY KEY (account, calendar_id, event_id, instance_start, offset_minutes)
);
