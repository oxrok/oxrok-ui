#![cfg(feature = "mock")]
//! Signal groups (buses).

use egui::{Event, Key, Modifiers, PointerButton, Pos2, Role};
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use oxrok_ui::app::dialogs::ScriptedDialogs;
use oxrok_ui::app::{AppOptions, LaApp};
use oxrok_ui::backend::{MockBackend, MockOptions, Streaming};
use oxrok_ui::core::capture::CaptureState;
use oxrok_ui::core::{ChannelId, ManualClock};
use oxrok_ui::store::DigitalChannel;
use std::sync::Arc;
use std::time::{Duration, Instant};

const D: fn(u16) -> ChannelId = ChannelId::digital;
const G1: ChannelId = ChannelId::bus(0);
const SAMPLES: u64 = 80_000;
const INSPECTOR_W: f32 = 300.0;

struct Rig {
    h: Harness<'static, LaApp>,
    clock: ManualClock,
    dialogs: Arc<ScriptedDialogs>,
    dir: tempfile::TempDir,
}

impl Rig {
    fn new() -> Self {
        let clock = ManualClock::new();
        let dir = tempfile::tempdir().unwrap();
        let dialogs = Arc::new(ScriptedDialogs { folder: Some(dir.path().to_owned()), ..Default::default() });
        let (c, d) = (clock.clone(), dialogs.clone());
        let h = Harness::builder().with_size([1280.0, 800.0]).with_step_dt(1.0 / 60.0).build_eframe(move |cc| {
            let options = MockOptions { streaming: Streaming::RealTime(Arc::new(c.clone())), ..MockOptions::default() };
            LaApp::with_options(
                cc,
                AppOptions {
                    backend: Arc::new(MockBackend::new(options)),
                    clock: Arc::new(c),
                    dialogs: d,
                    settings_path: None,
                    animation_speed: 0.0,
                    scan_sync: true,
                    profiles_dir: None,
                    extra_builtin_profiles: Vec::new(),
                    mock_decoders: cfg!(feature = "mock"),
                },
            )
        });
        Rig { h, clock, dialogs, dir }
    }

    fn until(&mut self, what: &str, cond: impl Fn(&LaApp) -> bool) {
        let t0 = Instant::now();
        while !cond(self.h.state()) {
            assert!(t0.elapsed() < Duration::from_secs(30), "timed out waiting for {what}");
            self.h.step();
            std::thread::sleep(Duration::from_millis(2));
        }
    }

    /// D0–D3 enabled, a 10 ms capture, then known levels: D0 and D1 high,
    /// D3 low, D2 rising halfway.
    fn capture(&mut self) {
        self.h.run();
        self.h.get_by_label("Mock 16 digital + 4 analog").click();
        self.until("device opened", |a| a.session().is_some_and(|s| s.caps.is_some()));
        self.h.state_mut().session_mut().unwrap().settings.enabled = (0..4).map(D).collect();
        self.h.key_press(Key::Space);
        self.until("recording", |a| a.session().unwrap().state == CaptureState::Recording);
        self.clock.advance(Duration::from_millis(10));
        self.until("data", |a| a.session().unwrap().has_data());
        self.h.key_press(Key::Space);
        self.until("finished", |a| a.session().unwrap().state == CaptureState::Finished);
        let level = |high: bool, rise: Option<u64>| {
            let mut c = DigitalChannel::new();
            c.start(0, high);
            if let Some(t) = rise {
                c.push_transition(t);
            }
            c.extend_to(SAMPLES);
            c
        };
        {
            let s = self.h.state_mut().session_mut().unwrap();
            let cap = s.capture.as_ref().unwrap().clone();
            let mut d = cap.write();
            d.ids = (0..4).map(D).collect();
            let channels =
                vec![level(true, None), level(true, None), level(false, Some(SAMPLES / 2)), level(false, None)];
            d.ingest = Some(oxrok_ui::store::LogicIngest::from_channels(1, vec![0, 1, 2, 3], channels));
            d.analog.clear();
        }
        self.h.key_press(Key::F);
        self.h.run_steps(5);
    }

    fn label_center(&self, text: &str) -> Pos2 {
        self.h.get_by_role_and_label(Role::Label, text).rect().center()
    }

    /// Right-clicks the row label `label`.
    fn menu_on(&mut self, label: &str) {
        let at = self.label_center(label);
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

    /// A channel check box in the dialog (the Inspector's chips are check
    /// boxes named alike, docked on the left).
    fn check(&mut self, label: &str) {
        let n = self
            .h
            .get_all_by_role_and_label(Role::CheckBox, label)
            .find(|n| (n.rect().center().x > INSPECTOR_W) && (n.rect().center().x < 900.0))
            .expect("dialog check box");
        n.click();
        self.h.run();
    }

    fn click(&mut self, label: &str) {
        self.h.get_by_label(label).click();
        self.h.run_steps(4);
    }

    fn rows(&self) -> Vec<ChannelId> {
        self.h.state().session().unwrap().visible_rows().map(|r| r.channel).collect()
    }

    fn bubbles(&self) -> Vec<String> {
        ["0xC", "0xE", "12", "14", "-4", "-2", "0b1100", "0b1110", "0x3", "0x7", "0x6"]
            .into_iter()
            .filter(|t| self.h.query_by_role_and_label(Role::Label, t).is_some())
            .map(str::to_owned)
            .collect()
    }
}

/// Group Channels… on D1, also selecting D0, D2 and D3: `G1` where
/// D0 was, the members indented under it, Hex bubbles; collapse; move;
/// a member's move changes the wire order.
#[test]
fn group_channels() {
    let mut r = Rig::new();
    r.capture();
    r.menu_on("D1");
    r.h.get_by_role_and_label(Role::MenuItem, "Group Channels…").click();
    r.h.run_steps(4);
    r.h.get_by_label("Group Channels");
    for c in ["D0", "D2", "D3"] {
        r.check(c);
    }
    r.h.snapshot("bus_dialog");
    r.click("Create");
    assert!(r.h.query_by_label("Group Channels").is_none());
    assert_eq!(r.rows(), [G1, D(0), D(1), D(2), D(3)]);
    r.h.get_by_role_and_label(Role::Label, "G1 D0–D3");
    assert_eq!(r.bubbles(), ["0xC", "0xE"]);
    r.h.snapshot("bus_rows");

    // Collapse hides the members; the triangle expands again.
    r.click("Collapse group");
    assert_eq!(r.rows(), [G1]);
    r.click("Expand group");
    assert_eq!(r.rows().len(), 5);

    // D3 dragged above D0 becomes wire 0: 0b0110 → 0x6, 0x7.
    r.h.state_mut().session_mut().unwrap().move_row(D(3), Some(D(0)));
    r.h.run_steps(3);
    assert_eq!(r.rows(), [G1, D(3), D(0), D(1), D(2)]);
    assert_eq!(r.bubbles(), ["0x7", "0x6"]);
}

/// Dragging a chip in the dialog's wire order strip sets the order.
#[test]
fn dialog_wire_order() {
    let mut r = Rig::new();
    r.capture();
    r.menu_on("D0");
    r.h.get_by_role_and_label(Role::MenuItem, "Group Channels…").click();
    r.h.run_steps(4);
    // Checking out of order still gives index order.
    for c in ["D3", "D1", "D2"] {
        r.check(c);
    }
    let chip = |r: &Rig, k: usize, c: &str| r.h.get_by_role_and_label(Role::Button, &format!("Wire {k}: {c}")).rect();
    for (k, c) in ["D0", "D1", "D2", "D3"].into_iter().enumerate() {
        chip(&r, k, c);
    }
    // D3's chip dropped on the left half of D0's.
    let (from, to) = (chip(&r, 3, "D3").center(), chip(&r, 0, "D0").left_center() + egui::vec2(4.0, 0.0));
    r.h.hover_at(from);
    r.h.event(Event::PointerButton {
        pos: from,
        button: PointerButton::Primary,
        pressed: true,
        modifiers: Modifiers::NONE,
    });
    r.h.run_steps(2);
    for k in 1..=6 {
        r.h.hover_at(from + (to - from) * (k as f32 / 6.0));
        r.h.run_steps(2);
    }
    r.h.event(Event::PointerButton {
        pos: to,
        button: PointerButton::Primary,
        pressed: false,
        modifiers: Modifiers::NONE,
    });
    r.h.run_steps(3);
    chip(&r, 0, "D3");
    r.h.get_by_label("Drag to reorder. The first channel is the most significant bit.");
    r.click("Create");
    assert_eq!(r.rows(), [G1, D(3), D(0), D(1), D(2)]);
    assert_eq!(r.h.state().session().unwrap().wire_order(0), [D(3), D(0), D(1), D(2)]);
    r.h.get_by_role_and_label(Role::Label, "G1 D3–D2");
}

/// A one-channel group, named after its channel, with a 1-bit value.
#[test]
fn one_channel_group() {
    let mut r = Rig::new();
    r.capture();
    r.menu_on("D2");
    r.h.get_by_role_and_label(Role::MenuItem, "Group Channels…").click();
    r.h.run_steps(4);
    r.h.get_by_label("Check more channels to combine them into one value.");
    r.click("Create");
    assert_eq!(r.rows(), [D(0), D(1), G1, D(2), D(3)]);
    assert_eq!(r.h.state().session().unwrap().wire_order(0), [D(2)]);
    r.h.get_by_role_and_label(Role::Label, "G1 D2");
    for v in ["0x0", "0x1"] {
        r.h.get_by_role_and_label(Role::Label, v);
    }
}

/// Format, endianness and signed from the row menu.
#[test]
fn format_and_signed() {
    let mut r = Rig::new();
    r.capture();
    r.h.state_mut().session_mut().unwrap().create_bus(
        &(0..4).map(D).collect::<Vec<_>>(),
        "",
        oxrok_ui::core::bus::BusFormat::Hex,
        oxrok_ui::core::bus::Endianness::MsbFirst,
        false,
    );
    r.h.run_steps(3);
    for (format, want) in [("Decimal", ["12", "14"]), ("Binary", ["0b1100", "0b1110"])] {
        r.menu_on("G1 D0–D3");
        r.click("Value Format");
        r.click(format);
        assert_eq!(r.bubbles(), want, "{format}");
    }
    r.menu_on("G1 D0–D3");
    r.click("Value Format");
    r.click("Decimal");
    r.menu_on("G1 D0–D3");
    r.click("Value Format");
    r.click("Signed");
    assert_eq!(r.bubbles(), ["-4", "-2"]);
    // Hex shows the raw pattern even when signed.
    r.menu_on("G1 D0–D3");
    r.click("Value Format");
    r.click("Hex");
    assert_eq!(r.bubbles(), ["0xC", "0xE"]);
    r.menu_on("G1 D0–D3");
    r.click("Value Format");
    r.click("Wire LSB → Data LSB");
    assert_eq!(r.bubbles(), ["0x3", "0x7"]);

    // Edit Group…: drop D3; the default name follows.
    r.menu_on("G1 D0–D3");
    r.click("Edit Group…");
    r.h.get_by_label("Edit Group");
    r.check("D3");
    r.click("Save");
    r.h.get_by_role_and_label(Role::Label, "G1 D0–D2");
    assert_eq!(r.rows(), [G1, D(0), D(1), D(2), D(3)]);
    assert_eq!(r.h.state().session().unwrap().wire_order(0), [D(0), D(1), D(2)]);
}

/// A No Data group has no bubbles and no value options; a group
/// named by the user shows just its name.
#[test]
fn no_data_and_name() {
    let mut r = Rig::new();
    r.capture();
    r.h.state_mut().session_mut().unwrap().create_bus(
        &(0..4).map(D).collect::<Vec<_>>(),
        "",
        oxrok_ui::core::bus::BusFormat::Hex,
        oxrok_ui::core::bus::Endianness::MsbFirst,
        false,
    );
    r.h.run_steps(3);
    assert_eq!(r.bubbles(), ["0xC", "0xE"]);
    r.menu_on("G1 D0–D3");
    r.click("Value Format");
    r.click("No Data");
    assert!(r.bubbles().is_empty());
    r.menu_on("G1 D0–D3");
    r.click("Value Format");
    for item in ["Wire LSB → Data LSB", "Signed"] {
        assert!(r.h.get_by_label(item).accesskit_node().is_disabled(), "{item}");
    }
    r.h.key_press(Key::Escape);
    r.h.run_steps(4);

    r.h.state_mut().session_mut().unwrap().rename_row(G1, "SPI");
    r.h.run_steps(3);
    r.h.get_by_role_and_label(Role::Label, "G1 SPI");
    r.h.snapshot("bus_no_data_named");
}

/// Remove Group leaves the members; a saved capture keeps the group.
#[test]
fn remove_and_persist() {
    let mut r = Rig::new();
    r.capture();
    r.menu_on("D0");
    r.h.get_by_role_and_label(Role::MenuItem, "Group Channels…").click();
    r.h.run_steps(4);
    for c in ["D1", "D2", "D3"] {
        r.check(c);
    }
    r.click("Create");
    r.menu_on("G1 D0–D3");
    r.click("Value Format");
    r.click("Decimal");

    // Save, close, reopen: the group, its members and options are back.
    r.h.key_press_modifiers(Modifiers::COMMAND, Key::S);
    r.until("saved", |a| !a.session().unwrap().is_unsaved());
    let path = r.dir.path().join("Session 0.oxrok");
    assert!(path.exists());
    *r.dialogs.open.lock() = vec![path.clone()];
    r.h.key_press_modifiers(Modifiers::COMMAND, Key::O);
    let p = path.clone();
    r.until("opened", move |a| a.session().is_some_and(|s| s.opened_from.as_deref() == Some(&*p)));
    r.h.run_steps(3);
    assert_eq!(r.rows(), [G1, D(0), D(1), D(2), D(3)]);
    assert_eq!(r.h.state().session().unwrap().bus(0).unwrap().format, oxrok_ui::core::bus::BusFormat::Decimal);
    assert_eq!(r.bubbles(), ["12", "14"]);

    r.menu_on("G1 D0–D3");
    r.click("Remove Group");
    assert_eq!(r.rows(), [D(0), D(1), D(2), D(3)]);
    assert!(r.bubbles().is_empty());
    let s = r.h.state().session().unwrap();
    assert!(s.capture.as_ref().unwrap().read().channel(D(2)).unwrap().transition_count() == 1);
}

/// Wire Status shows the live pins in the row labels and the `G1`
/// value; they follow the mock pattern; off leaves the last state dimmed.
#[test]
fn wire_status() {
    let mut r = Rig::new();
    r.h.run();
    r.h.get_by_label("Mock 16 digital + 4 analog").click();
    r.until("device opened", |a| a.session().is_some_and(|s| s.caps.is_some()));
    {
        let s = r.h.state_mut().session_mut().unwrap();
        s.settings.enabled = vec![D(0), D(1), D(9)];
        s.create_bus(
            &[D(0), D(1)],
            "",
            oxrok_ui::core::bus::BusFormat::Binary,
            oxrok_ui::core::bus::Endianness::MsbFirst,
            false,
        );
    }
    r.h.run();
    // View → Wire Status.
    r.click("View");
    r.h.get_by_role_and_label(Role::MenuItemCheckBox, "Wire Status").click();
    r.h.run_steps(3);
    r.until("pins read", |a| a.session().unwrap().pins.as_ref().is_some_and(|p| !p.stale));
    r.h.run_steps(2);
    let level = |r: &Rig, c: ChannelId| r.h.state().session().unwrap().pins.as_ref().unwrap().level(c).unwrap();
    let word = |l: bool| if l { "high" } else { "low" };
    let d9 = level(&r, D(9));
    r.h.get_by_label(&format!("D9: {}", word(d9)));
    let g1 = format!("0b{}{}", u8::from(level(&r, D(0))), u8::from(level(&r, D(1))));
    r.h.get_by_label(&format!("G1: {g1}"));
    r.h.snapshot("wire_status");

    // D9 toggles every 8 ms of the pattern; reads are 100 ms apart.
    r.clock.advance(Duration::from_millis(13 * 8));
    r.until("next read", move |a| {
        a.session().unwrap().pins.as_ref().is_some_and(|p| p.level(D(9)) == Some(!d9) && !p.stale)
    });
    r.h.run_steps(2);
    r.h.get_by_label(&format!("D9: {}", word(!d9)));

    // Off: no more reads; the last state stays, dimmed.
    r.menu_on("D0");
    r.h.get_by_role_and_label(Role::MenuItemCheckBox, "Wire Status").click();
    r.h.run_steps(3);
    assert!(!r.h.state().session().unwrap().wire_status);
    r.clock.advance(Duration::from_millis(8));
    r.h.run_steps(5);
    r.h.get_by_label(&format!("D9: {} (stale)", word(!d9)));
}
