#![cfg(feature = "mock")]
//! The trigger icons on the channel rows and Trigger Details.

use egui::{Event, Key, Modifiers, PointerButton, Pos2, Role};
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use oxrok_ui::app::dialogs::FixedDialogs;
use oxrok_ui::app::{AppOptions, LaApp};
use oxrok_ui::backend::{HwTriggerCaps, MockBackend, MockOptions, Streaming, TriggerMatch};
use oxrok_ui::core::capture::{CaptureMode, CaptureState};
use oxrok_ui::core::trigger::{Combine, DigitalTrigger, Hold, Match, StageCount};
use oxrok_ui::core::{CaptureTime, ChannelId, ManualClock};
use std::sync::Arc;
use std::time::{Duration, Instant};

const RATE: u64 = 8_000_000;
const D0: ChannelId = ChannelId::digital(0);
const D1: ChannelId = ChannelId::digital(1);

/// A device trigger with everything oxrok's software trigger has.
fn full() -> HwTriggerCaps {
    use TriggerMatch::{Edge, Falling, One, Rising, Zero};
    HwTriggerCaps {
        matches: vec![Zero, One, Rising, Falling, Edge],
        capture_ratio: true,
        pulse: true,
        max_stages: 0,
        count: true,
        consecutive: true,
        duration: true,
        range: true,
        or: true,
        and: true,
        holdoff: true,
    }
}

/// Levels and single edges, one stage: no "either edge".
fn plain() -> HwTriggerCaps {
    use TriggerMatch::{Falling, One, Rising, Zero};
    HwTriggerCaps {
        matches: vec![Zero, One, Rising, Falling],
        capture_ratio: true,
        max_stages: 1,
        ..HwTriggerCaps::default()
    }
}

struct Rig {
    h: Harness<'static, LaApp>,
    clock: ManualClock,
}

impl Rig {
    fn new(opts: MockOptions) -> Self {
        let clock = ManualClock::new();
        let c = clock.clone();
        let h = Harness::builder().with_size([1280.0, 800.0]).with_step_dt(1.0 / 60.0).build_eframe(move |cc| {
            let options = MockOptions { streaming: Streaming::RealTime(Arc::new(c.clone())), ..opts.clone() };
            LaApp::with_options(
                cc,
                AppOptions {
                    backend: Arc::new(MockBackend::new(options)),
                    clock: Arc::new(c),
                    dialogs: Arc::new(FixedDialogs(None)),
                    settings_path: None,
                    animation_speed: 0.0,
                    scan_sync: true,
                    profiles_dir: None,
                    extra_builtin_profiles: Vec::new(),
                    mock_decoders: cfg!(feature = "mock"),
                },
            )
        });
        let mut r = Rig { h, clock };
        r.h.run();
        r.h.get_by_label("Mock 16 digital + 4 analog").click();
        r.until(|a| a.session().is_some_and(|s| s.caps.is_some()));
        r
    }

    fn until(&mut self, cond: impl Fn(&LaApp) -> bool) {
        let t0 = Instant::now();
        while !cond(self.h.state()) {
            assert!(t0.elapsed() < Duration::from_secs(30), "timed out");
            self.h.step();
            std::thread::sleep(Duration::from_millis(2));
        }
        self.h.run_steps(2);
    }

    fn trigger(&self) -> DigitalTrigger {
        self.h.state().session().unwrap().settings.trigger.clone()
    }

    fn mode(&self) -> CaptureMode {
        self.h.state().session().unwrap().settings.mode
    }

    fn click(&mut self, role: Role, label: &str) {
        self.h.get_by_role_and_label(role, label).click();
        self.h.run_steps(3);
    }

    fn right_click(&mut self, label: &str) {
        let at: Pos2 = self.h.get_by_role_and_label(Role::Button, label).rect().center();
        self.h.hover_at(at);
        for pressed in [true, false] {
            self.h.event(Event::PointerButton {
                pos: at,
                button: PointerButton::Secondary,
                pressed,
                modifiers: Modifiers::NONE,
            });
        }
        self.h.run_steps(4);
    }

    /// Records until the capture ends by itself, advancing the clock.
    fn record_until_done(&mut self) {
        self.h.key_press(Key::Space);
        self.until(|a| a.session().unwrap().state == CaptureState::Recording);
        let t0 = Instant::now();
        while self.h.state().session().unwrap().state != CaptureState::Finished {
            assert!(t0.elapsed() < Duration::from_secs(30), "never stopped");
            self.clock.advance(Duration::from_millis(20));
            self.h.step();
            std::thread::sleep(Duration::from_millis(2));
        }
        self.h.run_steps(3);
    }
}

/// One stage: a click steps through the conditions and turns Trigger
/// mode on; clearing leaves the mode. The app finds the trigger.
#[test]
fn a_click_cycles_the_condition() {
    let mut r = Rig::new(MockOptions::default());
    assert_eq!(r.mode(), CaptureMode::Continuous);
    assert!(r.trigger().is_empty(), "nothing to wait for at first");
    // One column: no stage letter in the names, none above the icons.
    assert!(r.h.query_by_role_and_label(Role::Button, "Trigger A on D0").is_none());
    assert!(r.h.query_by_role_and_label(Role::Button, "Trigger stage A details").is_none());
    let mut seen = Vec::new();
    for _ in 0..5 {
        r.click(Role::Button, "Trigger on D0");
        seen.push(r.trigger().get(0, D0).unwrap());
        assert_eq!(r.mode(), CaptureMode::Triggered);
    }
    assert_eq!(seen, Match::CYCLE);
    r.click(Role::Button, "Trigger on D0");
    assert!(r.trigger().is_empty(), "after the last, none");
    assert_eq!(r.mode(), CaptureMode::Triggered, "clearing leaves the mode");
    // The analog rows have no icon.
    assert!(r.h.query_by_role_and_label(Role::Button, "Trigger on A0").is_none());

    // D0 rising with D1 high: the mock's D1 idles high, D0 rises at 1 ms.
    r.click(Role::Button, "Trigger on D0");
    for _ in 0..4 {
        r.click(Role::Button, "Trigger on D1");
    }
    assert_eq!(r.trigger().stages[0].conditions, [(D0, Match::Rising), (D1, Match::High)]);
    r.h.run_steps(3);
    r.h.snapshot("trigger_icons_one_stage");
    r.h.state_mut().session_mut().unwrap().settings.trigger.post_trigger_seconds = 0.01;
    r.record_until_done();
    let s = r.h.state().session().unwrap();
    let cap = s.capture.as_ref().unwrap().read();
    let t = cap.trigger.expect("the trigger");
    assert_eq!(cap.hw_trigger, None, "found by the app");
    let (d0, d1) = (cap.channel(D0).unwrap(), cap.channel(D1).unwrap());
    assert_eq!((d0.level_at(t - 1), d0.level_at(t), d1.level_at(t)), (Some(false), Some(true), Some(true)));
    assert_eq!(t % (RATE / 1000), 0, "a rising edge of the 1 kHz square");
}

/// While a capture runs the icons show the trigger and take no click,
/// but the one for its details, which can be read and not changed.
#[test]
fn locked_while_recording() {
    let mut r = Rig::new(MockOptions::default());
    r.click(Role::Button, "Trigger on D1");
    r.h.state_mut().session_mut().unwrap().settings.mode = CaptureMode::Continuous;
    r.h.key_press(Key::Space);
    r.until(|a| a.session().unwrap().state == CaptureState::Recording);
    assert!(r.h.get_by_role_and_label(Role::Button, "Trigger on D1").accesskit_node().is_disabled());
    let before = r.trigger();
    r.click(Role::Button, "Trigger on D1");
    assert_eq!(r.trigger(), before);
    r.right_click("Trigger on D1");
    r.h.get_by_label("Trigger Details");
    assert!(r.h.get_by_role_and_label(Role::Button, "Clear Stage").accesskit_node().is_disabled());
    r.click(Role::Button, "Done");
    assert_eq!(r.trigger(), before);
    r.h.key_press(Key::Space);
    r.until(|a| !a.session().unwrap().state.is_busy());
    assert!(!r.h.get_by_role_and_label(Role::Button, "Trigger on D1").accesskit_node().is_disabled());
}

/// A device whose trigger has no "either edge" is not offered it.
#[test]
fn the_cycle_follows_the_device() {
    let mut r = Rig::new(MockOptions { hw_trigger: Some(plain()), ..MockOptions::default() });
    let mut seen = Vec::new();
    for _ in 0..4 {
        r.click(Role::Button, "Trigger on D0");
        seen.push(r.trigger().get(0, D0).unwrap());
    }
    assert_eq!(seen, [Match::Rising, Match::Falling, Match::High, Match::Low]);
    r.click(Role::Button, "Trigger on D0");
    assert!(r.trigger().is_empty());
}

/// Two stages: columns A and B with their letters, the details of a
/// stage on a right click, and the device's trigger finding the second
/// falling edge after a rising one.
#[test]
fn two_stages_and_details() {
    let mut r = Rig::new(MockOptions { hw_trigger: Some(full()), ..MockOptions::default() });
    r.h.get_by_role_and_label(Role::Button, "Trigger stage A details");
    r.h.get_by_role_and_label(Role::Button, "Trigger stage B details");
    r.click(Role::Button, "Trigger A on D0");
    r.click(Role::Button, "Trigger B on D0");
    r.click(Role::Button, "Trigger B on D0");
    assert_eq!((r.trigger().get(0, D0), r.trigger().get(1, D0)), (Some(Match::Rising), Some(Match::Falling)));

    // The row's own menu stays where it was; the icon opens the details.
    r.right_click("Trigger B on D0");
    r.h.get_by_label("Trigger Details");
    assert!(r.h.query_by_label("Rename").is_none(), "not the row menu");
    r.click(Role::CheckBox, "Trigger on the Nth match");
    assert_eq!(r.trigger().stages[1].count, Some(StageCount { n: 2, consecutive: false }), "of stage B");
    assert_eq!(r.trigger().stages[0].count, None);
    // A count and a duration do not go together.
    assert!(r.h.get_by_role_and_label(Role::CheckBox, "Pulse width").accesskit_node().is_disabled());
    r.click(Role::RadioButton, "Stages A and B: A or B");
    assert_eq!(r.trigger().combine, Combine::Or);
    r.click(Role::RadioButton, "Stages A and B: A, then B");
    r.h.run_steps(3);
    r.h.snapshot("trigger_details");
    r.click(Role::Button, "Done");
    assert!(r.h.query_by_label("Trigger Details").is_none());
    r.h.snapshot("trigger_icons_two_stage");

    // The command line says the same.
    let cli = r.h.state().session().unwrap().cli_command().one_line();
    assert!(cli.contains(r#"-t "D0=r;D0=f*2""#), "{cli}");

    // The device arms its trigger after the 4 ms (32000 samples) to keep
    // before it. D0 rises every 1 ms: at sample 32000, and its second
    // fall after is at 44000. The capture is 29 % before the trigger.
    {
        let t = &mut r.h.state_mut().session_mut().unwrap().settings.trigger;
        (t.pre_trigger_seconds, t.post_trigger_seconds) = (0.004, 0.01);
    }
    r.record_until_done();
    let s = r.h.state().session().unwrap();
    let cap = s.capture.as_ref().unwrap().read();
    let t = cap.hw_trigger.expect("the device's trigger");
    assert_eq!(cap.trigger, Some(t), "taken as it is: the scanner cannot tell which fall it was");
    assert_eq!(s.trigger, Some(CaptureTime::from_sample(t, RATE)));
    let (begin, end) = cap.sample_range().unwrap();
    assert!(t - begin >= 32_000, "the time before the trigger is there: {} samples", t - begin);
    assert!(end - t >= 80_000, "the post-trigger time: {end}");
    // A fall of D0, 1.5 ms after a rise that the capture has too.
    let d0 = cap.channel(D0).unwrap();
    assert_eq!((d0.level_at(t - 1), d0.level_at(t)), (Some(true), Some(false)));
    assert_eq!((d0.level_at(t - 12_001), d0.level_at(t - 12_000)), (Some(false), Some(true)));
}

/// Trigger Details from the Device tab: the stage in words, a pulse
/// width for the app to find on a device without a trigger, and what a
/// device without one cannot do, with the reason.
#[test]
fn details_from_the_device_tab() {
    let mut r = Rig::new(MockOptions::default());
    r.click(Role::Button, "Trigger on D0");
    r.click(Role::Button, "Trigger on D0");
    r.h.key_press_modifiers(Modifiers::ALT, Key::Num1);
    r.h.run_steps(3);
    r.h.get_by_label("D0 Falling Edge");
    r.h.get_by_label("The app looks for this trigger in the data.");
    r.h.get_by_role_and_label(Role::Button, "Trigger Details…").scroll_to_me();
    r.h.run_steps(3);
    r.click(Role::Button, "Trigger Details…");
    // No trigger in the device: no count, no value range, no stage B.
    for label in ["Trigger on the Nth match", "Value on a group of channels"] {
        assert!(r.h.get_by_role_and_label(Role::CheckBox, label).accesskit_node().is_disabled(), "{label}");
    }
    assert!(r.h.get_by_role_and_label(Role::RadioButton, "Stage: Stage B").accesskit_node().is_disabled());
    // A pulse width is the app's to find.
    r.click(Role::CheckBox, "Pulse width");
    assert_eq!(r.trigger().stages[0].hold, Some(Hold::Between(1e-3, 10e-3)));
    // In the dialog and behind it on the Device tab.
    assert_eq!(r.h.query_all_by_label("The app looks for this trigger in the data.").count(), 2);
    r.click(Role::Button, "Clear Stage");
    assert!(r.trigger().is_empty());
    r.click(Role::Button, "Done");
}

/// A driver that lists more stages than it arms says so at the start:
/// the error, and what to try.
#[test]
fn a_refused_trigger_says_why() {
    let refuse = "Only one trigger stage supported for now.";
    let options =
        MockOptions { hw_trigger: Some(full()), refuse_trigger: Some(refuse.into()), ..MockOptions::default() };
    let mut r = Rig::new(options);
    r.click(Role::Button, "Trigger A on D0");
    r.click(Role::Button, "Trigger B on D1");
    r.h.key_press(Key::Space);
    r.until(|a| a.toasts().iter().any(|t| t.body.contains(refuse)));
    let toasts = r.h.state().toasts();
    let t = toasts.iter().find(|t| t.body.contains(refuse)).unwrap();
    assert!(t.body.contains("clear stage B"), "{}", t.body);
    assert_eq!(r.h.state().session().unwrap().state, CaptureState::Empty);
}
