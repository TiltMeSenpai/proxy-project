mod tls;
mod proxy;
mod gui;
mod store;

use eframe;

#[tokio::main]
async fn main() {
    let config: proxy::ProxyConfig = Default::default();
    let proxy = config.build();
    let app = gui::ProxyApp::run(proxy);
    eframe::run_native(app, eframe::NativeOptions::default())
}
