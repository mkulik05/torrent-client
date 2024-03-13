use crate::gui::MyApp;
use crate::gui::{TorrentInfo, DownloadStatus, UiHandle, UiMsg, WorkerInfo, TorrentDownload};
use crate::engine::backup;
use crate::engine::{
    download_torrent,
    logger::{log, LogLevel},
};
use eframe::egui;
use tokio::sync::broadcast;

impl MyApp {
    pub fn start_download(&mut self, torrent_info: TorrentInfo, ctx: &egui::Context) {
        let torrent = match &torrent_info {
            TorrentInfo::Torrent(torrent) => torrent.clone(),
            TorrentInfo::Backup(backup) => backup.torrent.clone(),
        };

        let (sender, receiver) = broadcast::channel(20_000);
        let folder = if let TorrentInfo::Backup(ref backup) = torrent_info {
            backup.save_path.clone()
        } else {
            self.import_dest_dir.clone()
        };
        let handle = {
            let folder = folder.clone();
            let name = torrent.info.name.clone();
            let sender = sender.clone();
            let ctx = ctx.clone();
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
                save_dir: folder
            });
        } else {
            self.torrents[torrent_i.unwrap()].worker_info = Some(info);
        }
    }
    pub fn pause_torrent(&mut self, i: usize) {
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
        let backup = backup::load_backup(&self.torrents[i].torrent.info_hash);
        log!(LogLevel::Info, "{:?}", backup);
        match backup {
            Ok(backup) => {
                self.start_download(TorrentInfo::Backup(backup), ctx);
            }
            Err(_) => {
                self.start_download(
                    TorrentInfo::Torrent(self.torrents[i].torrent.clone()),
                    ctx,
                );
            }
        }
        self.torrents[i].status = DownloadStatus::Downloading;
    }

    pub fn delete_torrent(&mut self, i: usize) {
        if let Some(ref info) = self.torrents[i].worker_info {
            info.sender.send(UiMsg::ForceOff).unwrap();
        }
        backup::remove_torrent(&self.torrents[i].torrent.info_hash).unwrap();
        self.torrents.remove(i);
    }
    pub fn torrent_updates(&mut self) {
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
    }
}