//! Siri Remote MultitouchSupport frames → pointer motion; HID digitizer fallback.
use crate::{actions::mouse, mapping::engine::TouchpadConfig, AppState};
use std::sync::{mpsc::{channel, Sender, RecvTimeoutError}, OnceLock};
use std::time::{Duration, Instant};
use tauri::Manager;

const POST_INTERVAL: Duration = Duration::from_millis(8);
const TAP_DURATION: Duration = Duration::from_millis(300);
const TAP_TRAVEL: f64 = 12.0;
enum Input {
    Hid(String, i64),
    Frame { device: u64, count: i32, x: f64, y: f64 },
}
static TP_TX: OnceLock<Sender<Input>> = OnceLock::new();
static MOVES: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
unsafe extern "C" {
    fn wc_multitouch_start(callback: extern "C" fn(u64, i32, f64, f64));
    fn wc_multitouch_devices() -> i32;
    fn wc_multitouch_error() -> i32;
    fn wc_multitouch_frames() -> u64;
}
extern "C" fn multitouch_frame(device: u64, count: i32, x: f64, y: f64) {
    if let Some(tx) = TP_TX.get() { let _ = tx.send(Input::Frame { device, count, x, y }); }
}

pub fn diagnostics() -> serde_json::Value {
    serde_json::json!({
        "source": "MultitouchSupport",
        "devices": unsafe { wc_multitouch_devices() },
        "error": unsafe { wc_multitouch_error() },
        "frames": unsafe { wc_multitouch_frames() },
        "pointerMoves": MOVES.load(std::sync::atomic::Ordering::Relaxed),
    })
}

pub fn push(name: &str, value: i64) {
    if let Some(tx) = TP_TX.get() { let _ = tx.send(Input::Hid(name.into(), value)); }
}

#[derive(Default)]
struct Gesture {
    count: Option<i64>,
    tip: Option<bool>,
    started: Option<Instant>,
    xy: [Option<i64>; 2],
    pending: (f64, f64),
    travel: f64,
    cancelled: bool,
    button: bool,
}

impl Gesture {
    fn frame(&mut self, count: i32, x: f64, y: f64, now: Instant) -> bool {
        if count < 0 || !x.is_finite() || !y.is_finite() {
            *self = Self::default();
            return false;
        }
        let tap = self.sample("touchContact", count as i64, now);
        if count == 1 {
            self.sample("touchX", (x * 1000.0).round() as i64, now);
            self.sample("touchY", ((1.0 - y) * 1000.0).round() as i64, now);
        }
        tap
    }

    fn flush(&mut self) -> (f64, f64) { std::mem::take(&mut self.pending) }

    // Each axis establishes its own baseline. Never fabricate a zero coordinate.
    fn sample(&mut self, name: &str, value: i64, now: Instant) -> bool {
        if name == "touchReset" { *self = Self::default(); return false; }
        if name == "touchButton" {
            self.button = value != 0;
            if self.button { self.cancelled = true; }
            return false;
        }
        match name {
            "touchContact" => self.count = Some(value),
            "touchTip" => self.tip = Some(value != 0),
            "touchX" | "touchY" => {
                if self.started.is_none() { return false; }
                let axis = usize::from(name == "touchY");
                let previous = self.xy[axis].replace(value);
                if let Some(previous) = previous {
                    let delta = value as f64 - previous as f64;
                    self.travel += delta.abs();
                    if !self.cancelled {
                        if axis == 0 { self.pending.0 += delta; }
                        else { self.pending.1 += delta; }
                    }
                }
                return false;
            }
            _ => return false,
        }
        // An explicit release from either available signal takes precedence.
        let touching = self.count.unwrap_or(1) > 0 && self.tip.unwrap_or(true)
            && (self.count.is_some() || self.tip.is_some());
        if touching {
            if self.started.is_none() {
                self.started = Some(now);
                self.xy = [None; 2];
                self.travel = 0.0;
                self.cancelled = self.button;
            }
            if self.count.unwrap_or(1) > 1 {
                // Cancel the whole gesture: resuming with another finger would jump.
                self.cancelled = true;
                self.pending = (0.0, 0.0);
            }
            false
        } else {
            let tap = self.started.take().is_some_and(|start|
                now.duration_since(start) < TAP_DURATION && self.travel < TAP_TRAVEL
                    && self.xy.iter().all(Option::is_some) && !self.cancelled);
            self.xy = [None; 2];
            tap
        }
    }
}

pub fn start(app: tauri::AppHandle) {
    let (tx, rx) = channel();
    if TP_TX.set(tx).is_err() { return; }
    unsafe { wc_multitouch_start(multitouch_frame); }
    std::thread::spawn(move || {
        let mut gesture = Gesture::default();
        let mut owner: Option<(u64, Instant)> = None;
        let mut motion_scale = 1.0;
        let mut previous_config: Option<TouchpadConfig> = None;
        let mut last_post = Instant::now();
        loop {
            let sample = match rx.recv_timeout(POST_INTERVAL) {
                Ok(sample) => Some(sample),
                Err(RecvTimeoutError::Timeout) => None,
                Err(RecvTimeoutError::Disconnected) => break,
            };
            let cfg = app.state::<AppState>().engine.lock().unwrap().config.touchpad.clone();
            if previous_config.as_ref() != Some(&cfg) {
                gesture = Gesture::default();
                previous_config = Some(cfg.clone());
            }
            if !cfg.enabled || !mouse_enabled() { gesture = Gesture::default(); continue; }
            let now = Instant::now();
            let tap = match sample {
                Some(Input::Frame { device, count, x, y }) => {
                    if owner.is_some_and(|(id, at)| id != device && now.duration_since(at) < Duration::from_millis(250)) {
                        continue;
                    }
                    if count > 0 && owner.map(|(id, _)| id) != Some(device) {
                        gesture = Gesture { button: gesture.button, ..Gesture::default() };
                    }
                    owner = if count > 0 { Some((device, now)) } else { None };
                    motion_scale = 0.1;
                    // Normalize to 1000 units across the surface; invert Y for screen coordinates.
                    gesture.frame(count, x, y, now)
                }
                Some(Input::Hid(name, value)) => {
                    if name == "touchReset" { owner = None; }
                    if name == "touchButton" || name == "touchReset" || unsafe { wc_multitouch_devices() } == 0 {
                        if name == "touchX" || name == "touchY" { motion_scale = 1.0; }
                        gesture.sample(&name, value, now)
                    } else { false }
                }
                None => false,
            };
            // Flush on timeout and lift as well as incoming samples; never lose the tail.
            if now.duration_since(last_post) >= POST_INTERVAL || gesture.started.is_none() {
                let (dx, dy) = gesture.flush();
                if dx != 0.0 || dy != 0.0 { mouse::move_by(dx * cfg.gain * motion_scale, dy * cfg.gain * motion_scale);
                    MOVES.fetch_add(1, std::sync::atomic::Ordering::Relaxed); }
                last_post = now;
            }
            if tap && cfg.tap_click { mouse::left_click(); }
        }
    });
}

fn mouse_enabled() -> bool { crate::actions::keyboard::accessibility_granted() }

#[cfg(test)]
mod tests {
    use super::*;
    fn feed(g: &mut Gesture, name: &str, value: i64) -> bool { g.sample(name, value, Instant::now()) }
    #[test]
    fn multitouch_frames_move_right_and_up_without_initial_jump() {
        let mut g = Gesture::default();
        let now = Instant::now();
        assert!(!g.frame(1, 0.4, 0.4, now));
        assert_eq!(g.flush(), (0.0, 0.0));
        g.frame(1, 0.6, 0.7, now);
        assert_eq!(g.flush(), (200.0, -300.0));
        assert!(!g.frame(0, 0.0, 0.0, now));
    }
    #[test]
    fn multitouch_disconnect_does_not_tap() {
        let mut g = Gesture::default();
        let now = Instant::now();
        g.frame(1, 0.4, 0.4, now);
        assert!(!g.frame(-1, 0.0, 0.0, now));
        assert!(g.started.is_none());
    }

    #[test]
    fn independent_baselines_and_slow_motion() {
        let mut g = Gesture::default();
        feed(&mut g, "touchTip", 1);
        feed(&mut g, "touchX", 1000); feed(&mut g, "touchY", 2000);
        assert_eq!(g.flush(), (0.0, 0.0));
        feed(&mut g, "touchX", 1001); feed(&mut g, "touchY", 2001);
        assert_eq!(g.flush(), (1.0, 1.0));
    }
    #[test]
    fn lift_preserves_tail_and_next_touch_reanchors() {
        let mut g = Gesture::default();
        feed(&mut g, "touchTip", 1); feed(&mut g, "touchX", 10); feed(&mut g, "touchY", 10);
        feed(&mut g, "touchX", 40);
        assert!(!feed(&mut g, "touchTip", 0));
        assert_eq!(g.flush(), (30.0, 0.0));
        feed(&mut g, "touchTip", 1); feed(&mut g, "touchY", 900); feed(&mut g, "touchX", 900);
        assert_eq!(g.flush(), (0.0, 0.0));
    }
    #[test]
    fn taps_require_coordinates_and_no_physical_click_or_multitouch() {
        for cancel in ["none", "touchButton", "touchContact"] {
            let mut g = Gesture::default();
            feed(&mut g, "touchTip", 1); feed(&mut g, "touchX", 10); feed(&mut g, "touchY", 10);
            feed(&mut g, cancel, 2);
            assert_eq!(feed(&mut g, "touchTip", 0), cancel == "none");
            assert!(!feed(&mut g, "touchTip", 0));
        }
    }
    #[test]
    fn long_contact_and_disconnect_never_click() {
        let mut g = Gesture::default();
        let now = Instant::now();
        g.sample("touchTip", 1, now);
        g.sample("touchX", 10, now); g.sample("touchY", 10, now);
        assert!(!g.sample("touchTip", 0, now + Duration::from_millis(400)));
        feed(&mut g, "touchTip", 1); feed(&mut g, "touchX", 10); feed(&mut g, "touchY", 10);
        feed(&mut g, "touchX", 20); feed(&mut g, "touchReset", 0);
        assert_eq!(g.flush(), (0.0, 0.0));
        assert!(!feed(&mut g, "touchTip", 0));
    }

    #[test]
    fn explicit_tip_release_overrides_stale_count() {
        let mut g = Gesture::default();
        feed(&mut g, "touchContact", 1); feed(&mut g, "touchTip", 1);
        feed(&mut g, "touchX", 10); feed(&mut g, "touchY", 10);
        assert!(feed(&mut g, "touchTip", 0));
        assert!(!feed(&mut g, "touchContact", 0));
    }

    #[test]
    fn jitter_counts_toward_tap_travel() {
        let mut g = Gesture::default();
        feed(&mut g, "touchTip", 1); feed(&mut g, "touchX", 0); feed(&mut g, "touchY", 0);
        for i in 0..30 { feed(&mut g, "touchX", i % 2); }
        assert!(!feed(&mut g, "touchTip", 0));
    }
}
