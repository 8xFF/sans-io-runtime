use std::{
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::Duration,
};

use sans_io_runtime::{backend::MioBackend, Controller};
use sfu::{ChannelId, ExtIn, ExtOut, ICfg, SCfg, SfuEvent, SfuWorker};
mod http;
mod sfu;

fn main() {
    if std::env::var_os("RUST_LOG").is_none() {
        std::env::set_var("RUST_LOG", "info");
    }
    env_logger::init();

    let mut server = http::SimpleHttpServer::new(8080);
    let mut controller = Controller::<ExtIn, ExtOut, SCfg, ChannelId, SfuEvent, 128>::default();
    controller.add_worker::<_, SfuWorker, MioBackend<128, 512>>(
        ICfg {
            udp_addr: "192.168.1.39:0".parse().unwrap(),
        },
        None,
    );
    controller.add_worker::<_, SfuWorker, MioBackend<128, 512>>(
        ICfg {
            udp_addr: "192.168.1.39:0".parse().unwrap(),
        },
        None,
    );

    let term = Arc::new(AtomicBool::new(false));
    signal_hook::flag::register(signal_hook::consts::SIGINT, Arc::clone(&term))
        .expect("Should register hook");

    while let Ok(req) = server.recv(Duration::from_micros(100)) {
        if controller.process().is_none() {
            break;
        }
        if term.load(Ordering::Relaxed) {
            controller.shutdown();
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
