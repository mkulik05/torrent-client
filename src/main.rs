mod engine;

use crate::engine::{
    download_torrent,
    logger::{self, log, LogLevel},
    parse_torrent,
    torrent::Torrent,
};
use eframe::egui::{self, Ui};
use egui_extras::{Column, TableBuilder};
use tokio::{
    sync::broadcast::{self, Receiver, Sender},
    task::JoinHandle,
};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    logger::Logger::init(format!(
        "/tmp/log{}.txt",
        chrono::Local::now().format("%d-%m-%Y_%H-%M-%S")
    ))?;

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_inner_size([900.0, 750.0]),
        ..Default::default()
    };
    eframe::run_native("Encryptor", options, Box::new(|_| Box::<MyApp>::default()));
    Ok(())
}

#[derive(Clone, Debug)]
enum UiMsg {
    // torrent hash as param
    PieceDone
}

struct TorrentDownload {
    handle: JoinHandle<()>,
    sender: Sender<UiMsg>,
    receiver: Receiver<UiMsg>,
    torrent: Torrent,
    path: String,
    pieces_done: u32,
}

struct MyApp {
    torrents: Vec<TorrentDownload>,
    context_selected_row: Option<usize>,
    selected_row: Option<usize>,
}

impl Default for MyApp {
    fn default() -> Self {
        Self {
            torrents: Vec::new(),
            context_selected_row: None,
            selected_row: None,
        }
    }
}

impl eframe::App for MyApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        for (i, torrent) in self.torrents.iter_mut().enumerate() {
            if let Ok(msg) = torrent.receiver.try_recv() {
                match msg {
                    UiMsg::PieceDone => {
                        torrent.pieces_done += 1;
                    }
                }
            }
            
        }
        egui::TopBottomPanel::top("top_panel")
            .exact_height(50.0)
            .show(ctx, |ui| {
                egui::menu::bar(ui, |ui| {
                    ui.menu_button("File", |ui| {
                        if ui.button("Open").clicked() {
                            let file = rfd::FileDialog::new()
                                .add_filter("Torrent file", &["torrent"])
                                .pick_file();
                            if let Some(path) = file {
                                let path = path.to_str().unwrap();
                                match parse_torrent(path) {
                                    Ok(torrent) => {
                                        if self
                                            .torrents
                                            .iter()
                                            .position(|x| x.torrent.info_hash == torrent.info_hash)
                                            .is_none()
                                        {
                                            let (sender, receiver) = broadcast::channel(100);
                                            let handle = {
                                                let path: String = path.to_string();
                                                let sender = sender.clone();
                                                tokio::spawn(async move {
                                                    download_torrent(
                                                        path.clone(),
                                                        "/home/mkul1k/Videos",
                                                        sender,
                                                    )
                                                    .await
                                                    .unwrap();
                                                    log!(
                                                        LogLevel::Info,
                                                        "{} download finished",
                                                        path
                                                    );
                                                })
                                            };
                                            self.torrents.push(TorrentDownload {
                                                torrent,
                                                path: path.to_string(),
                                                handle,
                                                sender,
                                                receiver,
                                                pieces_done: 0,
                                            });
                                        } else {
                                        }
                                    }
                                    Err(e) => {
                                        log!(
                                            LogLevel::Error,
                                            "Error on torrent file {path} open: {}",
                                            e
                                        );
                                    }
                                }
                            }
                        }
                    });
                    ui.menu_button("Edit", |ui| {});
                    ui.button("Settigns");
                });
            });

        egui::TopBottomPanel::bottom("bottom_panel")
            .resizable(true)
            .show(ctx, |ui| {
                egui::ScrollArea::vertical()
                    .auto_shrink(false)
                    .id_source("bottom panel scroll")
                    .show(ui, |ui| {
                        ui.label("world!");
                        ui.label("Hello");
                    });
            });

        egui::CentralPanel::default().show(ctx, |ui| {
            egui::ScrollArea::vertical()
                .auto_shrink(false)
                .id_source("table scroll")
                .min_scrolled_height(0.0)
                .show(ui, |ui| {
                    self.draw_table(ui);
                });
        });
    }
}

impl MyApp {
    fn draw_table(&mut self, ui: &mut Ui) {
        let mut table = TableBuilder::new(ui)
            .striped(true)
            .resizable(true)
            .cell_layout(egui::Layout::left_to_right(egui::Align::Center))
            .column(Column::initial(100.0).range(40.0..=300.0).clip(true))
            .column(Column::auto())
            .column(Column::initial(100.0).range(40.0..=300.0))
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
                    ui.strong("Done");
                });
                header.col(|ui| {
                    ui.strong("Expanding content");
                });
                header.col(|ui| {
                    ui.strong("Clipped text");
                });
                header.col(|ui| {
                    ui.strong("Content");
                });
            })
            .body(|body| {
                {
                    body.heterogeneous_rows((0..self.torrents.len()).map(|_| 18.0), |mut row| {
                        let row_index = row.index();
                        if let Some(n) = self.selected_row {
                            row.set_selected(n == row_index);
                        }

                        row.col(|ui| {
                            ui.label(&self.torrents[row_index].path);
                        });
                        row.col(|ui| {
                            ui.label(format!(
                                "{}",
                                self.torrents[row_index].pieces_done
                            ));
                        });
                        row.col(|ui| {
                            ui.label(self.torrents[row_index].torrent.info.piece_hashes.len().to_string());
                        });
                        row.col(|ui| {
                            ui.label(row_index.to_string());
                        });
                        row.col(|ui| {
                            ui.label(row_index.to_string());
                        });
                        row.response().context_menu(|ui| {
                            self.context_selected_row = Some(row_index);
                            if ui.button("Item").clicked() {
                                ui.close_menu();
                            };
                            ui.menu_button("Item2", |ui| {
                                ui.button("Hello2");
                            });
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
                    })
                };
            });
    }
}
