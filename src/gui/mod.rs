use crate::engine::backup;
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
use egui::{ViewportBuilder, ViewportId};
use egui_extras::{Column, TableBuilder};
use std::collections::VecDeque;
use std::path::Path;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tokio::{
    sync::broadcast::{self, Receiver, Sender},
    task::JoinHandle,
};

#[derive(Serialize, Deserialize, Clone, Debug)]
pub enum DownloadStatus {
    Downloading,
    Paused,
    Finished,
}

// remember to update bitfields for each piece
#[derive(Serialize, Deserialize, Debug)]
pub struct TorrentBackupInfo {
    pub pieces_tasks: VecDeque<PieceTask>,
    pub chunks_tasks: VecDeque<ChunksTask>,
    pub torrent: Torrent,
    pub save_path: String,
    pub pieces_done: usize,
    pub status: DownloadStatus,
}

#[derive(Clone, Debug)]
pub struct UiHandle {
    pub ui_sender: Sender<UiMsg>,
    pub ctx: egui::Context,
}

impl UiHandle {
    pub fn send_with_update(&self, msg: UiMsg) -> anyhow::Result<()> {
        self.ui_sender.send(msg)?;
        self.ctx.request_repaint();
        Ok(())
    }
}

#[derive(Clone, Debug)]
pub enum UiMsg {
    TorrentFinished,
    PieceDone(u16),
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

pub struct MyApp {
    torrents: Vec<TorrentDownload>,
    selected_row: Option<usize>,
    user_msg: Option<(String, String)>,
    inited: bool,
    pieces: Vec<u16>,
    import_opened: bool,
    import_dest_dir: String,
    import_torrent: Option<Torrent>,
}

pub fn start_gui() -> anyhow::Result<()> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_inner_size([900.0, 750.0]),
        ..Default::default()
    };
    eframe::run_native("MkTorrent", options, Box::new(|_| Box::<MyApp>::default())).unwrap();

    std::thread::sleep(Duration::from_secs(5));
    Ok(())
}

impl Default for MyApp {
    fn default() -> Self {
        Self {
            torrents: Vec::new(),
            selected_row: None,
            user_msg: None,
            inited: false,
            pieces: Vec::new(),
            import_opened: false,
            import_dest_dir: String::new(),
            import_torrent: None,
        }
    }
}

impl MyApp {
    fn start_download(&mut self, torrent_info: TorrentInfo, resume: bool, ctx: &egui::Context) {
        let torrent = match &torrent_info {
            TorrentInfo::Torrent(torrent) => torrent.clone(),
            TorrentInfo::Backup(backup) => backup.torrent.clone(),
        };

        let (sender, receiver) = broadcast::channel(20_000);
        let handle = {
            let name = torrent.info.name.clone();
            let sender = sender.clone();
            let ctx = ctx.clone();
            let folder = if let TorrentInfo::Backup(ref backup) = torrent_info {
                backup.save_path.clone()
            } else {
                self.import_dest_dir.clone()
            };
            tokio::spawn(async move {
                log!(LogLevel::Info, "Strating torrent downloading: {name}");
                download_torrent(
                    torrent_info,
                    &folder,
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

        if self.import_opened {
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
                                false,
                                ctx,
                            );
                        }
                    });
                },
            );
            ctx.request_repaint();
        }

        for torrent in &mut self.torrents {
            if torrent.worker_info.is_none() {
                continue;
            }
            while let Ok(msg) = torrent.worker_info.as_mut().unwrap().receiver.try_recv() {
                match msg {
                    UiMsg::PieceDone(piece) => {
                        self.pieces.push(piece);
                        log!(LogLevel::Info, "{:?}", self.pieces);
                        torrent.pieces_done += 1;
                        if torrent.pieces_done == torrent.torrent.info.piece_hashes.len() as u32 {
                            torrent.status = DownloadStatus::Finished;
                        }
                    }
                    UiMsg::TorrentFinished => {
                        torrent.status = DownloadStatus::Finished;
                        torrent.pieces_done = torrent.torrent.info.piece_hashes.len() as u32;
                    }
                    _ => {}
                }
            }
        }

        self.show_message(ctx);

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

        egui::TopBottomPanel::bottom("bottom_panel")
            .resizable(true)
            .show(ctx, |ui| {
                ui.set_enabled(!self.import_opened);
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
                    ui.set_enabled(!self.import_opened);
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
                    log!(LogLevel::Info, "done: {}", backup.pieces_done);
                    if let DownloadStatus::Downloading = backup.status {
                        self.start_download(TorrentInfo::Backup(backup), true, ctx);
                    }
                }
            }
            Err(e) => log!(LogLevel::Error, "Failed to open backup file: {e}"),
        }
        let ctx = ctx.clone();
        tokio::spawn(async move {
            loop {
                ctx.request_repaint();
                tokio::time::sleep(Duration::from_millis(200)).await;
            }
        });
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

    fn pause_torrent(&mut self, i: usize) {
        if let Some(ref info) = self.torrents[i].worker_info {
            log!(LogLevel::Info, "Sended msg!!!");
            info.sender
                .send(UiMsg::Pause(self.torrents[i].pieces_done as u16))
                .unwrap();
            log!(LogLevel::Info, "Finished: sended msg!!!");
            self.torrents[i].status = DownloadStatus::Paused;
        }
    }

    fn resume_torrent(&mut self, i: usize, ctx: &egui::Context) {
        let backup = backup::load_backup(&self.torrents[i].torrent.info_hash);
        log!(LogLevel::Info, "{:?}", backup);
        match backup {
            Ok(backup) => {
                self.start_download(TorrentInfo::Backup(backup), true, ctx);
            }
            Err(_) => {
                self.start_download(
                    TorrentInfo::Torrent(self.torrents[i].torrent.clone()),
                    true,
                    ctx,
                );
            }
        }
        self.torrents[i].status = DownloadStatus::Downloading;
    }

    fn delete_torrent(&mut self, i: usize) {
        if let Some(ref info) = self.torrents[i].worker_info {
            info.sender.send(UiMsg::ForceOff).unwrap();
        }
        backup::remove_torrent(&self.torrents[i].torrent.info_hash).unwrap();
        self.torrents.remove(i);
    }

    fn draw_table(&mut self, ui: &mut Ui, ctx: &egui::Context) {
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
                            let size = get_readable_size(
                                self.torrents[row_index].pieces_done as usize
                                    * self.torrents[row_index].torrent.info.piece_length as usize,
                            );
                            ui.label(size);
                        });
                        row.col(|ui| {
                            ui.label("0");
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

fn get_readable_size(bytes: usize) -> String {
    match bytes {
        0..=1023 => format!("{bytes}B"),
        1024..=1_048_575 => format!("{:.2}KB", bytes as f64 / 1024.0),
        1_048_576..=1_073_741_823 => format!("{:.2}MB", bytes as f64 / 1_048_576.0),
        _ => format!("{:.2}GB", bytes as f64 / 1_073_741_824.0),
    }
}
