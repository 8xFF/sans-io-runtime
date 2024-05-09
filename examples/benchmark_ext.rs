use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use sans_io_runtime::backend::PollingBackend;
use sans_io_runtime::collections::DynamicDeque;
use sans_io_runtime::{
    group_owner_type, Controller, WorkerInner, WorkerInnerInput, WorkerInnerOutput,
};

group_owner_type!(SimpleOwner);

#[derive(Clone)]
struct ExtIn {
    ts: Instant,
    _data: [u8; 10],
}
type ExtOut = ExtIn;
type ChannelId = ();
type Event = ();
type ICfg = EchoWorkerCfg;
type SCfg = ();

struct EchoWorkerCfg {}

struct EchoWorker {
    worker: u16,
    shutdown: bool,
    queue: DynamicDeque<WorkerInnerOutput<SimpleOwner, ExtOut, ChannelId, Event, SCfg>, 16>,
}

impl WorkerInner<SimpleOwner, ExtIn, ExtOut, ChannelId, Event, ICfg, SCfg> for EchoWorker {
    fn build(worker: u16, _cfg: EchoWorkerCfg) -> Self {
        Self {
            worker,
            shutdown: false,
            queue: Default::default(),
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
        event: WorkerInnerInput<SimpleOwner, ExtIn, ChannelId, Event>,
    ) {
        if let WorkerInnerInput::Ext(ext) = event {
            self.queue.push_back(WorkerInnerOutput::Ext(true, ext));
        }
    }

    fn pop_output(
        &mut self,
        _now: Instant,
    ) -> Option<WorkerInnerOutput<SimpleOwner, ExtOut, ChannelId, Event, SCfg>> {
        self.queue.pop_front()
    }

    fn on_shutdown(&mut self, _now: Instant) {
        log::info!("EchoServer {} shutdown", self.worker);
        self.shutdown = true;
    }
}

fn main() {
    env_logger::init();
    let mut controller = Controller::<ExtIn, ExtOut, SCfg, ChannelId, Event, 512>::default();
    controller.add_worker::<SimpleOwner, _, EchoWorker, PollingBackend<_, 16, 16>>(
        Duration::from_secs(1),
        EchoWorkerCfg {},
        None,
    );
    controller.add_worker::<SimpleOwner, _, EchoWorker, PollingBackend<_, 16, 16>>(
        Duration::from_secs(1),
        EchoWorkerCfg {},
        None,
    );
    let term = Arc::new(AtomicBool::new(false));
    signal_hook::flag::register(signal_hook::consts::SIGINT, Arc::clone(&term))
        .expect("Should register hook");

    for _ in 0..400 {
        controller.send_to(
            0,
            ExtIn {
                ts: Instant::now(),
                _data: [0; 10],
            },
        );
    }

    let mut count: u64 = 0;
    let mut last_measure = Instant::now();
    let mut sum_wait_us = 0;
    const CYCLE: u64 = 2000000;

    while controller.process().is_some() {
        if term.load(Ordering::Relaxed) {
            controller.shutdown();
        }
        std::thread::sleep(Duration::from_micros(200));
        while let Some(mut out) = controller.pop_event() {
            sum_wait_us += out.ts.elapsed().as_micros() as u64;
            count += 1;
            if count % CYCLE == 0 {
                let elapsed_ms = last_measure.elapsed().as_millis() as u64;
                last_measure = Instant::now();
                log::info!(
                    "received: {} mps, avg wait: {} us",
                    CYCLE * 1000 / elapsed_ms,
                    sum_wait_us / CYCLE,
                );
                sum_wait_us = 0;
            }
            //log::info!("received after: {}", out.elapsed().as_millis());
            out.ts = Instant::now();
            controller.send_to_best(out);
        }
    }

    log::info!("Server shutdown");
}
