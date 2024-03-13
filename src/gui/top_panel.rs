use crate::engine::parse_torrent;
use crate::gui::{DownloadStatus, MyApp};
impl MyApp {
    pub fn top_panel(&mut self, ctx: &egui::Context) {
        egui::TopBottomPanel::top("top_panel")
            .exact_height(50.0)
            .show(ctx, |ui| {
                ui.set_enabled(!self.import_opened);
                egui::menu::bar(ui, |ui| {
                    ui.menu_button("File", |ui| {
                        if ui.button("Open").clicked() {
                            ui.close_menu();
                            let file = rfd::FileDialog::new()
                                .add_filter("Torrent file", &["torrent"])
                                .pick_file();
                            if let Some(path) = file {
                                let torrent = parse_torrent(path.to_str().unwrap());
                                if let Ok(torrent) = torrent {
                                    if self
                                        .torrents
                                        .iter()
                                        .position(|x| x.torrent.info_hash == torrent.info_hash)
                                        .is_none()
                                    {
                                        self.import_torrent = Some(torrent);
                                        self.import_opened = true;
                                    } else {
                                        self.user_msg = Some((
                                            "Alert".to_string(),
                                            "This torrent is already imported".to_string(),
                                        ));
                                        ctx.request_repaint();
                                    }
                                }
                            }
                        }
                    });
                    ui.menu_button("Edit", |ui| {});
                    ui.button("Settigns");
                });

                ui.separator();
                ui.horizontal(|ui| {
                    ui.horizontal(|ui| {
                        ui.set_enabled(self.selected_row.is_some());
                        if ui.button("Pause").clicked() {
                            self.pause_torrent(self.selected_row.unwrap());
                        }
                        if ui.button("Resume").clicked() {
                            self.resume_torrent(self.selected_row.unwrap(), ctx);
                        }
                        if ui.button("Delete").clicked() {
                            self.delete_torrent(self.selected_row.unwrap());
                        }
                    });
                    ui.separator();
                    ui.horizontal(|ui| {
                        if ui.button("Pause All").clicked() {
                            let mut torrents_to_pause = Vec::new();
                            for (i, entry) in self.torrents.iter().enumerate() {
                                if let DownloadStatus::Downloading = entry.status {
                                    torrents_to_pause.push(i);
                                }
                            }
                            for torrent_i in torrents_to_pause {
                                self.pause_torrent(torrent_i);
                            }
                        };
                        if ui.button("Resume All").clicked() {
                            let mut torrents_to_resume = Vec::new();
                            for (i, entry) in self.torrents.iter().enumerate() {
                                if let DownloadStatus::Paused = entry.status {
                                    torrents_to_resume.push(i);
                                }
                            }
                            for torrent_i in torrents_to_resume {
                                self.resume_torrent(torrent_i, ctx);
                            }
                        };
                    });
                });
            });
    }
}
