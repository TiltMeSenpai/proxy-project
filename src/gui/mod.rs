use eframe::{epi, egui};
use super::proxy::{ProxyServer, ProxyEvent};
use super::store::Store;
use tokio::{sync::broadcast::Receiver, task::JoinHandle};

pub struct ProxyApp {
    events: Receiver<ProxyEvent>,
    server: JoinHandle<Result<(), hyper::Error>>,
    store: Store,
}

impl ProxyApp {
    pub fn run(server: ProxyServer) -> Box<Self> {
        let mut store = Store::new();
        store.subscribe(server.subscribe());
        Box::new(Self {
            events: server.subscribe(),
            store:  store,
            server: server.run()
        })
    }
}

impl epi::App for ProxyApp {
    fn update(&mut self, ctx: &egui::CtxRef, frame: &epi::Frame) {
        self.store.set_frame(frame.clone());
        egui::CentralPanel::default().show(ctx, |ui| {
            ui.heading("Request count:");
            ui.label(format!("Processed {:?} requests", self.store.size()));
            ui.label(format!("Dropped {} requests", self.store.lag()));
        });
    }

    fn name(&self) -> &str {
        "Proxy App"
    }
}