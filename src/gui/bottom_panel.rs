use crate::gui::MyApp;
use super::get_readable_size;
use crate::gui::files_tree::draw_tree;

impl MyApp {
    pub fn bottom_panel(&mut self, ctx: &egui::Context) {
        egui::TopBottomPanel::bottom("bottom_panel")
            .default_height(200.0)
            .resizable(true)
            .show(ctx, |ui| {
                ui.set_enabled(!self.import_opened);
                egui::ScrollArea::vertical()
                    .auto_shrink(false)
                    .id_source("bottom panel scroll")
                    .show(ui, |ui| {
                        ui.add_space(10.0);
                        ui.columns(2, |cols| {
                            cols[0].label("General info");
                            cols[0].group(|ui| {
                                let max_w = 15;
                                let mut data = String::new();
                                if let Some(i) = self.selected_row {
                                    data = self.torrents[i].save_dir.clone();
                                }
                                ui.monospace(format!(
                                    "{:<width$} {}",
                                    "Save path:",
                                    data,
                                    width = max_w
                                ));

                                if let Some(i) = self.selected_row {
                                    data = get_readable_size(
                                        self.torrents[i].torrent.info.length as usize,
                                        3,
                                    );
                                }
                                ui.monospace(format!(
                                    "{:<width$} {}",
                                    "Total size:",
                                    data,
                                    width = max_w
                                ));

                                if let Some(i) = self.selected_row {
                                    data = get_readable_size(
                                        self.torrents[i].torrent.info.piece_length as usize,
                                        0,
                                    );
                                }
                                ui.monospace(format!(
                                    "{:<width$} {}",
                                    "Piece size:",
                                    data,
                                    width = max_w
                                ));

                                if let Some(i) = self.selected_row {
                                    data = self.torrents[i]
                                        .torrent
                                        .info
                                        .piece_hashes
                                        .len()
                                        .to_string();
                                }
                                ui.monospace(format!(
                                    "{:<width$} {}",
                                    "Pieces number:",
                                    data,
                                    width = max_w
                                ));

                                if let Some(i) = self.selected_row {
                                    data = hex::encode(&self.torrents[i].torrent.info_hash);
                                }
                                ui.monospace(format!(
                                    "{:<width$} {}",
                                    "Info hash:",
                                    data,
                                    width = max_w
                                ));
                            });

                            cols[1].label("Downloading");
                            cols[1].group(|ui| {
                                let max_w = 20;
                                let mut data = String::new();

                                if let Some(i) = self.selected_row {
                                    data = get_readable_size(
                                        self.torrents[i].torrent.info.piece_length as usize
                                            * self.torrents[i].torrent.info.piece_hashes.len()
                                                as usize,
                                        1,
                                    );
                                }
                                ui.monospace(format!(
                                    "{:<width$} {}",
                                    "Downloaded:",
                                    data,
                                    width = max_w
                                ));
                                ui.monospace(format!(
                                    "{:<width$} {}",
                                    "Uploaded:",
                                    0,
                                    width = max_w
                                ));
                                ui.monospace(format!(
                                    "{:<width$} {}",
                                    "Peers number:",
                                    0,
                                    width = max_w
                                ));
                            });
                        });

                        ui.add_space(10.0);

                        ui.columns(2, |cols| {
                            cols[0].label("Files");
                            cols[0].group(|ui| {
                                if let Some(i) = self.selected_row {
                                    egui::ScrollArea::vertical()
                                        .max_height(ui.available_height() / 1.3)
                                        .auto_shrink(false)
                                        .show(ui, |ui| {
                                            let torrent = &self.torrents[i].torrent;
                                            if let Some(files) = &torrent.info.files {
                                                draw_tree(
                                                    &files
                                                        .iter()
                                                        .map(|x| x.path.as_str())
                                                        .collect(),
                                                    torrent.info.name.clone(),
                                                    ui,
                                                )
                                            } else {
                                                ui.label(&torrent.info.name);
                                            }
                                        });
                                } else {
                                    ui.label("");
                                }
                            });
                            cols[1].label("Peers");
                            cols[1].group(|ui| {
                                ui.monospace("world!");
                                ui.monospace("Hello");
                            });
                        });
                    });
            });
    }
}
