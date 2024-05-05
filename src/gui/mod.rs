mod download_table;
mod top_panel;
mod bottom_panel;
mod central_panel;
mod torrent_import;
mod torrent_actions;
mod files_tree;

use once_cell::sync::OnceCell;
use crate::engine::backup::Backup;
use crate::engine::TorrentInfo;
use crate::engine::{
    download::{ChunksTask, PieceTask},
    logger::{log, LogLevel},
    torrent::Torrent,
};
use crate::engine::saver;

use eframe::egui;
use egui::Modifiers;
use std::collections::VecDeque;
use std::time::Duration;
use std::time::Instant;

use serde::{Deserialize, Serialize};
use tokio::{
    sync::broadcast::{Receiver, Sender},
    task::JoinHandle,
};


const PIECES_TO_TIME_MEASURE: u8 = 5;

pub fn start_gui() -> anyhow::Result<()> {
    let icon = include_bytes!("../../folder-download.png");
    let image = image::load_from_memory(icon).expect("Failuse to load image").to_rgba8();
    let (icon_width, icon_height) = image.dimensions();
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_inner_size([900.0, 750.0]).with_min_inner_size([400.0, 200.0]).with_icon(
            egui::IconData {
                rgba: image.into_raw(), 
                width: icon_width, 
                height: icon_height,
        }),
        ..Default::default()
    };
    eframe::run_native("MkTorrent", options, Box::new(|_| Box::<MyApp>::default())).unwrap();
    Ok(())
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub enum DownloadStatus {
    Downloading,
    Paused,
    Finished,
    Error(String)
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

    PeerDiscovered(String),

    PeerDisconnect(String),

    TorrentFinished,
    PieceDone(u16),
    ForceOff,

    // String with error text
    TorrentErr(String),

    // Pieces downloaded in total
    Pause(u16),

    // Pieces done
    Stop(u16),

    // Uploaded bytes
    DataUploaded(u64)
}

struct WorkerInfo {
    handle: JoinHandle<()>,
    sender: Sender<UiMsg>,
    receiver: Receiver<UiMsg>,
}

pub struct TimeStamp {
    pub time: Instant,
    pub pieces_n: u32
}
struct TorrentDownload {
    status: DownloadStatus,
    worker_info: Option<WorkerInfo>,
    peers: Vec<String>,
    torrent: Torrent,
    pieces_done: u32,
    last_timestamp: Option<TimeStamp>,
    download_speed: Option<u16>,
    save_dir: String,
    uploaded: u32
}

pub struct MyApp {
    torrents: Vec<TorrentDownload>,
    selected_row: Option<usize>,
    user_msg: Option<(String, String)>,
    inited: bool,
    import_opened: bool,
    import_dest_dir: String,
    import_torrent: Option<Torrent>,
    torrent_to_delete: Option<usize>,
    zoom: f32,
    peer_id: String,
}


impl Default for MyApp {
    fn default() -> Self {
        use rand::distributions::{Alphanumeric, DistString};
        Self {
            torrents: Vec::new(),
            selected_row: None,
            user_msg: None,
            inited: false,
            import_opened: false,
            import_dest_dir: String::new(),
            import_torrent: None,
            torrent_to_delete: None,
            zoom: 1.0,
            peer_id: Alphanumeric.sample_string(&mut rand::thread_rng(), 20),
        }
    }
}

impl eframe::App for MyApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.handle_keys(ctx);
        ctx.set_zoom_factor(self.zoom);
        
        if !self.inited {
            self.init(ctx);
        }
        
        if self.import_opened {
            self.import_window(ctx);            
        }

        self.torrent_updates(ctx);

        self.show_message(ctx);
        
        self.top_panel(ctx);
        self.bottom_panel(ctx);
        self.cenral_panel(ctx);
        
    }
    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {

        for q_torrent in &self.torrents {
            match &q_torrent.status {
                DownloadStatus::Downloading => {
                    if let Some(info) = &q_torrent.worker_info {
                        info.sender
                            .send(UiMsg::Stop(q_torrent.pieces_done as u16))
                            .unwrap();
                    }
                }
                DownloadStatus::Paused => {
                    // Data is already backed up
                },
                status => {
                    async_std::task::block_on(Backup::global().backup_torrent(TorrentBackupInfo {
                        pieces_tasks: VecDeque::new(),
                        chunks_tasks: VecDeque::new(),
                        torrent: q_torrent.torrent.clone(),
                        save_path: q_torrent.save_dir.clone(),
                        pieces_done: q_torrent.pieces_done as usize,
                        status: status.clone(),
                    }))
                    .unwrap();
                }
            }
        }

        let app_data = std::mem::take(self); 
        for q_torrent in app_data.torrents {
            match q_torrent.status {
                DownloadStatus::Downloading => {
                    if let Some(info) = q_torrent.worker_info {
                        async_std::task::block_on(info.handle).unwrap();
                    }
                }, 
                _ => {}
            }
        }
    }
}

impl MyApp {
    fn init(&mut self, ctx: &egui::Context) {
        self.inited = true;
        saver::init_saver_globals();
        Backup::init().expect("Saver does not work");
        match async_std::task::block_on(Backup::global().load_config()) {
            Ok(backups) => {
                for backup in backups {
                    self.torrents.push(TorrentDownload {
                        peers: Vec::new(),
                        status: backup.status.clone(),
                        worker_info: None,
                        torrent: backup.torrent.clone(),
                        pieces_done: backup.pieces_done as u32,
                        save_dir: backup.save_path.clone(),
                        last_timestamp: None,
                        download_speed: None,
                        uploaded: 0
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

    fn handle_keys(&mut self, ctx: &egui::Context) {
        let input = ctx.input(|input| input.clone());
        if input.modifiers.contains(Modifiers::CTRL) {
            if input.key_pressed(egui::Key::Plus) {
                self.zoom += 0.1;
            } else if input.key_pressed(egui::Key::Minus) {
                self.zoom -= 0.1;
            }
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
}

fn get_readable_size(bytes: usize, prec: usize) -> String {
    match bytes {
        0..=1023 => format!("{bytes}B"),
        1024..=1_048_575 => format!("{:.1$}KB", bytes as f64 / 1024.0, prec),
        1_048_576..=1_073_741_823 => format!("{:.1$}MB", bytes as f64 / 1_048_576.0, prec),
        _ => format!("{:.1$}GB", bytes as f64 / 1_073_741_824.0, prec),
    }
}
