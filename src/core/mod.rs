//! Domain model and pure logic shared by every other crate.
//!
//! No egui, no FFI: everything here is unit-testable on its own.

pub mod analog;
pub mod animator;
pub mod bus;
pub mod capture;
pub mod channel;
pub mod clock;
pub mod device;
pub mod format;
pub mod history;
pub mod layout;
pub mod markers;
pub mod nav;
pub mod probe;
pub mod ranges;
pub mod ticks;
pub mod time;
pub mod trigger;
pub mod view;

pub use channel::{ChannelId, ChannelKind};
pub use clock::{Clock, ManualClock, MonotonicClock};
pub use time::{CaptureTime, FS_PER_S, WallClock};
pub use view::ViewState;
