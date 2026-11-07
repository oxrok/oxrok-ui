//! The app's injectable clock, stored in the egui context so widgets
//! and view animations read the same time that tests control.

use crate::core::{Clock, MonotonicClock};
use egui::{Context, Id};
use std::sync::Arc;
use std::time::Duration;

#[derive(Clone)]
struct ClockHandle(Arc<dyn Clock>);

fn clock_id() -> Id {
    Id::unique("crate::ui::clock")
}

pub fn set_clock(ctx: &Context, clock: Arc<dyn Clock>) {
    ctx.data_mut(|d| d.insert_temp(clock_id(), ClockHandle(clock)));
}

pub fn clock(ctx: &Context) -> Arc<dyn Clock> {
    if let Some(ClockHandle(c)) = ctx.data(|d| d.get_temp::<ClockHandle>(clock_id())) {
        return c;
    }
    let c: Arc<dyn Clock> = Arc::new(MonotonicClock::new());
    set_clock(ctx, c.clone());
    c
}

pub fn now(ctx: &Context) -> Duration {
    clock(ctx).now()
}
