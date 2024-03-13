mod download_table;
mod top_panel;
mod bottom_panel;
mod central_panel;
mod torrent_import;
mod torrent_actions;
mod files_tree;

use crate::engine::backup;
use crate::engine::TorrentInfo;
use crate::engine::{
    download::{ChunksTask, PieceTask},
    logger::{log, LogLevel},
    torrent::Torrent,
};
use eframe::egui;
use std::collections::VecDeque;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tokio::{
    sync::broadcast::{Receiver, Sender},
    task::JoinHandle,
};

pub fn start_gui() -> anyhow::Result<()> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_inner_size([900.0, 750.0]),
        ..Default::default()
    };
    eframe::run_native("MkTorrent", options, Box::new(|_| Box::<MyApp>::default())).unwrap();

    std::thread::sleep(Duration::from_secs(5));
    Ok(())
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub enum DownloadStatus {
    Downloading,
    Paused,
    Finished,
}

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

impl eframe::App for MyApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        
        if !self.inited {
            self.init(ctx);
        }
        if self.import_opened {
            self.import_window(ctx);            
        }

        self.torrent_updates();

        self.show_message(ctx);
        
        self.top_panel(ctx);
        self.bottom_panel(ctx);
        self.cenral_panel(ctx);
        
    }
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
                        self.start_download(TorrentInfo::Backup(backup), ctx);
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
}

fn get_readable_size(bytes: usize) -> String {
    match bytes {
        0..=1023 => format!("{bytes}B"),
        1024..=1_048_575 => format!("{:.2}KB", bytes as f64 / 1024.0),
        1_048_576..=1_073_741_823 => format!("{:.2}MB", bytes as f64 / 1_048_576.0),
        _ => format!("{:.2}GB", bytes as f64 / 1_073_741_824.0),
    }
}
