//! The device worker: owns every open [`Device`] and runs backend calls off
//! the UI thread. Results come back as events, followed by a
//! repaint request.

use crate::backend::{
    AcquisitionPlan, AppliedConfig, Backend, BackendError, DeviceCaps, DeviceConfig, DeviceId, SampleSink,
};
use crossbeam_channel::{Receiver, Sender, unbounded};
use std::collections::HashMap;
use std::sync::Arc;

pub type SessionId = u64;

pub enum WorkerCmd {
    Open {
        session: SessionId,
        device: DeviceId,
    },
    Start {
        session: SessionId,
        config: DeviceConfig,
        plan: Box<PlanFor>,
        sink: Box<dyn SampleSink>,
    },
    Stop {
        session: SessionId,
    },
    Close {
        session: SessionId,
    },
    /// Fresh option values.
    ReadOptions {
        session: SessionId,
    },
    /// The live pins.
    ReadPins {
        session: SessionId,
        config: DeviceConfig,
    },
}

/// The plan of a capture, made for the settings the device took: its
/// sample rate may not be the one asked for, and the plan counts in
/// samples. An error fails the start: a trigger that cannot be used at
/// that rate.
pub type PlanFor = dyn FnOnce(&AppliedConfig) -> Result<AcquisitionPlan, String> + Send;

/// Why a capture did not start.
#[derive(Clone, Debug, PartialEq)]
pub enum StartError {
    /// The device's own words.
    Device(String),
    /// No plan for the settings the device took.
    Plan(String),
}

#[derive(Debug)]
pub enum WorkerEvent {
    Opened {
        session: SessionId,
        result: Result<DeviceCaps, String>,
    },
    Started {
        session: SessionId,
        result: Result<AppliedConfig, StartError>,
    },
    Stopped {
        session: SessionId,
    },
    Options {
        session: SessionId,
        groups: Vec<crate::backend::OptionGroup>,
    },
    /// `took`: how long the read kept the worker busy (back-off).
    Pins {
        session: SessionId,
        result: Result<Vec<(crate::core::ChannelId, bool)>, String>,
        took: std::time::Duration,
    },
}

/// One open handle per device, shared by the sessions bound
/// to it; it closes with the last of them.
#[derive(Default)]
struct Devices {
    bound: HashMap<SessionId, DeviceId>,
    open: HashMap<DeviceId, (Box<dyn crate::backend::Device>, usize)>,
}

impl Devices {
    fn open(&mut self, backend: &dyn Backend, session: SessionId, id: DeviceId) -> Result<DeviceCaps, String> {
        if self.bound.get(&session) != Some(&id) {
            self.release(session);
            if let Some((_, users)) = self.open.get_mut(&id) {
                *users += 1;
            } else {
                let d = backend.open(&id).map_err(|e| e.to_string())?;
                self.open.insert(id.clone(), (d, 1));
            }
            self.bound.insert(session, id.clone());
        }
        Ok(self.open[&id].0.capabilities().clone())
    }

    fn device(&mut self, session: SessionId) -> Option<&mut Box<dyn crate::backend::Device>> {
        let id = self.bound.get(&session)?;
        self.open.get_mut(id).map(|(d, _)| d)
    }

    fn release(&mut self, session: SessionId) {
        let Some(id) = self.bound.remove(&session) else { return };
        if let Some((_, users)) = self.open.get_mut(&id) {
            *users -= 1;
            if *users == 0 {
                self.open.remove(&id);
            }
        }
    }
}

pub struct Worker {
    tx: Sender<WorkerCmd>,
    rx: Receiver<WorkerEvent>,
}

impl Worker {
    pub fn spawn(backend: Arc<dyn Backend>, repaint: impl Fn() + Send + 'static) -> Self {
        let (tx, cmd_rx) = unbounded::<WorkerCmd>();
        let (ev_tx, rx) = unbounded();
        std::thread::Builder::new()
            .name("device-worker".into())
            .spawn(move || {
                let mut w = Devices::default();
                for cmd in cmd_rx {
                    let ev = match cmd {
                        WorkerCmd::Open { session, device } => {
                            Some(WorkerEvent::Opened { session, result: w.open(&*backend, session, device) })
                        }
                        WorkerCmd::Start { session, config, plan, sink } => {
                            let result = match w.device(session) {
                                None => Err(StartError::Device("device not open".to_owned())),
                                Some(d) => {
                                    let device = |e: BackendError| StartError::Device(e.to_string());
                                    d.apply(&config).map_err(device).and_then(|applied| {
                                        let plan = plan(&applied).map_err(StartError::Plan)?;
                                        d.start(plan, sink).map_err(device)?;
                                        Ok(applied)
                                    })
                                }
                            };
                            Some(WorkerEvent::Started { session, result })
                        }
                        WorkerCmd::Stop { session } => {
                            if let Some(d) = w.device(session) {
                                let _ = d.stop();
                            }
                            Some(WorkerEvent::Stopped { session })
                        }
                        WorkerCmd::Close { session } => {
                            w.release(session);
                            None
                        }
                        WorkerCmd::ReadOptions { session } => {
                            w.device(session).map(|d| WorkerEvent::Options { session, groups: d.read_options() })
                        }
                        WorkerCmd::ReadPins { session, config } => {
                            let t0 = std::time::Instant::now();
                            let result = match w.device(session) {
                                None => Err("device not open".to_owned()),
                                Some(d) => d.read_current_state(&config).map_err(|e| e.to_string()),
                            };
                            Some(WorkerEvent::Pins { session, result, took: t0.elapsed() })
                        }
                    };
                    if let Some(ev) = ev
                        && ev_tx.send(ev).is_ok()
                    {
                        repaint();
                    }
                }
            })
            .expect("spawn device-worker thread");
        Self { tx, rx }
    }

    pub fn send(&self, cmd: WorkerCmd) {
        let _ = self.tx.send(cmd);
    }

    pub fn try_recv(&self) -> Option<WorkerEvent> {
        self.rx.try_recv().ok()
    }
}

#[cfg(all(test, feature = "mock"))]
mod tests {
    use super::*;
    use crate::backend::{BackendError, DecoderInfo, DeviceInfo, MockBackend};
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// Counts the devices open at once.
    struct Counting {
        mock: MockBackend,
        open: Arc<AtomicUsize>,
    }

    struct Tracked(Box<dyn crate::backend::Device>, Arc<AtomicUsize>);

    impl Drop for Tracked {
        fn drop(&mut self) {
            self.1.fetch_sub(1, Ordering::SeqCst);
        }
    }

    impl crate::backend::Device for Tracked {
        fn info(&self) -> &DeviceInfo {
            self.0.info()
        }
        fn capabilities(&self) -> &DeviceCaps {
            self.0.capabilities()
        }
        fn apply(&mut self, cfg: &DeviceConfig) -> Result<AppliedConfig, BackendError> {
            self.0.apply(cfg)
        }
        fn start(&mut self, plan: AcquisitionPlan, sink: Box<dyn SampleSink>) -> Result<(), BackendError> {
            self.0.start(plan, sink)
        }
        fn stop(&mut self) -> Result<(), BackendError> {
            self.0.stop()
        }
    }

    impl Backend for Counting {
        fn scan_devices(&self) -> Vec<DeviceInfo> {
            self.mock.scan_devices()
        }
        fn open(&self, id: &DeviceId) -> Result<Box<dyn crate::backend::Device>, BackendError> {
            let d = self.mock.open(id)?;
            self.open.fetch_add(1, Ordering::SeqCst);
            Ok(Box::new(Tracked(d, self.open.clone())))
        }
        fn decoders(&self) -> &[DecoderInfo] {
            self.mock.decoders()
        }
    }

    /// Sessions on one device share its handle; it closes
    /// with the last of them.
    #[test]
    fn sessions_share_a_device() {
        let open = Arc::new(AtomicUsize::new(0));
        let backend = Counting { mock: MockBackend::seeded(1), open: open.clone() };
        let ids: Vec<DeviceId> = backend.scan_devices().into_iter().map(|d| d.id).collect();
        let mut w = Devices::default();
        assert!(w.open(&backend, 0, ids[0].clone()).is_ok());
        assert!(w.open(&backend, 1, ids[0].clone()).is_ok());
        assert_eq!(open.load(Ordering::SeqCst), 1, "one handle for two sessions");
        assert!(w.device(1).is_some());
        // Rebinding session 1 to another device keeps session 0's open.
        assert!(w.open(&backend, 1, ids[1].clone()).is_ok());
        assert_eq!(open.load(Ordering::SeqCst), 2);
        w.release(0);
        assert_eq!(open.load(Ordering::SeqCst), 1);
        assert!(w.device(0).is_none());
        w.release(1);
        assert_eq!(open.load(Ordering::SeqCst), 0);
    }
}
