use crate::bencode::{decode_bencoded_value, BencodeValue};
use crate::torrent::Torrent;
use tokio::net::TcpStream;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use reqwest::Client;
pub struct TrackerReq {
    pub info_hash: String,
    pub peer_id: String,
    pub port: usize,
    pub uploaded: usize,
    pub downloaded: usize,
    pub left: usize,
    compact: usize,
}
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
            peer_id: "00112253445566770099".to_string(),
            port: 6681,
            uploaded: 0,
            downloaded: 0,
            left: torrent.info.length as usize,
            compact: 1,
        }
    }
    pub async fn send(&self, torrent: &Torrent) -> TrackerResp {
        let params = &[
            ("peer_id", self.peer_id.clone()),
            ("port", self.port.to_string()),
            ("uploaded", self.uploaded.to_string()),
            ("downloaded", self.downloaded.to_string()),
            ("left", self.left.to_string()),
            ("compact", self.compact.to_string()),
        ];
        let client = Client::new();
        let body = client
            .get(format!(
                "{}?info_hash={}",
                torrent.tracker_url, self.info_hash
            ))
            .query(params)
            .send()
            .await
            .unwrap()
            .bytes()
            .await
            .unwrap();
        let response = decode_bencoded_value(&body).0;
        let BencodeValue::Bytes(ref peers_bytes) = response["peers"] else {
            panic!("Error")
        };
        let BencodeValue::Num(interval) = response["interval"] else {
            panic!("Error")
        };
        let peers_n = peers_bytes.len() / 6;
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
        TrackerResp { interval, peers }
    }
}

pub struct Peer {
    pub addr: String,
}

impl Peer {
    pub async fn handshake(&self, torrent: &Torrent) -> String {
        let mut msg = Vec::new();
        msg.push(b"\x13"[0]); // 0x13 = 19
        msg.extend_from_slice(b"BitTorrent protocol");
        msg.extend_from_slice(&[0; 8]);
        msg.extend_from_slice(&torrent.info_hash);
        msg.extend_from_slice(b"00112233445566770099");
        let mut stream = TcpStream::connect(&self.addr).await.unwrap();
        stream.write_all(&msg).await.unwrap();
        let mut response = [0; 68];
        stream.read_exact(&mut response).await.unwrap();
        let peer_id = &response[response.len() - 20..response.len()];
        hex::encode(peer_id)
    }
}