mod tls;
mod proxy;
mod gui;
mod store;
mod util;

pub use util::*;

pub const ORDERING: std::sync::atomic::Ordering = std::sync::atomic::Ordering::Relaxed;

use eframe;

#[tokio::main(worker_threads = 4)]
async fn main() {
    let config: proxy::ProxyConfig = Default::default();
    let proxy = config.build();
    let app = gui::ProxyApp::run(proxy);
    eframe::run_native(app, eframe::NativeOptions::default())
}
