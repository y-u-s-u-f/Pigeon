//! Profiling mode, for one headless run (polish plan): `PIGEON_PROFILE=1`
//! with `PIGEON_DEMO=1` runs a fixed sequence on the demo data with no
//! input, logs each scenario's frame times, writes them to
//! `$PIGEON_DATA_DIR/profile.txt` and quits. Without `PIGEON_PROFILE=1`
//! nothing here runs and the hooks are one atomic load.
//!
//! What a frame's time is: from the start of `PigeonApp::render` to the
//! paint of a marker drawn last in the root view. That covers the root's
//! render, the renders of the child views it holds (calendar, composer,
//! palette), taffy layout, prepaint and paint of the main window's tree.
//! It leaves out gpui-base's `Root` around it, deferred draws (popovers,
//! painted after the tree), the accessibility update, and building and
//! presenting the Metal scene. "Root render" is the `PigeonApp::render` call
//! alone. Only frames the main window draws count, so a scenario's frame
//! count is how many it took, animations included.

use std::{
    cell::RefCell,
    fmt::Write as _,
    sync::atomic::{AtomicBool, Ordering},
    time::{Duration, Instant},
};

use gpui::{
    AnyWindowHandle, App, AsyncApp, Context, IntoElement, Keystroke, Styled as _, WeakEntity,
    Window, canvas,
};

use super::{Mode, PigeonApp, Row};
use crate::{
    actions::*,
    data::Body,
    logging,
    ui::calendar::{CalendarView, ViewKind},
};

static ACTIVE: AtomicBool = AtomicBool::new(false);
/// The sequence takes about 30 s; past this the process exits.
const WATCHDOG: Duration = Duration::from_secs(120);

#[derive(Default)]
struct Recorder {
    /// The scenario frames count toward; none between scenarios.
    scenario: Option<&'static str>,
    started: Option<Instant>,
    root_render: Option<Duration>,
    /// Per scenario, in order: (frame, root render) times.
    samples: Vec<(&'static str, Vec<(Duration, Duration)>)>,
}

thread_local! {
    static RECORDER: RefCell<Recorder> = RefCell::new(Recorder::default());
}

pub fn active() -> bool {
    ACTIVE.load(Ordering::Relaxed)
}

/// At the start of `PigeonApp::render`.
pub fn frame_start() {
    if active() {
        RECORDER.with(|r| r.borrow_mut().started = Some(Instant::now()));
    }
}

/// At the end of `PigeonApp::render`.
pub fn render_end() {
    if active() {
        RECORDER.with(|r| {
            let mut r = r.borrow_mut();
            r.root_render = r.started.map(|s| s.elapsed());
        });
    }
}

/// Painted last in the root view: the frame is done.
pub fn marker() -> impl IntoElement {
    canvas(|_, _, _| {}, |_, _, _, _| frame_end())
        .absolute()
        .size_0()
}

fn frame_end() {
    RECORDER.with(|r| {
        let mut r = r.borrow_mut();
        let (Some(started), Some(name)) = (r.started.take(), r.scenario) else {
            return;
        };
        let sample = (started.elapsed(), r.root_render.take().unwrap_or_default());
        if let Some((_, frames)) = r.samples.last_mut().filter(|(last, _)| *last == name) {
            frames.push(sample);
        }
    });
}

/// Frames count toward `name` from now; every scenario is listed, even one
/// that drew nothing.
fn set_scenario(name: Option<&'static str>) {
    RECORDER.with(|r| {
        let mut r = r.borrow_mut();
        r.scenario = name;
        if let Some(name) = name
            && r.samples.last().is_none_or(|(last, _)| *last != name)
        {
            r.samples.push((name, Vec::new()));
        }
    });
}

/// Start the run, when asked for and on demo data. Once, from
/// `PigeonApp::new`.
pub fn start(demo: bool, window: &Window, cx: &mut Context<PigeonApp>) {
    if std::env::var("PIGEON_PROFILE").as_deref() != Ok("1") {
        return;
    }
    if std::env::var("PIGEON_DEMO").as_deref() != Ok("1") || !demo {
        logging::info("profile: PIGEON_PROFILE=1 needs PIGEON_DEMO=1; not profiling");
        return;
    }
    ACTIVE.store(true, Ordering::Relaxed);
    logging::info("profile: starting");
    // The run is unattended: a hang still ends, and says so.
    std::thread::spawn(|| {
        std::thread::sleep(WATCHDOG);
        let path = crate::backend::data_dir().join("profile.txt");
        let line = format!("timed out after {} s\n", WATCHDOG.as_secs());
        std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .and_then(|mut file| std::io::Write::write_all(&mut file, line.as_bytes()))
            .ok();
        logging::info("profile: timed out");
        std::process::exit(3);
    });
    let main = window.window_handle();
    cx.spawn(async move |this, cx| {
        run(main, &this, cx).await;
        let report = report();
        for line in report.lines() {
            logging::info(&format!("profile: {line}"));
        }
        let path = crate::backend::data_dir().join("profile.txt");
        if let Err(e) = std::fs::write(&path, &report) {
            logging::info(&format!("profile: couldn't write {}: {e}", path.display()));
        }
        cx.update(|cx| cx.quit());
    })
    .detach();
}

// Each step runs from the sequence's own task, a top-level update: never
// inside a render, a paint or another update of `PigeonApp`. What dispatches
// input (keystrokes, actions) or updates a child view runs outside the app's
// update, since their handlers update the app themselves.

/// Run `f` on the app in the main window; nothing once it's gone.
fn step(
    cx: &mut AsyncApp,
    main: AnyWindowHandle,
    app: &WeakEntity<PigeonApp>,
    f: impl FnOnce(&mut PigeonApp, &mut Window, &mut Context<PigeonApp>),
) {
    cx.update(|cx| {
        main.update(cx, |_, window, cx| {
            app.update(cx, |this, cx| f(this, window, cx)).ok();
        })
        .ok();
    });
}

/// Run `f` in the main window, with no view being updated: for input.
fn in_window(cx: &mut AsyncApp, main: AnyWindowHandle, f: impl FnOnce(&mut Window, &mut App)) {
    cx.update(|cx| main.update(cx, |_, window, cx| f(window, cx)).ok());
}

/// Run `f` on the calendar view, outside the app's update.
fn calendar_step(
    cx: &mut AsyncApp,
    main: AnyWindowHandle,
    app: &WeakEntity<PigeonApp>,
    f: impl FnOnce(&mut CalendarView, &mut Window, &mut Context<CalendarView>),
) {
    cx.update(|cx| {
        let Some(calendar) = app.upgrade().map(|app| app.read(cx).calendar.clone()) else {
            return;
        };
        main.update(cx, |_, window, cx| {
            calendar.update(cx, |c, cx| f(c, window, cx))
        })
        .ok();
    });
}

async fn wait(cx: &mut AsyncApp, ms: u64) {
    cx.background_executor()
        .timer(Duration::from_millis(ms))
        .await;
}

/// The fixed sequence: about 30 s.
async fn run(main: AnyWindowHandle, app: &WeakEntity<PigeonApp>, cx: &mut AsyncApp) {
    // Launch settles: bodies, counts, the first frames.
    wait(cx, 2000).await;
    step(cx, main, app, |this, window, cx| {
        this.set_mode(Mode::Mail, window, cx);
        this.go_to(super::Nav::Folder(crate::data::Folder::Inbox), window, cx);
    });
    wait(cx, 500).await;

    set_scenario(Some("open conversations"));
    for n in 0..12 {
        step(cx, main, app, move |this, window, cx| {
            let threads: Vec<usize> = this
                .rows
                .iter()
                .filter_map(|r| match r {
                    Row::Thread(ix) => Some(*ix),
                    _ => None,
                })
                .collect();
            if let Some(&ix) = threads.get(n % threads.len().max(1)) {
                this.select_thread(ix, window, cx);
            }
        });
        wait(cx, 250).await;
    }

    set_scenario(None);
    let mut large = None;
    step(cx, main, app, |this, _, cx| {
        large = this.largest_html_thread(cx)
    });
    set_scenario(Some("open the largest HTML conversation"));
    if let Some(large) = large {
        for _ in 0..3 {
            step(cx, main, app, |this, _, cx| {
                this.selected = None;
                cx.notify();
            });
            wait(cx, 300).await;
            let id = large.clone();
            step(cx, main, app, move |this, window, cx| {
                if let Some(ix) = this.data.thread_ix(&id) {
                    this.reveal_thread(ix, window, cx);
                }
            });
            wait(cx, 700).await;
        }
    }

    set_scenario(Some(
        "re-render with a conversation open (hover-like notify)",
    ));
    for _ in 0..120 {
        step(cx, main, app, |_, _, cx| cx.notify());
        wait(cx, 16).await;
    }

    set_scenario(Some("sidebar toggle"));
    for _ in 0..6 {
        step(cx, main, app, |this, window, cx| {
            this.toggle_sidebar(&ToggleSidebar, window, cx)
        });
        wait(cx, 450).await;
    }

    set_scenario(Some("palette open, filter, close"));
    for _ in 0..4 {
        step(cx, main, app, |this, window, cx| {
            this.toggle_command_palette(&ToggleCommandPalette, window, cx)
        });
        wait(cx, 200).await;
        for key in ["a", "r", "c", "h"] {
            in_window(cx, main, move |window, cx| {
                if let Ok(keystroke) = Keystroke::parse(key) {
                    window.dispatch_keystroke(keystroke, cx);
                }
            });
            wait(cx, 80).await;
        }
        step(cx, main, app, |this, window, cx| {
            this.close_overlays(window, cx)
        });
        wait(cx, 200).await;
    }

    set_scenario(Some("switch to Calendar week"));
    step(cx, main, app, |this, window, cx| {
        this.set_mode(Mode::Calendar, window, cx)
    });
    calendar_step(cx, main, app, |c, _, cx| c.set_view(ViewKind::Week, cx));
    wait(cx, 800).await;
    for _ in 0..30 {
        calendar_step(cx, main, app, |_, _, cx| cx.notify());
        wait(cx, 16).await;
    }

    set_scenario(Some("calendar period steps"));
    for forward in [true, false] {
        for _ in 0..8 {
            in_window(cx, main, move |window, cx| {
                if forward {
                    window.dispatch_action(Box::new(NextPeriod), cx);
                } else {
                    window.dispatch_action(Box::new(PrevPeriod), cx);
                }
            });
            wait(cx, 150).await;
        }
    }

    set_scenario(Some("event panel open and close (pane spring)"));
    for n in 0..4 {
        calendar_step(cx, main, app, move |c, window, cx| {
            let occurrences = c.visible_occurrences();
            if let Some(o) = occurrences.get(n % occurrences.len().max(1)) {
                let id = c.data.events[o.event].id.clone();
                c.open_event(&id, o.start, false, window, cx);
            }
        });
        wait(cx, 600).await;
        calendar_step(cx, main, app, |c, window, cx| c.close_panel(window, cx));
        wait(cx, 600).await;
    }
    set_scenario(None);
}

impl PigeonApp {
    /// The conversation whose stored messages hold the most HTML, by id.
    /// Read from the store, so the open still loads its bodies (MA-072).
    fn largest_html_thread(&self, cx: &mut Context<Self>) -> Option<String> {
        let ids: Vec<String> = self.data.threads.iter().map(|t| t.id.clone()).collect();
        let size = |t: &crate::data::Thread| -> usize {
            t.messages
                .iter()
                .map(|m| match &m.body {
                    Body::Loaded { html } => html.len(),
                    _ => 0,
                })
                .sum()
        };
        let (id, bytes) = self
            .backend
            .read(cx)
            .load_threads(&ids)
            .iter()
            .map(|t| (t.id.clone(), size(t)))
            .max_by_key(|(_, bytes)| *bytes)?;
        logging::info(&format!(
            "profile: largest HTML conversation: {id}, {bytes} bytes of HTML"
        ));
        Some(id)
    }
}

/// Each scenario's frame count and p50, p95 and max, in ms.
fn report() -> String {
    let mut out = String::from(
        "Frame = PigeonApp::render start → root marker paint (root render, child view \
         renders, layout, prepaint, paint). Root render = PigeonApp::render alone.\n",
    );
    RECORDER.with(|r| {
        for (name, frames) in &r.borrow().samples {
            // Zeros would read as fast.
            if frames.is_empty() {
                writeln!(out, "{name}: 0 frames: not measured").ok();
                continue;
            }
            let ms = |d: &Duration| d.as_secs_f64() * 1000.;
            let total: Vec<f64> = frames.iter().map(|(f, _)| ms(f)).collect();
            let root: Vec<f64> = frames.iter().map(|(_, r)| ms(r)).collect();
            let (t, rr) = (stats(total), stats(root));
            writeln!(
                out,
                "{name}: {} frames; frame p50 {:.2} ms, p95 {:.2} ms, max {:.2} ms; \
                 root render p50 {:.2} ms, p95 {:.2} ms, max {:.2} ms",
                frames.len(),
                t.0,
                t.1,
                t.2,
                rr.0,
                rr.1,
                rr.2
            )
            .ok();
        }
    });
    out
}

/// p50, p95 and max; zeros when empty.
fn stats(mut values: Vec<f64>) -> (f64, f64, f64) {
    if values.is_empty() {
        return (0., 0., 0.);
    }
    values.sort_by(f64::total_cmp);
    let at = |p: f64| values[((values.len() - 1) as f64 * p).round() as usize];
    (at(0.5), at(0.95), values[values.len() - 1])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn percentiles() {
        assert_eq!(stats(Vec::new()), (0., 0., 0.));
        let values: Vec<f64> = (1..=100).map(f64::from).collect();
        assert_eq!(stats(values), (51., 95., 100.));
        assert_eq!(stats(vec![3.]), (3., 3., 3.));
    }
}
