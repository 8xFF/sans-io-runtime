use std::time::Duration;

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
    let mut controller = Controller::<ExtIn, ExtOut, SCfg, ChannelId, SfuEvent, 128>::new();
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

    while let Ok(req) = server.recv(Duration::from_micros(100)) {
        controller.process();
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
}
