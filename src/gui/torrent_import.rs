use crate::gui::MyApp;
use crate::gui::TorrentInfo;
use egui::{ViewportBuilder, ViewportId};
use std::path::Path;
impl MyApp {
    pub fn import_window(&mut self, ctx: &egui::Context) {
        ctx.show_viewport_immediate(
            ViewportId::from_hash_of("Import torrent window"),
            ViewportBuilder::default()
                .with_title("Import torrent")
                .with_inner_size([500.0, 500.0]),
            |ctx, _| {
                if ctx.input(|i| i.viewport().close_requested()) {
                    self.import_opened = false;
                }
                egui::CentralPanel::default().show(ctx, |ui| {
                    ui.vertical(|ui| {
                        ui.label("Destination folder:");
                        ui.horizontal(|ui| {
                            ui.add(
                                egui::TextEdit::singleline(&mut self.import_dest_dir)
                                    .desired_width(100.0)
                                    .desired_rows(1)
                                    .id_source("import dir"),
                            );
                            if ui.button("Select folder").clicked() {
                                let file = rfd::FileDialog::new().pick_folder();
                                if let Some(path) = file {
                                    self.import_dest_dir = path.to_str().unwrap().to_owned();
                                }
                            };
                        });
                    });
                    let dest_path = Path::new(&self.import_dest_dir);
                    let button_enabled = if !self.import_dest_dir.is_empty()
                        && dest_path.exists()
                        && dest_path.is_dir()
                    {
                        true
                    } else {
                        false
                    };
                    ui.set_enabled(button_enabled);
                    if ui.button("Start").clicked() {
                        self.import_opened = false;
                        self.start_download(
                            TorrentInfo::Torrent(self.import_torrent.as_ref().unwrap().clone()),
                            ctx,
                        );
                    }
                });
            },
        );
    }
}