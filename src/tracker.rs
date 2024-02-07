use crate::bencode::BencodeValue;
use crate::download::DataPiece;
use crate::logger::{log, LogLevel};
use crate::peers::Peer;
use crate::torrent::Torrent;
use crate::DownloadEvents;
use rand::distributions::{Alphanumeric, DistString};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::mpsc::Sender;
pub struct TrackerReq {
    pub info_hash: String,
    pub peer_id: String,
    pub port: u32,
    pub uploaded: u64,
    pub downloaded: u64,
    pub left: u64,
    compact: u8,
}

#[derive(Debug)]
pub struct TrackerResp {
    pub interval: i64,
    pub peers: Vec<String>,
}

impl TrackerReq {
    pub fn init(torrent: &Torrent) -> Self {
        TrackerReq {
            info_hash: torrent
                .info_hash
                .iter()
                .map(|b| format!("%{:02x}", b))
                .collect(),
            peer_id: Alphanumeric.sample_string(&mut rand::thread_rng(), 20),
            port: 6681,
            uploaded: 0,
            downloaded: 0,
            left: torrent.info.length,
            compact: 1,
        }
    }
    pub async fn send(&self, torrent: &Torrent) -> anyhow::Result<TrackerResp> {
        let params = &[
            ("peer_id", self.peer_id.clone()),
            ("port", self.port.to_string()),
            ("uploaded", self.uploaded.to_string()),
            ("downloaded", self.downloaded.to_string()),
            ("left", self.left.to_string()),
            ("compact", self.compact.to_string()),
        ];
        log!(LogLevel::Debug, "Sending tracker request");
        let client = reqwest::Client::builder()
            .user_agent("my torrent")
            .build()?;
        let body = client
            .get(format!(
                "{}?info_hash={}",
                torrent.tracker_url, self.info_hash
            ))
            .query(params)
            .send()
            .await?
            .bytes()
            .await?;
        let response = BencodeValue::decode_bencoded_value(&body)?.0;
        let BencodeValue::Bytes(ref peers_bytes) = response["peers"] else {
            anyhow::bail!("Invalid torrent response structure");
        };
        let BencodeValue::Num(interval) = response["interval"] else {
            anyhow::bail!("Invalid torrent response structure");
        };
        log!(LogLevel::Debug, "Got valid response");
        let peers_n = peers_bytes.len() / 6;
        log!(LogLevel::Debug, "Found {peers_n} peer(s)");
        let mut peers = Vec::new();
        for i in 0..peers_n {
            peers.push(format!(
                "{}.{}.{}.{}:{}",
                peers_bytes[i * 6],
                peers_bytes[i * 6 + 1],
                peers_bytes[i * 6 + 2],
                peers_bytes[i * 6 + 3],
                u16::from_be_bytes([peers_bytes[i * 6 + 4], peers_bytes[i * 6 + 5]])
            ))
        }
        Ok(TrackerResp { interval, peers })
    }
}

impl TrackerResp {
    // Discover task - found working peers, sep task to immidiatly
    // start working with discovered peers
    pub fn find_working_peers(
        self: Arc<Self>,
        send_data: Sender<DataPiece>,
        send_status: Sender<DownloadEvents>,
    ) {
        tokio::spawn(async move {
            let mut n = 0;
            for peer in self.peers.iter() {
                if let Ok(peer) = Peer::new(&peer, send_data.clone(), Duration::from_secs(2)).await
                {
                    n += 1;
                    send_status
                        .send(DownloadEvents::PeerAdd(peer))
                        .await
                        .unwrap();
                }
            }
            log!(LogLevel::Info, "Connected to {} peer(s)", n);
        });
    }
}
