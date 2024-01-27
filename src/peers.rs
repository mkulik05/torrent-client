use crate::bencode::BencodeValue;
use crate::torrent::Torrent;
use reqwest::Client;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;


pub struct BlockRequest {
    pub piece_i: u32,
    pub begin: u32,
    pub length: u32,
}
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
        let response = BencodeValue::decode_bencoded_value(&body).0;
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
    pub peer_id: String,
    pub bitfield: Option<Vec<u8>>,
    socket: TcpStream,
}

impl Peer {
    // just for test 9 (with hanshake only)
    pub async fn new(addr: &str, torrent: &Torrent, only_handshake: bool) -> Peer {
        let mut socket = TcpStream::connect(addr).await.unwrap();
        let peer_id = Peer::handshake(&torrent, &mut socket).await;
        if !only_handshake {
            let mut msg_len = [0; 4];
            socket.read_exact(&mut msg_len).await.unwrap();
            let msg_len = u32::from_be_bytes(msg_len);
            let mut data = vec![0; msg_len as usize];
            socket.read_exact(&mut data).await.unwrap();
            assert_eq!(data[0], 5);
            Peer {
                peer_id,
                socket,
                bitfield: Some(data[1..].to_vec()),
            }
        } else {
            Peer {
                peer_id,
                socket,
                bitfield: None,
            }
        }
    }
    pub async fn send_interested_msg(&mut self) {
        self.socket.write_all(&1u32.to_be_bytes()).await.unwrap();
        self.socket.write_all(&2u8.to_be_bytes()).await.unwrap();
        let mut data = [0; 5];
        self.socket.read_exact(&mut data).await.unwrap();
        assert_eq!(data[4], 1); // unchoke message
    }

    pub async fn fetch(&mut self, req: &BlockRequest) -> Vec<u8> {
        self.socket.write_all(&13u32.to_be_bytes()).await.unwrap();
        self.socket.write_all(&6u8.to_be_bytes()).await.unwrap();
        self.socket.write_all(&req.piece_i.to_be_bytes()).await.unwrap();
        self.socket.write_all(&req.begin.to_be_bytes()).await.unwrap();
        self.socket.write_all(&req.length.to_be_bytes()).await.unwrap();
        let mut data = [0; 13];
        self.socket.read_exact(&mut data).await.unwrap();
        assert_eq!(data[4], 7); // piece message
        let mut buf = vec![0; req.length as usize];
        self.socket.read_exact(&mut buf).await.unwrap();
        buf
    }

    async fn handshake(torrent: &Torrent, stream: &mut TcpStream) -> String {
        let mut msg = Vec::new();
        msg.push(b"\x13"[0]); // 0x13 = 19
        msg.extend_from_slice(b"BitTorrent protocol");
        msg.extend_from_slice(&[0; 8]);
        msg.extend_from_slice(&torrent.info_hash);
        msg.extend_from_slice(b"00112233445566770099");
        stream.write_all(&msg).await.unwrap();
        let mut response = [0; 68];
        stream.read_exact(&mut response).await.unwrap();
        let peer_id = &response[response.len() - 20..response.len()];
        hex::encode(peer_id)
    }
}

