//! W11: what the status line says (ACC-5, STA-1/3/4), and noticing the
//! network go away between polls (STA-3).
//!
//! Sync reports Syncing and Synced. On top of that: an expired sign-in
//! wins, then Offline with the queued changes, then "Waiting for Gmail…"
//! while a request waits out a long backoff. A probe (a TCP connect to
//! Google) runs every few seconds while the window is active, so Offline
//! shows within about 10 s; offline, it backs off 15 s → 2 min, and the
//! first one that connects brings sync and the queue back.

use std::{
    net::{TcpStream, ToSocketAddrs},
    time::Duration,
};

use gpui::{Context, Task};

use super::{Backend, BackendEvent, Source, SyncStatus, http, sync::LAST_SYNC_KEY};

const PROBE_ACTIVE: Duration = Duration::from_secs(8);
const PROBE_INACTIVE: Duration = Duration::from_secs(60);
/// A failed probe tries again this soon before saying Offline.
const PROBE_RECHECK: Duration = Duration::from_secs(1);
const PROBE_TIMEOUT: Duration = Duration::from_secs(5);
const PROBE_HOST: (&str, u16) = ("gmail.googleapis.com", 443);

/// Quit waits this long at most for the queue (2.7).
pub const QUIT_WAIT: Duration = Duration::from_secs(5);

#[derive(Default)]
pub struct Health {
    /// What sync last said.
    base: Option<SyncStatus>,
    offline: bool,
    auth_expired: bool,
    waiting_for_gmail: bool,
    /// Changes in the outbox.
    queued: usize,
    /// Probes failed in a row.
    probe_failures: u32,
    /// The minute "Synced N min ago" was last told for, so it refreshes.
    told_minute: i64,
    _probe: Option<Task<()>>,
}

impl Health {
    pub fn new(status: SyncStatus) -> Self {
        let mut health = Self::default();
        health.note(status);
        health
    }

    /// A status from sync or the backend. Synced means a request got
    /// through, so it also ends Offline.
    pub fn note(&mut self, status: SyncStatus) {
        match status {
            SyncStatus::Offline { waiting } => {
                self.offline = true;
                self.queued = waiting;
            }
            SyncStatus::AuthExpired => self.auth_expired = true,
            SyncStatus::Waiting => self.waiting_for_gmail = true,
            status => {
                if matches!(status, SyncStatus::Synced { .. }) {
                    self.offline = false;
                    self.probe_failures = 0;
                }
                self.base = Some(status);
            }
        }
    }

    /// What the status line says now.
    pub fn shown(&self) -> SyncStatus {
        if self.auth_expired {
            SyncStatus::AuthExpired
        } else if self.offline {
            SyncStatus::Offline {
                waiting: self.queued,
            }
        } else if self.waiting_for_gmail {
            SyncStatus::Waiting
        } else {
            self.base
                .clone()
                .unwrap_or(SyncStatus::Syncing { done: 0, total: 0 })
        }
    }

    /// A probe's answer. True when it ended Offline.
    pub fn probed(&mut self, connected: bool) -> bool {
        if connected {
            self.probe_failures = 0;
            return std::mem::take(&mut self.offline);
        }
        self.probe_failures += 1;
        if self.probe_failures >= 2 {
            self.offline = true;
        }
        false
    }

    /// When to probe next.
    pub fn probe_wait(&self, active: bool) -> Duration {
        if self.offline {
            let backoff = 15u64 << self.probe_failures.saturating_sub(2).min(3);
            Duration::from_secs(backoff.min(120))
        } else if self.probe_failures > 0 {
            PROBE_RECHECK
        } else if active {
            PROBE_ACTIVE
        } else {
            PROBE_INACTIVE
        }
    }
}

/// Whether Google can be reached: a TCP connect, no request.
fn probe() -> bool {
    if http::simulating_offline() {
        return false;
    }
    let Ok(addrs) = PROBE_HOST.to_socket_addrs() else {
        return false;
    };
    addrs
        .take(2)
        .any(|addr| TcpStream::connect_timeout(&addr, PROBE_TIMEOUT).is_ok())
}

impl Backend {
    /// Google mode: start probing, and show the last sync from the store
    /// until this launch's first one.
    pub(super) fn start_health(&mut self, cx: &mut Context<Self>) {
        if self.source != Source::Google {
            return;
        }
        if self.sync.full_done
            && self.status == (SyncStatus::Syncing { done: 0, total: 0 })
            && let Ok(Some(at)) = self.store.sync_state(self.email(), LAST_SYNC_KEY)
            && let Ok(at) = at.parse()
        {
            self.health.note(SyncStatus::Synced { at });
            self.status = self.health.shown();
        }
        self.health._probe = Some(cx.spawn(async move |this, cx| {
            loop {
                let Ok(wait) = this.update(cx, |this, _| this.health.probe_wait(this.sync.active))
                else {
                    return;
                };
                cx.background_executor().timer(wait).await;
                let connected = cx.background_executor().spawn(async { probe() }).await;
                if this
                    .update(cx, |this, cx| this.probed(connected, cx))
                    .is_err()
                {
                    return;
                }
            }
        }));
    }

    fn probed(&mut self, connected: bool, cx: &mut Context<Self>) {
        if self.health.probed(connected) {
            // Back: sync, and send what's queued.
            self.run_sync(cx);
            self.schedule(cx);
        }
        self.refresh_status(cx);
    }

    /// Recount the queue and look at the transport's backoff; also tells
    /// "Synced N min ago" again once a minute.
    pub(super) fn refresh_status(&mut self, cx: &mut Context<Self>) {
        self.health.queued = self.outbox.waiting();
        self.health.waiting_for_gmail = self
            .google
            .as_ref()
            .is_some_and(|g| g.transport.waiting_for_gmail());
        let shown = self.health.shown();
        let minute = match shown {
            SyncStatus::Synced { at } => (super::now_ms() - at) / 60_000,
            _ => -1,
        };
        if shown != self.status || minute != self.health.told_minute {
            self.health.told_minute = minute;
            self.status = shown;
            cx.emit(BackendEvent::Status(self.status.clone()));
        }
    }

    /// When the last sync finished (Unix ms), for Settings › Accounts.
    pub fn last_synced(&self) -> Option<i64> {
        if let SyncStatus::Synced { at } = self.status {
            return Some(at);
        }
        self.store
            .sync_state(self.email(), LAST_SYNC_KEY)
            .ok()
            .flatten()?
            .parse()
            .ok()
    }

    /// Quit (2.7): sends still in their undo window go now, and the
    /// returned task ends once nothing due is left to commit, or after
    /// `QUIT_WAIT`. What's left goes at the next launch.
    pub fn flush_on_quit(&mut self, cx: &mut Context<Self>) -> Task<()> {
        self.flush_settings();
        let now = super::now_ms();
        if let Err(e) = self.outbox.send_now(now) {
            eprintln!("Pigeon: couldn't hurry the queued sends: {e:#}");
        }
        self.schedule(cx);
        cx.spawn(async move |this, cx| {
            let started = std::time::Instant::now();
            while started.elapsed() < QUIT_WAIT {
                let busy = this
                    .update(cx, |this, _| {
                        this.committing
                            || this
                                .outbox
                                .next_due()
                                .is_some_and(|due| due <= super::now_ms())
                    })
                    .unwrap_or(false);
                if !busy {
                    return;
                }
                cx.background_executor()
                    .timer(Duration::from_millis(50))
                    .await;
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn syncing_synced_offline_synced() {
        let mut h = Health::new(SyncStatus::Syncing { done: 0, total: 0 });
        h.note(SyncStatus::Syncing {
            done: 10,
            total: 50,
        });
        assert_eq!(
            h.shown(),
            SyncStatus::Syncing {
                done: 10,
                total: 50
            }
        );
        h.note(SyncStatus::Synced { at: 1 });
        assert_eq!(h.shown(), SyncStatus::Synced { at: 1 });

        // One failed probe checks again soon; the second is Offline.
        assert!(!h.probed(false));
        assert_eq!(h.probe_wait(true), PROBE_RECHECK);
        assert_eq!(h.shown(), SyncStatus::Synced { at: 1 });
        assert!(!h.probed(false));
        h.queued = 3;
        assert_eq!(h.shown(), SyncStatus::Offline { waiting: 3 });
        // Progress while offline doesn't hide it.
        h.note(SyncStatus::Syncing { done: 1, total: 2 });
        assert_eq!(h.shown(), SyncStatus::Offline { waiting: 3 });

        // Offline, probes back off: 15 s, then up to 2 min.
        assert_eq!(h.probe_wait(true), Duration::from_secs(15));
        h.probed(false);
        assert_eq!(h.probe_wait(true), Duration::from_secs(30));
        for _ in 0..5 {
            h.probed(false);
        }
        assert_eq!(h.probe_wait(true), Duration::from_secs(120));

        // A probe that connects ends it, and so does a sync.
        assert!(h.probed(true));
        assert_eq!(h.shown(), SyncStatus::Syncing { done: 1, total: 2 });
        assert_eq!(h.probe_wait(true), PROBE_ACTIVE);
        assert_eq!(h.probe_wait(false), PROBE_INACTIVE);
        h.note(SyncStatus::Offline { waiting: 0 });
        h.note(SyncStatus::Synced { at: 2 });
        assert_eq!(h.shown(), SyncStatus::Synced { at: 2 });
    }

    #[test]
    fn auth_expired_and_waiting_for_gmail() {
        let mut h = Health::new(SyncStatus::Synced { at: 1 });
        h.waiting_for_gmail = true;
        assert_eq!(h.shown(), SyncStatus::Waiting);
        h.waiting_for_gmail = false;
        assert_eq!(h.shown(), SyncStatus::Synced { at: 1 });
        // An expired sign-in wins over everything, Offline too.
        h.note(SyncStatus::Offline { waiting: 1 });
        h.note(SyncStatus::AuthExpired);
        h.note(SyncStatus::Synced { at: 2 });
        assert_eq!(h.shown(), SyncStatus::AuthExpired);
    }
}
