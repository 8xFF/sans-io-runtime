use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::{
    net::SocketAddr,
    time::{Duration, Instant},
};

use sans_io_runtime::backend::{BackendIncoming, BackendOutgoing};
use sans_io_runtime::collections::DynamicDeque;
use sans_io_runtime::{
    backend::PollBackend, Controller, WorkerInner, WorkerInnerInput, WorkerInnerOutput,
};

type ExtIn = ();
type ExtOut = ();
type ChannelId = ();
type Event = ();
type ICfg = EchoWorkerCfg;
type SCfg = ();

type OwnerType = ();

struct EchoWorkerCfg {
    bind: SocketAddr,
}

struct EchoWorker {
    worker: u16,
    backend_slot: usize,
    output: DynamicDeque<WorkerInnerOutput<OwnerType, ExtOut, ChannelId, Event, SCfg>, 16>,
    shutdown: bool,
}

impl WorkerInner<OwnerType, ExtIn, ExtOut, ChannelId, Event, ICfg, SCfg> for EchoWorker {
    fn build(worker: u16, cfg: EchoWorkerCfg) -> Self {
        log::info!("Create new echo task in addr {}", cfg.bind);
        Self {
            worker,
            backend_slot: 0,
            output: DynamicDeque::from([WorkerInnerOutput::Net(
                (),
                BackendOutgoing::UdpListen {
                    addr: cfg.bind,
                    reuse: true,
                },
            )]),
            shutdown: false,
        }
    }

    fn worker_index(&self) -> u16 {
        self.worker
    }

    fn tasks(&self) -> usize {
        if self.shutdown {
            0
        } else {
            1
        }
    }

    fn spawn(&mut self, _now: Instant, _cfg: SCfg) {}
    fn on_tick(&mut self, _now: Instant) {}
    fn on_event(
        &mut self,
        _now: Instant,
        event: WorkerInnerInput<OwnerType, ExtIn, ChannelId, Event>,
    ) {
        match event {
            WorkerInnerInput::Net(_owner, BackendIncoming::UdpListenResult { bind, result }) => {
                log::info!("UdpListenResult: {} {:?}", bind, result);
                self.backend_slot = result.expect("Should bind success").1;
            }
            WorkerInnerInput::Net(_owner, BackendIncoming::UdpPacket { from, slot, data }) => {
                assert!(data.len() <= 1500, "data too large");
                self.output.push_back(WorkerInnerOutput::Net(
                    (),
                    BackendOutgoing::UdpPacket {
                        slot,
                        to: from,
                        data,
                    },
                ));
            }
            _ => {}
        }
    }

    fn pop_output(
        &mut self,
        _now: Instant,
    ) -> Option<WorkerInnerOutput<OwnerType, ExtOut, ChannelId, Event, SCfg>> {
        self.output.pop_front()
    }

    fn on_shutdown(&mut self, _now: Instant) {
        if self.shutdown {
            return;
        }
        log::info!("EchoServer {} shutdown", self.worker);
        self.shutdown = true;
        self.output.push_back(WorkerInnerOutput::Net(
            (),
            BackendOutgoing::UdpUnlisten {
                slot: self.backend_slot,
            },
        ));
    }
}

fn main() {
    env_logger::init();
    let mut controller = Controller::<ExtIn, ExtOut, SCfg, ChannelId, Event, 1024>::default();
    controller.add_worker::<OwnerType, _, EchoWorker, PollBackend<_, 16, 1024>>(
        Duration::from_secs(1),
        EchoWorkerCfg {
            bind: SocketAddr::from(([127, 0, 0, 1], 10001)),
        },
        None,
    );
    controller.add_worker::<OwnerType, _, EchoWorker, PollBackend<_, 16, 1024>>(
        Duration::from_secs(1),
        EchoWorkerCfg {
            bind: SocketAddr::from(([127, 0, 0, 1], 10002)),
        },
        None,
    );
    let term = Arc::new(AtomicBool::new(false));
    signal_hook::flag::register(signal_hook::consts::SIGINT, Arc::clone(&term))
        .expect("Should register hook");

    while controller.process().is_some() {
        if term.load(Ordering::Relaxed) {
            controller.shutdown();
        }
        std::thread::sleep(Duration::from_millis(10));
    }

    log::info!("Server shutdown");
}
