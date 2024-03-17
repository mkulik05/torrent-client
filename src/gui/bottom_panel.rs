use super::get_readable_size;
use crate::gui::files_tree::draw_tree;
use crate::gui::MyApp;
use egui::{FontFamily, FontId, TextFormat};

macro_rules! label {
    ($ui:ident, $str1:expr, $str2:expr, $max_n:expr) => {{
        let mut job = egui::text::LayoutJob::default();
        job.wrap = egui::text::TextWrapping {
            break_anywhere: true,
            ..Default::default()
        };
        job.append(
            format!("{:<width$} {}", $str1, $str2, width = $max_n).as_str(),
            0.0,
            TextFormat {
                font_id: FontId::new(12.0, FontFamily::Monospace),
                ..Default::default()
            },
        );
        $ui.label(job)
    }};
}

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
                                let max_w = 11;
                                let mut data = String::new();
                                if let Some(i) = self.selected_row {
                                    data = self.torrents[i].save_dir.clone();
                                }
                                label!(ui, "Save path:", data, max_w);

                                if let Some(i) = self.selected_row {
                                    data = get_readable_size(
                                        self.torrents[i].torrent.info.length as usize,
                                        3,
                                    );
                                }

                                label!(ui, "Total size:", data, max_w);

                                if let Some(i) = self.selected_row {
                                    data = get_readable_size(
                                        self.torrents[i].torrent.info.piece_length as usize,
                                        0,
                                    );
                                }

                                label!(ui, "Piece size:", data, max_w);

                                if let Some(i) = self.selected_row {
                                    data = self.torrents[i]
                                        .torrent
                                        .info
                                        .piece_hashes
                                        .len()
                                        .to_string();
                                }

                                label!(ui, "Pieces N:", data, max_w);

                                if let Some(i) = self.selected_row {
                                    data = hex::encode(&self.torrents[i].torrent.info_hash);
                                }

                                label!(ui, "Info hash:", data, max_w);
                            });

                            cols[1].label("Downloading");
                            cols[1].group(|ui| {
                                let max_w = 15;
                                let mut data = String::new();

                                if let Some(i) = self.selected_row {
                                    let mut size = self.torrents[i].pieces_done as usize
                                    * self.torrents[i].torrent.info.piece_length as usize;
                                    if size > self.torrents[i].torrent.info.length as usize {
                                        size = self.torrents[i].torrent.info.length as usize;
                                    } 
                                    data = get_readable_size(size,
                                        1,
                                    );
                                }

                                label!(ui, "Downloaded:", data, max_w);

                                label!(ui, "Uploaded:", 0, max_w);

                                label!(ui, "Peers number:", 0, max_w);
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
