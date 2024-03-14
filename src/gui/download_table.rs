use crate::gui::{get_readable_size, DownloadStatus, MyApp};
use eframe::egui::Ui;
use egui::Color32;
use egui_extras::{Column, TableBuilder};
impl MyApp {
    pub fn draw_table(&mut self, ui: &mut Ui, ctx: &egui::Context) {
        let mut table = TableBuilder::new(ui)
            .striped(true)
            .resizable(true)
            .cell_layout(egui::Layout::left_to_right(egui::Align::Center))
            .column(Column::auto().clip(true))
            .column(Column::auto())
            .column(Column::auto())
            .column(Column::initial(100.0).at_least(40.0).clip(true))
            .column(Column::remainder())
            .min_scrolled_height(0.0);

        table = table.sense(egui::Sense::click());
        table
            .header(20.0, |mut header| {
                header.col(|ui| {
                    ui.strong("Name");
                });
                header.col(|ui| {
                    ui.strong("Size");
                });
                header.col(|ui| {
                    ui.strong("Progress");
                });
                header.col(|ui| {
                    ui.strong("Downloaded");
                });
                header.col(|ui| {
                    ui.strong("Uploaded");
                });
            })
            .body(|body| {
                {
                    body.heterogeneous_rows((0..self.torrents.len()).map(|_| 16.0), |mut row| {
                        let row_index = row.index();
                        if let Some(n) = self.selected_row {
                            row.set_selected(n == row_index);
                        }

                        row.col(|ui| {
                            ui.label(&self.torrents[row_index].torrent.info.name);
                        });
                        row.col(|ui| {
                            let postfixed_size = get_readable_size(
                                self.torrents[row_index].torrent.info.length as usize,
                                2,
                            );
                            ui.label(postfixed_size);
                        });
                        row.col(|ui| {
                            let progress_bar = {
                                match self.torrents[row_index].status {
                                    DownloadStatus::Downloading => {
                                        let progress = self.torrents[row_index].pieces_done as f32
                                            / self.torrents[row_index]
                                                .torrent
                                                .info
                                                .piece_hashes
                                                .len()
                                                as f32;
                                        egui::ProgressBar::new(progress)
                                            .text(format!("{:.2}%", progress * 100.0))
                                    }
                                    DownloadStatus::Finished => {
                                        egui::ProgressBar::new(1.0).fill(Color32::GREEN)
                                    }
                                    _ => {
                                        let progress = self.torrents[row_index].pieces_done as f32
                                            / self.torrents[row_index]
                                                .torrent
                                                .info
                                                .piece_hashes
                                                .len()
                                                as f32;
                                        egui::ProgressBar::new(progress)
                                            .text(format!("{:.2}%", progress * 100.0))
                                            .fill(Color32::GRAY)
                                    }
                                }
                            };
                            ui.add(progress_bar);
                        });
                        row.col(|ui| {
                            let size = get_readable_size(
                                self.torrents[row_index].pieces_done as usize
                                    * self.torrents[row_index].torrent.info.piece_length as usize,
                                0,
                            );
                            ui.label(size);
                        });
                        row.col(|ui| {
                            ui.label("0");
                        });

                        if row.response().clicked() {
                            self.selected_row = if let Some(n) = self.selected_row {
                                if n == row_index {
                                    None
                                } else {
                                    Some(row_index)
                                }
                            } else {
                                Some(row_index)
                            }
                        }
                        if let DownloadStatus::Error(msg) = &self.torrents[row_index].status {
                            row.response().on_hover_text(msg);
                        }
                        
                        row.response().context_menu(|ui| {
                            // self.context_selected_row = Some(row_index);

                            let enabled = if let DownloadStatus::Finished
                            | DownloadStatus::Downloading =
                                self.torrents[row_index].status
                            {
                                false
                            } else {
                                true
                            };
                            if ui
                                .add_enabled(enabled, egui::Button::new("Resume"))
                                .clicked()
                            {
                                self.resume_torrent(row_index, ctx);
                                ui.close_menu();
                            };

                            let enabled = if let DownloadStatus::Finished | DownloadStatus::Paused =
                                self.torrents[row_index].status
                            {
                                false
                            } else {
                                true
                            };
                            if ui
                                .add_enabled(enabled, egui::Button::new("Pause"))
                                .clicked()
                            {
                                self.pause_torrent(row_index);
                                ui.close_menu();
                            };

                            if ui.button("Delete").clicked() {
                                self.delete_torrent(row_index);
                                ui.close_menu();
                            };
                        });
                    })
                };
            });
    }
}
