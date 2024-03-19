use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use sans_io_runtime::backend::PollBackend;
use sans_io_runtime::Owner;
use sans_io_runtime::{Controller, WorkerInner, WorkerInnerInput, WorkerInnerOutput};

#[derive(Clone)]
struct ExtIn {
    ts: Instant,
    _data: [u8; 1500],
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
}

impl WorkerInner<ExtIn, ExtOut, ChannelId, Event, ICfg, SCfg> for EchoWorker {
    fn build(worker: u16, _cfg: EchoWorkerCfg) -> Self {
        Self {
            worker,
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
    fn on_tick<'a>(
        &mut self,
        _now: Instant,
    ) -> Option<WorkerInnerOutput<'a, ExtOut, ChannelId, Event, SCfg>> {
        None
    }
    fn on_event<'a>(
        &mut self,
        _now: Instant,
        event: WorkerInnerInput<'a, ExtIn, ChannelId, Event>,
    ) -> Option<WorkerInnerOutput<'a, ExtOut, ChannelId, Event, SCfg>> {
        match event {
            WorkerInnerInput::Ext(ext) => Some(WorkerInnerOutput::Ext(true, ext)),
            _ => None,
        }
    }

    fn pop_output<'a>(
        &mut self,
        _now: Instant,
    ) -> Option<WorkerInnerOutput<'a, ExtOut, ChannelId, Event, SCfg>> {
        None
    }

    fn shutdown<'a>(
        &mut self,
        _now: Instant,
    ) -> Option<WorkerInnerOutput<'a, ExtOut, ChannelId, Event, SCfg>> {
        if self.shutdown {
            return None;
        }
        log::info!("EchoServer {} shutdown", self.worker);
        self.shutdown = true;
        None
    }
}

fn main() {
    env_logger::init();
    let mut controller = Controller::<ExtIn, ExtOut, SCfg, ChannelId, Event, 4096>::default();
    controller.add_worker::<_, EchoWorker, PollBackend<16, 1024>>(EchoWorkerCfg {}, None);
    let term = Arc::new(AtomicBool::new(false));
    signal_hook::flag::register(signal_hook::consts::SIGINT, Arc::clone(&term))
        .expect("Should register hook");

    for i in 0..4000 {
        controller.send_to(
            Owner::worker(0),
            ExtIn {
                ts: Instant::now(),
                _data: [0; 1500],
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
            controller.send_to(Owner::worker(0), out);
        }
    }

    log::info!("Server shutdown");
}
