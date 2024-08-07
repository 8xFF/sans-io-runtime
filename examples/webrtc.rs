use std::{
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::Duration,
};

use sans_io_runtime::{backend::PollingBackend, Controller};
use sfu::{ChannelId, ExtIn, ExtOut, ICfg, OwnerType, SCfg, SfuEvent, SfuWorker};
mod http;
mod sfu;

fn main() {
    if std::env::var_os("RUST_LOG").is_none() {
        std::env::set_var("RUST_LOG", "info");
    }
    env_logger::builder().format_timestamp_millis().init();

    let mut server = http::SimpleHttpServer::new(8080);
    let mut controller = Controller::<ExtIn, ExtOut, SCfg, ChannelId, SfuEvent, 128>::default();
    controller.add_worker::<OwnerType, _, SfuWorker, PollingBackend<_, 128, 512>>(
        Duration::from_millis(100),
        ICfg {
            udp_addr: "127.0.0.1:0".parse().unwrap(),
        },
        None,
    );
    controller.add_worker::<OwnerType, _, SfuWorker, PollingBackend<_, 128, 512>>(
        Duration::from_millis(100),
        ICfg {
            udp_addr: "127.0.0.1:0".parse().unwrap(),
        },
        None,
    );

    let mut shutdown_count = 0;
    let term = Arc::new(AtomicBool::new(false));
    signal_hook::flag::register(signal_hook::consts::SIGINT, Arc::clone(&term))
        .expect("Should register hook");

    while let Ok(req) = server.recv(Duration::from_millis(100)) {
        if controller.process().is_none() {
            break;
        }
        if term.load(Ordering::Relaxed) {
            if shutdown_count == 0 {
                controller.shutdown();
            }
            shutdown_count += 1;
            if shutdown_count > 10 {
                log::warn!("Shutdown timeout => force shutdown");
                break;
            }
        }
        while let Some(ext) = controller.pop_event() {
            match ext {
                ExtOut::HttpResponse(resp) => {
                    server.send_response(resp);
                }
            }
        }
        if let Some(req) = req {
            controller.spawn(SCfg::HttpRequest(req));
        }
    }

    log::info!("Server shutdown");
}
