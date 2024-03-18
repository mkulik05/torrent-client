use std::fmt::Write;
use std::net::{SocketAddr, UdpSocket};
use std::sync::Arc;
use std::time::Duration;

use rand::distributions::{Alphanumeric, DistString};
use tokio::sync::mpsc::Sender;
use tokio::task::JoinHandle;
use tokio::time::timeout;

use super::bencode::BencodeValue;
use super::download::DataPiece;
use super::logger::{log, LogLevel};
use super::peers::Peer;
use super::torrent::Torrent;
use super::DownloadEvents;

#[derive(Clone)]
pub struct TrackerReq {
    pub info_hash: Vec<u8>,
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

macro_rules! concat_slices {
    ($($slice:expr),*) => {{
        let mut buf = Vec::new();
        $(
            buf.extend_from_slice($slice);
        )*
        buf
    }};
}

impl TrackerReq {
    pub fn init(torrent: &Torrent) -> Self {
        TrackerReq {
            info_hash: torrent.info_hash.clone(),
            peer_id: Alphanumeric.sample_string(&mut rand::thread_rng(), 20),
            port: 6681,
            uploaded: 0,
            downloaded: 0,
            left: torrent.info.length,
            compact: 1,
        }
    }

    pub async fn discover_peers(
        &self,
        torrent: Arc<Torrent>,
        event_sender: Sender<DownloadEvents>,
        data_sender: Sender<DataPiece>,
    ) -> Vec<JoinHandle<()>> {
        let mut handles = Vec::new();
        let max_attempts = 5;
        if let Some(trackers) = &torrent.tracker_urls {
            for tracker in trackers {
                handles.push(self.spawn_tracker_task(
                    tracker,
                    max_attempts,
                    event_sender.clone(),
                    data_sender.clone(),
                ));
                break;
            }
        } else {
            handles.push(self.spawn_tracker_task(
                &torrent.tracker_url,
                max_attempts,
                event_sender,
                data_sender,
            ));
        }
        handles
    }

    fn spawn_tracker_task(
        &self,
        tracker_url: &String,
        max_attempts: usize,
        event_sender: Sender<DownloadEvents>,
        data_sender: Sender<DataPiece>,
    ) -> JoinHandle<()> {
        let req = (*self).clone();
        let tracker_url = tracker_url.clone();
        tokio::spawn(async move {
            let mut attempts_n = 0;
            let mut tracker_resp = None;
            while attempts_n < max_attempts {
                match req.clone().clone().send(&tracker_url).await {
                    Ok(resp) => {
                        tracker_resp = Some(resp);
                        break;
                    }
                    Err(_) => {
                        log!(LogLevel::Info, "Failed tracker: {tracker_url}");
                        attempts_n += 1;
                    }
                }
            }
            log!(LogLevel::Info, "Got response {:?}", tracker_resp);
            if let Some(resp) = tracker_resp {
                for peer in resp.peers {
                    let data_sender = data_sender.clone();
                    let event_sender = event_sender.clone();
                    tokio::spawn(async move {
                        if let Ok(peer) =
                            Peer::new(&peer, data_sender.clone(), Duration::from_secs(2)).await
                        {
                            log!(LogLevel::Info, "ok peer {:?}", peer.peer_addr);
                            event_sender
                                .send(DownloadEvents::PeerAdd(peer, true))
                                .await
                                .unwrap();
                        }
                    });
                }
            }
        })
    }

    async fn send(&self, tracker_url: &String) -> anyhow::Result<TrackerResp> {
        log!(LogLevel::Info, "{tracker_url}");
        if tracker_url.starts_with("udp://") {
            return self.send_udp(tracker_url).await;
        }
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

        let res = timeout(
            Duration::from_secs(5),
            client
                .get(format!(
                    "{}?info_hash={}",
                    tracker_url,
                    self.info_hash.iter().fold(String::new(), |mut s, b| {
                        write!(s, "%{:02x}", b).unwrap();
                        s
                    })
                ))
                .query(params)
                .send(),
        )
        .await;

        match res {
            Err(_) => {
                anyhow::bail!("Timeout error")
            }
            Ok(res) => {
                let body = res?.bytes().await?;
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
    }

    async fn send_udp(&self, tracker_url: &String) -> anyhow::Result<TrackerResp> {
        const MAX_RETRIES: usize = 20;
        let mut timeout_ms: u64 = 100; // Timeout duration in seconds
        let mut retries = 0;

        loop {
            if retries >= MAX_RETRIES {
                return Err(anyhow::anyhow!("Max retries exceeded"));
            }

            let connection_id: u64 = 0x41727101980;
            let action_connect: u32 = 0;
            let action_announce: u32 = 1;
            let transaction_id: u32 = rand::random();

            // Create UDP socket
            let socket = UdpSocket::bind("0.0.0.0:0")?;

            // Connect to tracker
            socket.connect(tracker_url.strip_prefix("udp://").unwrap())?;
            let connect_packet = concat_slices![
                &connection_id.to_be_bytes(),
                &action_connect.to_be_bytes(),
                &transaction_id.to_be_bytes()
            ];

            socket.send(&connect_packet)?;

            // Set read timeout
            socket.set_read_timeout(Some(Duration::from_millis(timeout_ms)))?;

            // Receive response
            let mut buf = [0u8; 1024];
            match socket.recv_from(&mut buf) {
                Ok((bytes_read, _)) if bytes_read < 16 => {
                    anyhow::bail!("Invalid response received");
                }
                Ok((bytes_read, _)) => {
                    let received_transaction_id =
                        u32::from_be_bytes([buf[4], buf[5], buf[6], buf[7]]);
                    let received_action = u32::from_be_bytes([buf[0], buf[1], buf[2], buf[3]]);
                    if received_transaction_id != transaction_id
                        || received_action != action_connect
                    {
                        anyhow::bail!("Invalid response received");
                    }
                    let received_connection_id = u64::from_be_bytes([
                        buf[8], buf[9], buf[10], buf[11], buf[12], buf[13], buf[14], buf[15],
                    ]);

                    log!(
                        LogLevel::Info,
                        "{} {:?}",
                        received_connection_id,
                        &self.info_hash
                    );

                    // Announce
                    let announce_packet = concat_slices![
                        &received_connection_id.to_be_bytes(),
                        &action_announce.to_be_bytes(),
                        &transaction_id.to_be_bytes(),
                        &self.info_hash,
                        &vec![0u8; 20],         // Peer ID
                        &0i64.to_be_bytes(),    // Downloaded
                        &0i64.to_be_bytes(),    // Left
                        &0i64.to_be_bytes(),    // Uploaded
                        &0i32.to_be_bytes(),    // Event
                        &0i32.to_be_bytes(),    // IP Address
                        &0i32.to_be_bytes(),    // Key
                        &(-1i32).to_be_bytes(), // Num Want
                        &6881u16.to_be_bytes()  // Port
                    ];
                    socket.send(&announce_packet)?;

                    // Receive announce response
                    match socket.recv_from(&mut buf) {
                        Ok((bytes_read, _)) if bytes_read < 20 => {
                            anyhow::bail!("Invalid announce response received");
                        }
                        Ok((bytes_read, _)) => {
                            let received_transaction_id =
                                u32::from_be_bytes([buf[4], buf[5], buf[6], buf[7]]);
                            let received_action =
                                u32::from_be_bytes([buf[0], buf[1], buf[2], buf[3]]);
                            if received_transaction_id != transaction_id
                                || received_action != action_announce
                            {
                                anyhow::bail!("Invalid announce response received");
                            }
                            let interval = u32::from_be_bytes([buf[8], buf[9], buf[10], buf[11]]);
                            let mut peers = Vec::new();
                            let mut offset = 20;
                            while offset + 6 < bytes_read {
                                let ip_bytes: [u8; 4] = [
                                    buf[offset],
                                    buf[offset + 1],
                                    buf[offset + 2],
                                    buf[offset + 3],
                                ];
                                let port = u16::from_be_bytes([buf[offset + 4], buf[offset + 5]]);
                                let peer_addr = format!(
                                    "{}.{}.{}.{}:{}",
                                    ip_bytes[0], ip_bytes[1], ip_bytes[2], ip_bytes[3], port
                                );
                                peers.push(peer_addr);
                                offset += 6;
                            }
                            return Ok(TrackerResp {
                                interval: interval as i64,
                                peers,
                            });
                        }
                        Err(_) => {
                            timeout_ms += 100;
                            retries += 1;
                            continue; // Retry
                        }
                    }
                }
                Err(_) => {
                    timeout_ms += 100;
                    retries += 1;
                    continue; // Retry
                }
            }
        }
    }
}