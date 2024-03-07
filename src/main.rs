mod backup;
mod engine;

use crate::engine::TorrentInfo;
use crate::engine::{
    download::{ChunksTask, PieceTask},
    download_torrent,
    logger::{self, log, LogLevel},
    parse_torrent,
    torrent::Torrent,
};
use eframe::egui::{self, Ui};
use egui::Color32;
use egui_extras::{Column, TableBuilder};
use std::collections::VecDeque;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tokio::{
    sync::broadcast::{self, Receiver, Sender},
    task::JoinHandle,
};

#[derive(Serialize, Deserialize, Clone)]
enum DownloadStatus {
    Downloading,
    Paused,
    Finished,
}

// remember to update bitfields for each piece
#[derive(Serialize, Deserialize)]
struct TorrentBackupInfo {
    pieces_tasks: VecDeque<PieceTask>,
    chunks_tasks: VecDeque<ChunksTask>,
    torrent: Torrent,
    save_path: String,
    pieces_done: usize,
    status: DownloadStatus,
}

#[derive(Clone, Debug)]
struct UiHandle {
    ui_sender: Sender<UiMsg>,
    ctx: egui::Context,
}

impl UiHandle {
    fn send_with_update(&self, msg: UiMsg) -> anyhow::Result<()> {
        self.ui_sender.send(msg)?;
        self.ctx.request_repaint();
        Ok(())
    }
}

#[derive(Clone, Debug)]
enum UiMsg {
    PieceDone,
    ForceOff,

    // Pieces downloaded in total
    Pause(u16),

    // Pieces done
    Stop(u16),
}

struct WorkerInfo {
    handle: JoinHandle<()>,
    sender: Sender<UiMsg>,
    receiver: Receiver<UiMsg>,
}

struct TorrentDownload {
    status: DownloadStatus,
    worker_info: Option<WorkerInfo>,
    torrent: Torrent,
    pieces_done: u32,
}

struct MyApp {
    torrents: Vec<TorrentDownload>,
    context_selected_row: Option<usize>,
    selected_row: Option<usize>,
    user_msg: Option<(String, String)>,
    inited: bool,
}

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
    eframe::run_native("Encryptor", options, Box::new(|_| Box::<MyApp>::default())).unwrap();

    std::thread::sleep(Duration::from_secs(5));
    Ok(())
}

impl Default for MyApp {
    fn default() -> Self {
        Self {
            torrents: Vec::new(),
            context_selected_row: None,
            selected_row: None,
            user_msg: None,
            inited: false,
        }
    }
}

impl MyApp {
    fn start_download(&mut self, torrent_info: TorrentInfo, resume: bool, ctx: &egui::Context) {
        let torrent = match &torrent_info {
            TorrentInfo::Torrent(torrent) => torrent.clone(),
            TorrentInfo::Backup(backup) => backup.torrent.clone(),
        };
        if resume
            || self
                .torrents
                .iter()
                .position(|x| x.torrent.info_hash == torrent.info_hash)
                .is_none()
        {
            let (sender, receiver) = broadcast::channel(100);
            let handle = {
                let name = torrent.info.name.clone();
                let sender = sender.clone();
                let ctx = ctx.clone();
                tokio::spawn(async move {
                    log!(LogLevel::Info, "Strating torrent downloading: {name}");
                    download_torrent(
                        torrent_info,
                        "/home/mkul1k/Videos",
                        UiHandle {
                            ui_sender: sender,
                            ctx,
                        },
                    )
                    .await
                    .unwrap();
                    log!(LogLevel::Info, "{} download finished", name);
                })
            };
            let info = WorkerInfo {
                handle,
                sender,
                receiver,
            };

            let torrent_i = self
                .torrents
                .iter()
                .position(|x| x.torrent.info_hash == torrent.info_hash);
            if torrent_i.is_none() {
                self.torrents.push(TorrentDownload {
                    torrent,
                    status: DownloadStatus::Downloading,
                    worker_info: Some(info),
                    pieces_done: 0,
                });
            } else {
                self.torrents[torrent_i.unwrap()].worker_info = Some(info);
            }
        } else {
            self.user_msg = Some((
                "Alert".to_string(),
                "This torrent is already imported".to_string(),
            ));
            ctx.request_repaint();
        }
    }
}

impl eframe::App for MyApp {
    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        for q_torrent in &self.torrents {
            match q_torrent.status {
                DownloadStatus::Downloading => {
                    if let Some(info) = &q_torrent.worker_info {
                        info.sender
                            .send(UiMsg::Stop(q_torrent.pieces_done as u16))
                            .unwrap();
                    }
                }
                DownloadStatus::Finished => {
                    backup::backup_torrent(TorrentBackupInfo {
                        pieces_tasks: VecDeque::new(),
                        chunks_tasks: VecDeque::new(),
                        torrent: q_torrent.torrent.clone(),
                        save_path: "".to_string(),
                        pieces_done: 0,
                        status: DownloadStatus::Finished,
                    })
                    .unwrap();
                }
                DownloadStatus::Paused => {
                    // Data is already backed up
                }
            }
        }
    }

    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        if !self.inited {
            self.init(ctx);
        }

        for torrent in &mut self.torrents {
            if torrent.worker_info.is_none() {
                continue;
            }
            if let Ok(msg) = torrent.worker_info.as_mut().unwrap().receiver.try_recv() {
                match msg {
                    UiMsg::PieceDone => {
                        torrent.pieces_done += 1;
                    }
                    _ => {}
                }
            }
        }

        self.show_message(ctx);

        egui::TopBottomPanel::top("top_panel")
            .exact_height(50.0)
            .show(ctx, |ui| {
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
                                    self.start_download(TorrentInfo::Torrent(torrent), false, ctx);
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
                    self.draw_table(ui, ctx);
                });
        });
    }
}

impl MyApp {
    fn init(&mut self, ctx: &egui::Context) {
        self.inited = true;
        match backup::load_config() {
            Ok(backups) => {
                for backup in backups {
                    self.torrents.push(TorrentDownload {
                        status: backup.status.clone(),
                        worker_info: None,
                        torrent: backup.torrent.clone(),
                        pieces_done: backup.pieces_done as u32,
                    });
                    if let DownloadStatus::Downloading = backup.status {
                        self.start_download(TorrentInfo::Backup(backup), true, ctx);
                    }
                }
            }
            Err(e) => log!(LogLevel::Error, "Failed to open backup file: {e}"),
        }
    }
    fn show_message(&mut self, ctx: &egui::Context) {
        let Some((header, msg)) = self.user_msg.clone() else {
            return;
        };
        egui::Window::new(header)
            .collapsible(false)
            .resizable(false)
            .show(ctx, |ui| {
                ui.vertical(|ui| {
                    ui.label(msg);
                    ui.spacing();
                    if ui.button("Okay").clicked() {
                        self.user_msg = None;
                    }
                });
            });
    }
    fn draw_table(&mut self, ui: &mut Ui, ctx: &egui::Context) {
        let mut table = TableBuilder::new(ui)
            .striped(true)
            .resizable(true)
            .cell_layout(egui::Layout::left_to_right(egui::Align::Center))
            .column(Column::initial(100.0).clip(true))
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
                    ui.strong("Clipped text");
                });
                header.col(|ui| {
                    ui.strong("Content");
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
                            let postfixed_size = {
                                let size = self.torrents[row_index].torrent.info.length;
                                match size {
                                    0..=999 => format!("{size}B"),
                                    1000..=999_999 => format!("{:.2}B", size / 1000),
                                    1000_000..=999_999_999 => format!("{:.2}MB", size / 1000_000),
                                    _ => {
                                        format!("{:.2}GB", size / 1000_000_000)
                                    }
                                }
                            };
                            ui.label(postfixed_size);
                        });
                        row.col(|ui| {
                            let progress_bar = {
                                match self.torrents[row_index].status {
                                    DownloadStatus::Downloading => {
                                        if self.torrents[row_index].pieces_done
                                        == self.torrents[row_index]
                                            .torrent
                                            .info
                                            .piece_hashes
                                            .len() as u32 {
                                                self.torrents[row_index].status = DownloadStatus::Finished;
                                            }
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
                                    DownloadStatus::Paused => {
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
                            ui.label(format!(
                                "{:.3}",
                                self.torrents[row_index].pieces_done as f32
                                    / self.torrents[row_index].torrent.info.piece_hashes.len()
                                        as f32
                            ));
                        });
                        row.col(|ui| {
                            ui.label(row_index.to_string());
                        });
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
                                let backup = backup::load_backup(
                                    &self.torrents[row_index].torrent.info_hash,
                                );
                                match backup {
                                    Ok(backup) => {
                                        self.start_download(TorrentInfo::Backup(backup), true, ctx);
                                    }
                                    Err(_) => {
                                        self.start_download(
                                            TorrentInfo::Torrent(
                                                self.torrents[row_index].torrent.clone(),
                                            ),
                                            false,
                                            ctx,
                                        );
                                    }
                                }
                                self.torrents[row_index].status = DownloadStatus::Downloading;
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
                                if let Some(ref info) = self.torrents[row_index].worker_info {
                                    info.sender
                                        .send(UiMsg::Pause(
                                            self.torrents[row_index].pieces_done as u16,
                                        ))
                                        .unwrap();
                                    self.torrents[row_index].status = DownloadStatus::Paused;
                                }
                                ui.close_menu();
                            };

                            if ui.button("Delete").clicked() {
                                if let Some(ref info) = self.torrents[row_index].worker_info {
                                    info.sender.send(UiMsg::ForceOff).unwrap();
                                }
                                self.torrents.remove(row_index);
                                ui.close_menu();
                            };
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
