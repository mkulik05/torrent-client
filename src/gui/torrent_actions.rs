use std::time::Instant;

use crate::engine::backup::Backup;
use crate::engine::{
    download_torrent,
    logger::{log, LogLevel},
};
use crate::gui::MyApp;
use crate::gui::{DownloadStatus, TorrentDownload, TorrentInfo, UiHandle, UiMsg, WorkerInfo};
use eframe::egui;
use tokio::sync::broadcast;

use super::{TimeStamp, PIECES_TO_TIME_MEASURE};

impl MyApp {
    pub fn start_download(&mut self, torrent_info: TorrentInfo, ctx: &egui::Context) {
        let torrent = match &torrent_info {
            TorrentInfo::Torrent(torrent) => torrent.clone(),
            TorrentInfo::Backup(backup) => backup.torrent.clone(),
        };

        let (sender, receiver) = broadcast::channel(20_000);
        let folder = match torrent_info {
            TorrentInfo::Backup(ref backup) => backup.save_path.clone(),

            TorrentInfo::Torrent(ref torrent) => {
                let pos = self
                    .torrents
                    .iter()
                    .position(|x| x.torrent.info_hash == torrent.info_hash);
                if let Some(i) = pos {
                    self.torrents[i].save_dir.clone()
                } else {
                    self.import_dest_dir.clone()
                }
            }
        };
        let handle = {
            let folder = folder.clone();
            let name = torrent.info.name.clone();
            let sender = sender.clone();
            let peer_id = self.peer_id.clone();
            let ctx = ctx.clone();
            tokio::spawn(async move {
                log!(LogLevel::Info, "Strating torrent downloading: {name}");
                let ui_handle = UiHandle {
                    ui_sender: sender,
                    ctx,
                };
                if let Err(e) =
                    download_torrent(torrent_info, &folder, ui_handle.clone(), peer_id).await
                {
                    log!(LogLevel::Fatal, "Failed to download torrent: {e}");
                    ui_handle
                        .send_with_update(UiMsg::TorrentErr(e.to_string()))
                        .unwrap();
                }
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

        let timestamp = Some(TimeStamp {
            time: Instant::now(),
            pieces_n: 0,
        });
        if torrent_i.is_none() {
            self.torrents.push(TorrentDownload {
                peers: Vec::new(),
                torrent,
                status: DownloadStatus::Downloading,
                worker_info: Some(info),
                pieces_done: 0,
                download_speed: None,
                save_dir: folder,
                last_timestamp: timestamp,
                uploaded: 0,
            });
        } else {
            let i = torrent_i.unwrap();
            self.torrents[i].worker_info = Some(info);
            self.torrents[i].last_timestamp = timestamp;
        }
    }
    pub fn pause_torrent(&mut self, i: usize) {
        self.torrents[i].peers.clear();
        if let Some(ref info) = self.torrents[i].worker_info {
            log!(LogLevel::Info, "Sended msg!!!");
            info.sender
                .send(UiMsg::Pause(self.torrents[i].pieces_done as u16))
                .unwrap();
            log!(LogLevel::Info, "Finished: sended msg!!!");
            self.torrents[i].status = DownloadStatus::Paused;
        }
    }

    pub fn resume_torrent(&mut self, i: usize, ctx: &egui::Context) {
        let backup = async_std::task::block_on(
            Backup::global().load_backup(&self.torrents[i].torrent.info_hash),
        );
        match backup {
            Ok(backup) => {
                self.start_download(TorrentInfo::Backup(backup), ctx);
            }
            Err(_) => {
                self.start_download(TorrentInfo::Torrent(self.torrents[i].torrent.clone()), ctx);
            }
        }
        self.torrents[i].status = DownloadStatus::Downloading;
    }

    pub fn delete_torrent(&mut self, i: usize) {
        if let Some(ref info) = self.torrents[i].worker_info {
            info.sender.send(UiMsg::ForceOff).unwrap();
        }
        async_std::task::block_on(
            Backup::global().remove_torrent(&self.torrents[i].torrent.info_hash),
        )
        .unwrap();
        self.torrents.remove(i);
        if self.selected_row.is_some() {
            let row = self.selected_row.unwrap();
            if row == i {
                self.selected_row = None;
                return;
            }
            if row > i {
                self.selected_row = Some(row - 1);
            }
        }
    }
    pub fn torrent_updates(&mut self, ctx: &egui::Context) {
        for t_i in 0..self.torrents.len() {
            if self.torrents[t_i].worker_info.is_none() {
                continue;
            }
            while let Ok(msg) = self.torrents[t_i]
                .worker_info
                .as_mut()
                .unwrap()
                .receiver
                .try_recv()
            {
                if let DownloadStatus::Downloading = self.torrents[t_i].status {
                    let mut done_piece = false;
                    match msg {
                        UiMsg::PieceDone(_) => {
                            done_piece = true;
                            self.torrents[t_i].pieces_done += 1;
                            log!(LogLevel::Info, "Donwloaded: {}", self.torrents[t_i].pieces_done);
                            if self.torrents[t_i].last_timestamp.is_some() {
                                let info = self.torrents[t_i].last_timestamp.as_ref().unwrap();
                                let pieces_done_from_timestamp =
                                    self.torrents[t_i].pieces_done - info.pieces_n;
                                if pieces_done_from_timestamp >= PIECES_TO_TIME_MEASURE as u32 {
                                    let time_per_piece = info.time.elapsed().as_millis()
                                        / PIECES_TO_TIME_MEASURE as u128;
                                    if self.torrents[t_i].download_speed.is_none() {
                                        self.torrents[t_i].download_speed = Some(time_per_piece as u16);
                                        continue;
                                    }
                                    log!(
                                        LogLevel::Debug,
                                        "Peice download time is {}ms for torrent {}",
                                        time_per_piece,
                                        self.torrents[t_i].torrent.info.name
                                    );
                                    log!(LogLevel::Info, "curr speed: {time_per_piece}, result: {}", self.torrents[t_i].download_speed.unwrap());

                                    if time_per_piece as f64
                                        / self.torrents[t_i].download_speed.unwrap() as f64
                                        >= 3.0
                                    {
                                        self.pause_torrent(t_i);
                                        self.resume_torrent(t_i, ctx)
                                    }

                                    self.torrents[t_i].last_timestamp = Some(TimeStamp {
                                        time: Instant::now(),
                                        pieces_n: self.torrents[t_i].pieces_done,
                                    });
                                }
                            } else {
                                self.torrents[t_i].last_timestamp = Some(TimeStamp {
                                    time: Instant::now(),
                                    pieces_n: self.torrents[t_i].pieces_done,
                                });
                            }

                            if self.torrents[t_i].pieces_done
                                == self.torrents[t_i].torrent.info.piece_hashes.len() as u32
                            {
                                self.torrents[t_i].status = DownloadStatus::Finished;
                            }
                        }
                        UiMsg::DataUploaded(n) => {
                            self.torrents[t_i].uploaded += n as u32;
                        }
                        UiMsg::TorrentFinished => {
                            self.torrents[t_i].peers.clear();
                            self.torrents[t_i].status = DownloadStatus::Finished;
                            self.torrents[t_i].pieces_done =
                                self.torrents[t_i].torrent.info.piece_hashes.len() as u32;
                        }
                        UiMsg::TorrentErr(msg) => {
                            self.torrents[t_i].status = DownloadStatus::Error(msg)
                        }
                        UiMsg::PeerDiscovered(peer) => {
                            if self.torrents[t_i]
                                .peers
                                .iter()
                                .position(|x| *x == peer)
                                .is_none()
                            {
                                self.torrents[t_i].peers.push(peer);
                            }
                        }
                        UiMsg::PeerDisconnect(peer) => {
                            if let Some(index) =
                                self.torrents[t_i].peers.iter().position(|x| *x == peer)
                            {
                                self.torrents[t_i].peers.remove(index);
                            }
                        }
                        _ => {}
                    }
                    if !done_piece {
                        if self.torrents[t_i].last_timestamp.is_some() {
                            let info = self.torrents[t_i].last_timestamp.as_ref().unwrap();
                            let piece_time = info.time.elapsed().as_millis() / PIECES_TO_TIME_MEASURE as u128;
                    
                            if info.time.elapsed().as_millis() > 30_000 && piece_time >= self.torrents[t_i].download_speed.unwrap_or(0) as u128 {
                                self.pause_torrent(t_i);
                                self.resume_torrent(t_i, ctx)
                            }
                        }
                    }
                }
            }
        }

        if let Some(i) = self.torrent_to_delete {
            self.delete_torrent(i);
            self.torrent_to_delete = None;
        }
    }
}
