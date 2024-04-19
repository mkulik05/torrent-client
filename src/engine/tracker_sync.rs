use std::collections::HashMap;
use once_cell::sync::OnceCell;
use tokio::sync::{mpsc::{Receiver, Sender}, RwLock};
use super::torrent::{self, Torrent};

pub enum DownloadStatus {
    // hash as param
    Stopped(String),
    Finished(String),
    PieceDone(String),
}

struct TrackerInfo {
    interval: u32,

    // info hashes of torrents
    requests_queue: Vec<String>,
}

struct TrackerReqInfo {
    peer_id: String,
    uploaded: u32,
    downloaded: u32, 
    left: u32
}

pub enum ToTrackerMsg {
    TorrentStarted(Torrent, Sender<String>),
    BytesDownloaded(String, u32),
    BytesUploaded(String, u32)
}

// to store peers for each torrent
static INSTANCE: OnceCell<RwLock<HashMap<String, Vec<String>>>> = OnceCell::new();

pub async fn init_sync(mut rcv: Receiver<ToTrackerMsg>) -> anyhow::Result<()> {
    INSTANCE.set(RwLock::new(HashMap::new())).unwrap();

    let mut trackers = HashMap::<String, TrackerInfo>::new();

    // info_hash => info for torrent
    let mut torrents = HashMap::<String, TrackerReqInfo>::new();

    loop {
        while let Some(tor_msg) = rcv.recv().await {
            match tor_msg {
                ToTrackerMsg::BytesDownloaded(info_hash, n) => {
                    let info = torrents.get_mut(&info_hash);
                    if let Some(info) = info {
                        info.downloaded += n;
                        info.left -= n;
                    }
                },
                ToTrackerMsg::BytesUploaded(info_hash, n) => {
                    let info = torrents.get_mut(&info_hash);
                    if let Some(info) = info {
                        info.uploaded += n;
                    }  
                },
                ToTrackerMsg::TorrentStarted(torrent, sender) => {
                    
                }
            }
        }
    }

    Ok(())
}