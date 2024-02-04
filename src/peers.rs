use crate::download::DataPiece;
use crate::logger::{log, LogLevel};
use crate::torrent::Torrent;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::sync::mpsc::Sender;
use tokio::time::timeout;

const MAX_INTERESTED_ATTEMPTS: u8 = 3;

#[derive(Debug)]
pub struct Peer {
    pub peer_id: Option<String>,
    pub peer_addr: String,
    // can be shorter than it should
    // in case received have request instead of bitfield msg
    pub bitfield: Option<Vec<u8>>,
    pub data_sender: Sender<DataPiece>,
    pub status: PeerStatus,
    pub socket: TcpStream,
}

#[derive(Debug, PartialEq)]
pub enum PeerMessage {
    Choke,
    Unchoke,
    Interested,
    NotInterested,
    Have(u32),
    Bitfield(Vec<u8>),
    Request(Vec<u8>),
    Piece(Vec<u8>),
    Cancel(Vec<u8>),
    KeepAlive,
}

impl std::fmt::Display for PeerMessage {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        let msg = match self {
            PeerMessage::Choke => "choke",
            PeerMessage::Have(_) => "have",
            PeerMessage::Cancel(_) => "cancel",
            PeerMessage::Interested => "interested",
            PeerMessage::KeepAlive => "keep-alive",
            PeerMessage::Piece(_) => "piece",
            PeerMessage::Request(_) => "request",
            PeerMessage::Bitfield(_) => "bitfield",
            PeerMessage::Unchoke => "unckoke",
            PeerMessage::NotInterested => "not-interested",
        };
        write!(f, "{}", msg)
    }
}

#[derive(Debug)]
pub enum PeerStatus {
    NotConnected,
    Choked,
    Unchoked,
}

impl Peer {
    pub async fn new(addr: &str, data_sender: Sender<DataPiece>, dur: Duration) -> anyhow::Result<Peer> {
        let socket = match timeout(dur, TcpStream::connect(addr)).await {
            Ok(res) => res?,
            Err(_) => anyhow::bail!("Connection timeout"),
        };
        Ok(Peer {
            peer_id: None,
            peer_addr: addr.to_owned(),
            socket,
            data_sender,
            bitfield: None,
            status: PeerStatus::NotConnected,
        })
    }
    pub async fn connect(&mut self, torrent: &Torrent) -> anyhow::Result<()> {
        match self.status {
            PeerStatus::Unchoked => return Ok(()),
            PeerStatus::NotConnected => {
                log!(
                    LogLevel::Debug,
                    "Connecting to peer: {}",
                    self.socket.peer_addr()?
                );
                self.peer_id = Some(self.handshake(&torrent).await?);
            },
            PeerStatus::Choked => {}
        }
        let mut timeout = 1;
        let mut attempts_n = 0;
        while attempts_n < MAX_INTERESTED_ATTEMPTS {
            self.send_message(&PeerMessage::Interested).await?;
            let res = self
                .wait_for_msg(&PeerMessage::Unchoke, 1, Some(Duration::from_secs(timeout)))
                .await;
            if let Err(ref e) = res {
                if e.to_string() != "Timeout Error!!!" {
                    res?
                }
            } else {
                break;
            }
            attempts_n += 1;
            timeout += 3;
        }
        if let PeerStatus::Unchoked = self.status {
            Ok(())
        } else {
            anyhow::bail!("Failed to unchoke peer");
        }
    }

    pub async fn wait_for_msg(
        &mut self,
        target_msg: &PeerMessage,
        msg_appear_n: u32,
        msg_timeout: Option<Duration>,
    ) -> anyhow::Result<()> {
        let mut n = 0;
        loop {
            let msg;
            if let Some(delay) = msg_timeout {
                if let Ok(val) = timeout(delay, self.receive_message()).await {
                    msg = val?;
                } else {
                    anyhow::bail!("Timeout Error!!!");
                }
            } else {
                msg = self.receive_message().await?;
            }
            log!(LogLevel::Debug, "Got peer message: {}", msg);
            let msg_str = msg.to_string();
            match msg {
                PeerMessage::Bitfield(buf) => self.bitfield = Some(buf),
                PeerMessage::Have(n) => {
                    if let Some(ref mut bitfield) = self.bitfield {
                        let bitfield_i = n / 8;
                        let bit_n = n as i32 % 8;
                        let mut mask = 128;
                        for _ in 0..(bit_n - 1) {
                            mask >>= 1;
                        }
                        bitfield[bitfield_i as usize] |= mask;
                    } else {
                        let bitfield_i = n / 8;
                        let bit_n = n as i32 % 8;
                        let mut mask = 128;
                        for _ in 0..(bit_n - 1) {
                            mask >>= 1;
                        }
                        let mut bitfield = vec![0u8; bitfield_i as usize];
                        bitfield[bitfield_i as usize] |= mask;
                        self.bitfield = Some(bitfield);
                    }
                }
                PeerMessage::Unchoke => self.status = PeerStatus::Unchoked,
                PeerMessage::Piece(buf) => {
                    let piece_i = u32::from_be_bytes([buf[0], buf[1], buf[2], buf[3]]);
                    let begin = u32::from_be_bytes([buf[4], buf[5], buf[6], buf[7]]);
                    self.data_sender
                        .send(DataPiece {
                            buf: buf[8..].to_vec(),
                            piece_i: piece_i as u64,
                            begin: begin as u64,
                        })
                        .await?;
                }
                PeerMessage::Choke => {
                    self.status = PeerStatus::Choked;
                    log!(LogLevel::Error, "peer {} choked", self.peer_addr);
                    anyhow::bail!("Peer choked");
                }
                PeerMessage::Interested | PeerMessage::Request(_) => {
                    // self.send_message(&PeerMessage::Choke).await?;
                }
                PeerMessage::KeepAlive => {
                    // self.send_message(&PeerMessage::KeepAlive).await?;
                }
                _ => {}
            }
            if msg_str == *target_msg.to_string() {
                n += 1;
                if n >= msg_appear_n {
                    break;
                }
            }
        }
        Ok(())
    }

    pub async fn send_message(&mut self, msg: &PeerMessage) -> anyhow::Result<()> {
        log!(LogLevel::Debug, "Sended msg {}", msg);
        match msg {
            PeerMessage::Interested => {
                let mut buf = Vec::new();
                buf.extend_from_slice(&1u32.to_be_bytes());
                buf.extend_from_slice(&2u8.to_be_bytes());
                self.socket.write_all(&buf).await?;
            }
            PeerMessage::Request(req) => {
                let mut buf = Vec::new();
                buf.extend_from_slice(&((1 + req.len()) as u32).to_be_bytes());
                buf.extend_from_slice(&6u8.to_be_bytes());
                buf.extend_from_slice(req);
                self.socket.write_all(&buf).await?;
            }
            PeerMessage::Choke => {
                let mut buf = Vec::new();
                buf.extend_from_slice(&1u32.to_be_bytes());
                buf.extend_from_slice(&0u8.to_be_bytes());
                self.socket.write_all(&buf).await?;
            }
            PeerMessage::KeepAlive => {
                self.socket.write_all(&0u32.to_be_bytes()).await?;
            }
            PeerMessage::Bitfield(bitfield) => {
                let mut buf = Vec::new();
                buf.extend_from_slice(&((1 + bitfield.len()) as u32).to_be_bytes());
                buf.extend_from_slice(&5u8.to_be_bytes());
                buf.extend_from_slice(bitfield);
                self.socket.write_all(&buf).await?;
            }
            _ => {
                panic!("Unimplemented msg to send: {}", msg)
            }
        }
        Ok(())
    }
    
    async fn receive_message(&mut self) -> anyhow::Result<PeerMessage> {
        let mut data = [0; 5]; // length + msg type
        self.socket.read_exact(&mut data).await?;
        let data_len = u32::from_be_bytes([data[0], data[1], data[2], data[3]]);
        if data_len == 0 {
            return Ok(PeerMessage::KeepAlive);
        }
        log!(LogLevel::Debug, "Received peer msg {}", data[4]);
        match data[4] {
            0 => Ok(PeerMessage::Choke),
            1 => Ok(PeerMessage::Unchoke),
            2 => Ok(PeerMessage::Interested),
            // 3 => Ok(PeerMessage::NotInterested),
            4 => {
                let mut data = [0; 4];
                self.socket.read_exact(&mut data).await?;
                Ok(PeerMessage::Have(u32::from_be_bytes([
                    data[0], data[1], data[2], data[3],
                ])))
            }
            5 => {
                let mut data = vec![0; data_len as usize - 1];
                self.socket.read_exact(&mut data).await?;
                Ok(PeerMessage::Bitfield(data))
            }
            6 => {
                let mut data = vec![0; data_len as usize - 1];
                self.socket.read_exact(&mut data).await?;
                Ok(PeerMessage::Request(data))
            }

            7 => {
                let mut data = vec![0; data_len as usize - 1];
                self.socket.read_exact(&mut data).await?;
                Ok(PeerMessage::Piece(data))
            }
            _ => Ok(PeerMessage::KeepAlive),
        }
    }

    pub fn have_piece(&self, piece_i: usize) -> bool {
        if let Some(ref bitfield) = self.bitfield {
            let bitfield_i = piece_i / 8;
            let bit_i = piece_i as i32 % 8;
            let mut mask = 128;
            for _ in 0..(bit_i - 1) {
                mask >>= 1;
            }
            mask & bitfield[bitfield_i] == mask
        } else {
            false
        }
    }

    async fn handshake(&mut self, torrent: &Torrent) -> anyhow::Result<String> {
        let mut msg = Vec::new();
        msg.push(b"\x13"[0]); // 0x13 = 19
        msg.extend_from_slice(b"BitTorrent protocol");
        msg.extend_from_slice(&[0; 8]);
        msg.extend_from_slice(&torrent.info_hash);
        msg.extend_from_slice(b"00112353448866770099");
        self.socket.write_all(&msg).await?;
        log!(LogLevel::Debug, "Sended handskake");
        let mut response = [0; 68];
        self.socket.read_exact(&mut response).await?;
        log!(LogLevel::Debug, "Received answer handskake");
        let peer_id = &response[response.len() - 20..response.len()];
        self.send_message(&PeerMessage::Bitfield(vec![
            0;
            (torrent.info.piece_hashes.len() as f64 / 8.0).ceil()
                as usize
        ]))
        .await?;
        Ok(hex::encode(peer_id))
    }
}
