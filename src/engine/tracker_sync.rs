use std::collections::HashMap;
use once_cell::sync::OnceCell;
use tokio::sync::{mpsc::{Receiver, Sender}, RwLock};
use super::torrent::{self, Torrent};
use std::sync::Arc;
use crate::engine::TrackerReq;

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

pub enum ToTrackerMsg {
    TorrentStarted(Arc<Torrent>, Sender<String>, u64),
    BytesDownloaded(String, u32),
    BytesUploaded(String, u32)
}

// to store peers for each torrent
static INSTANCE: OnceCell<RwLock<HashMap<String, Vec<String>>>> = OnceCell::new();

pub async fn init_sync(mut rcv: Receiver<ToTrackerMsg>, peer_id: String, port: u32) -> anyhow::Result<()> {
    INSTANCE.set(RwLock::new(HashMap::new())).unwrap();

    let mut trackers = HashMap::<String, TrackerInfo>::new();

    // info_hash => info for torrent
    let mut torrents = HashMap::<String, TrackerReq>::new();

    loop {
        while let Some(tor_msg) = rcv.recv().await {
            match tor_msg {
                ToTrackerMsg::BytesDownloaded(info_hash, n) => {
                    let info = torrents.get_mut(&info_hash);
                    if let Some(info) = info {
                        info.downloaded += n as u64;
                        info.left -= n as u64;
                    }
                },
                ToTrackerMsg::BytesUploaded(info_hash, n) => {
                    let info = torrents.get_mut(&info_hash);
                    if let Some(info) = info {
                        info.uploaded += n as u64;
                    }  
                },
                ToTrackerMsg::TorrentStarted(torrent, sender, left) => {
                    // trackers: tracker => interval
                    let tracker_req = TrackerReq::init(&torrent, peer_id.clone(), port, left);
                    let (peers, trackers) = get_trackers_peers(torrent.clone(), sender).await; 
                }
            }
        }
    }

    Ok(())
}

async fn get_trackers_peers(torrent: Arc<Torrent>, sender: Sender<String> ) -> (Vec<String>, HashMap<String, u32>) {
    
    todo!();
}