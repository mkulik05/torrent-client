use crate::engine::backup::Backup;
use crate::engine::{
    download_torrent,
    logger::{log, LogLevel},
};
use crate::gui::MyApp;
use crate::gui::{DownloadStatus, TorrentDownload, TorrentInfo, UiHandle, UiMsg, WorkerInfo};
use eframe::egui;
use tokio::sync::broadcast;

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
                if let Err(e) = download_torrent(torrent_info, &folder, ui_handle.clone(), peer_id).await {
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
        if torrent_i.is_none() {
            self.torrents.push(TorrentDownload {
                peers: Vec::new(),
                torrent,
                status: DownloadStatus::Downloading,
                worker_info: Some(info),
                pieces_done: 0,
                save_dir: folder,
                uploaded: 0,
            });
        } else {
            self.torrents[torrent_i.unwrap()].worker_info = Some(info);
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
    pub fn torrent_updates(&mut self) {
        for torrent in &mut self.torrents {
            if torrent.worker_info.is_none() {
                continue;
            }
            while let Ok(msg) = torrent.worker_info.as_mut().unwrap().receiver.try_recv() {
                if let DownloadStatus::Downloading = torrent.status {
                    match msg {
                        UiMsg::PieceDone(_) => {
                            torrent.pieces_done += 1;
                            if torrent.pieces_done == torrent.torrent.info.piece_hashes.len() as u32
                            {
                                torrent.status = DownloadStatus::Finished;
                            }
                        }
                        UiMsg::DataUploaded(n) => {
                            torrent.uploaded += n as u32;
                        }
                        UiMsg::TorrentFinished => {
                            torrent.peers.clear();
                            torrent.status = DownloadStatus::Finished;
                            torrent.pieces_done = torrent.torrent.info.piece_hashes.len() as u32;
                        }
                        UiMsg::TorrentErr(msg) => torrent.status = DownloadStatus::Error(msg),
                        UiMsg::PeerDiscovered(peer) => {
                            if torrent.peers.iter().position(|x| *x == peer).is_none() {
                                torrent.peers.push(peer);
                            }
                        }
                        _ => {}
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
