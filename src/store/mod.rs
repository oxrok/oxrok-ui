//! Sample storage: digital transition chunks with a rank index
//! analog samples with a min/max pyramid, and
//! logic-packet unpacking.
//!

pub mod analog;
pub mod digital;
pub mod unpack;

pub use analog::AnalogChannel;
pub use digital::{BlockSummary, ColumnClass, ColumnRun, DigitalChannel, Transitions, merge_runs};
pub use unpack::{IngestError, LogicIngest};
