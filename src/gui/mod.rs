use eframe::egui::ScrollArea;
use eframe::{epi, egui};
use super::proxy::ProxyServer;
use super::store::Store;
use tokio::task::JoinHandle;

pub struct ProxyApp {
    #[allow(dead_code)] // We mostly hold onto this so the future doesn't get cancelled
    server: JoinHandle<Result<(), hyper::Error>>,
    store: Store,
}

impl ProxyApp {
    pub fn run(server: ProxyServer) -> Box<Self> {
        let mut store = Store::new();
        store.subscribe(server.subscribe());
        Box::new(Self {
            store:  store,
            server: server.run()
        })
    }
}

impl epi::App for ProxyApp {
    fn update(&mut self, ctx: &egui::CtxRef, frame: &epi::Frame) {
        self.store.set_frame(frame.clone());
        egui::SidePanel::left("Request bar").show( ctx, |ui| {
            let text_style = egui::TextStyle::Monospace;
            let font = &ui.fonts()[text_style];
            let row_height = font.row_height();
            let char_width = font.glyph_width('w'); // Arbitrarily assuming "w" is one of the wider characters
            let width = (ui.available_width() / char_width).floor() as usize;
            let num_rows = self.store.size().unwrap_or(0);
            ScrollArea::vertical().show_rows(ui, row_height, num_rows, |ui, range| self.store.draw_sidebar(ui, range, width));
            ui.allocate_space(ui.available_size());
        });
        egui::CentralPanel::default().show(ctx, |ui| {
            self.store.draw_active(ui);
            ui.allocate_space(ui.available_size());
        });
    }

    fn name(&self) -> &str {
        "Proxy App"
    }
}