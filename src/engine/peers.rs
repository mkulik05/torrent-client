use async_recursion::async_recursion;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::sync::mpsc::Sender;
use tokio::time::timeout;

use super::download::DataPiece;
use super::logger::{log, LogLevel};
use super::torrent::Torrent;
use crate::engine::download::tasks::CHUNK_SIZE;
use crate::engine::saver;
use crate::engine::torrent::PieceBitmap;
use crate::gui::UiMsg;

const MAX_INTERESTED_ATTEMPTS: u8 = 3;

#[derive(Debug)]
pub struct Peer {
    pub peer_id: Option<String>,
    pub my_peer_id: String,
    pub peer_addr: String,
    // can be shorter than it should
    // in case received have request instead of bitfield msg
    pub peer_bitfield: Option<Vec<u8>>,
    pub own_bitfield: PieceBitmap,
    pub data_sender: Sender<DataPiece>,
    pub status: PeerStatus,
    pub socket: TcpStream,
    pub info_hash: String,
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
    pub async fn new(
        addr: &str,
        data_sender: Sender<DataPiece>,
        peer_id: String,
        info_hash: String,
        dur: Duration,
    ) -> anyhow::Result<Peer> {
        let socket = match timeout(dur, TcpStream::connect(addr)).await {
            Ok(res) => res?,
            Err(_) => anyhow::bail!("Connection timeout"),
        };
        Ok(Peer {
            peer_id: None,
            peer_addr: addr.to_owned(),
            my_peer_id: peer_id,
            socket,
            info_hash,
            data_sender,
            peer_bitfield: None,
            own_bitfield: PieceBitmap::new(1),
            status: PeerStatus::NotConnected,
        })
    }
    pub async fn reconnect(&mut self, torrent: &Torrent, dur: Duration) -> anyhow::Result<()> {
        self.socket = match timeout(dur, TcpStream::connect(&self.peer_addr)).await {
            Ok(res) => res?,
            Err(_) => anyhow::bail!("Connection timeout"),
        };
        self.status = PeerStatus::NotConnected;
        self.connect(torrent).await?;
        Ok(())
    }

    #[async_recursion]
    pub async fn connect(&mut self, torrent: &Torrent) -> anyhow::Result<()> {
        match self.status {
            PeerStatus::Unchoked => Ok(()),
            PeerStatus::NotConnected => {
                log!(
                    LogLevel::Debug,
                    "Connecting to peer: {}",
                    self.socket.peer_addr()?
                );
                self.peer_id = Some(self.handshake(torrent, Duration::from_secs(4)).await?);
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
            PeerStatus::Choked => {
                log!(LogLevel::Debug, "Peer is choked, have to fix");
                self.reconnect(torrent, Duration::from_secs(2)).await?;
                Ok(())
            }
        }
    }

    pub async fn sync_bitmaps(&mut self, b2: &PieceBitmap) -> anyhow::Result<()> {
        let diff = self.own_bitfield.diff(&b2);
        if diff.len() > 40 {
            for i in diff {
                self.send_message(&PeerMessage::Have(i as u32)).await?;
                self.own_bitfield.add(i);
            }
        }
        Ok(())
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
            let msg_str = msg.to_string();
            match msg {
                PeerMessage::Bitfield(buf) => self.peer_bitfield = Some(buf),
                PeerMessage::Have(n) => {
                    if let Some(ref mut bitfield) = self.peer_bitfield {
                        let bitfield_i = n / 8;
                        if bitfield_i as usize <= bitfield.len() {
                            let bit_n = n as i32 % 8;
                            let mut mask = 128;
                            for _ in 0..(bit_n - 1) {
                                mask >>= 1;
                            }
                            bitfield[bitfield_i as usize] |= mask;
                        }
                    } else {
                        let bitfield_i = n / 8;
                        let bit_n = n as i32 % 8;
                        let mut mask = 128;
                        for _ in 0..(bit_n - 1) {
                            mask >>= 1;
                        }
                        let mut bitfield = vec![0u8; (bitfield_i + 1) as usize];
                        bitfield[bitfield_i as usize] |= mask;
                        self.peer_bitfield = Some(bitfield);
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
                PeerMessage::Interested => {
                    self.send_message(&PeerMessage::Unchoke).await?;
                }
                PeerMessage::Request(buf) => {
                    log!(LogLevel::Debug, "Got request msg!!!");
                    if buf.len() > 11 {
                        let piece_i = u32::from_be_bytes([buf[0], buf[1], buf[2], buf[3]]);
                        let begin = u32::from_be_bytes([buf[4], buf[5], buf[6], buf[7]]);
                        let length = u32::from_be_bytes([buf[8], buf[9], buf[10], buf[11]]);
                        if length as u64 <= CHUNK_SIZE {
                            let Some(save_info) = saver::SAVE_INFO.get() else {
                                continue;
                            };
                            let hashmap = save_info.read().await;
                            let Some(save_info) = hashmap.get(&self.info_hash)
                            else {
                                continue;
                            };
                            let mut buf = Vec::new();
                            if let Some(size_progression) = &save_info.size_progression {
                                if let Ok(_) = saver::read_piece_from_files(
                                    &save_info.save_path,
                                    &save_info.torrent.clone(),
                                    piece_i as u64,
                                    begin as u64,
                                    length as u64,
                                    &mut buf,
                                    size_progression,
                                ) {

                                }
                            } else {
                                use std::fs::File;
                                use std::io::{Seek, Read};
                                let Ok(mut file) = File::options()
                                    .read(true)
                                    .write(true)
                                    .create(false)
                                    .open(&save_info.save_path) else {continue};
                                buf = vec![0u8; length as usize];
                                let Ok(_) = file.seek(std::io::SeekFrom::Start(begin as u64)) else {continue};
                                let Ok(_) = file.read_exact(&mut buf) else {continue};
                            }
                            let mut req = Vec::new();
                            req.extend_from_slice(&piece_i.to_be_bytes());
                            req.extend_from_slice(&begin.to_be_bytes());
                            req.extend_from_slice(&buf);

                            let _ = self.send_message(&PeerMessage::Piece(req)).await;
                            let _ = save_info.ui_h.send_with_update(UiMsg::DataUploaded(length as u64));
                            log!(LogLevel::Debug, "Data sent");
                        }
                    }
                }
                PeerMessage::KeepAlive => {}
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
            PeerMessage::Unchoke => {
                let mut buf = Vec::new();
                buf.extend_from_slice(&1u32.to_be_bytes());
                buf.extend_from_slice(&1u8.to_be_bytes());
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
            PeerMessage::Have(i) => {
                let mut buf = Vec::new();
                buf.extend_from_slice(&1u32.to_be_bytes());
                buf.extend_from_slice(&4u8.to_be_bytes());
                buf.extend_from_slice(&(*i).to_be_bytes());
                self.socket.write_all(&buf).await?;
            },
            PeerMessage::Piece(req) => {
                let mut buf = Vec::new();
                buf.extend_from_slice(&((1 + req.len()) as u32).to_be_bytes());
                buf.extend_from_slice(&7u8.to_be_bytes());
                buf.extend_from_slice(req);
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
        if let Some(ref bitfield) = self.peer_bitfield {
            let bitfield_i = piece_i / 8;
            let bit_i = piece_i as i32 % 8;
            let mut mask = 128;
            for _ in 0..(bit_i - 1) {
                mask >>= 1;
            }
            mask & bitfield[bitfield_i] == mask
        } else {
            // cause in the start bitfield is not inited
            true
        }
    }

    async fn handshake(&mut self, torrent: &Torrent, time: Duration) -> anyhow::Result<String> {
        let mut msg = Vec::new();
        msg.push(b"\x13"[0]); // 0x13 = 19
        msg.extend_from_slice(b"BitTorrent protocol");
        msg.extend_from_slice(&[0; 8]);
        msg.extend_from_slice(&torrent.info_hash);
        msg.extend_from_slice(&self.my_peer_id.as_bytes());
        self.socket.write_all(&msg).await?;
        log!(LogLevel::Debug, "Sended handskake");
        let mut response = [0; 68];
        let _ = timeout(time, self.socket.read_exact(&mut response)).await?;
        log!(LogLevel::Debug, "Received answer handskake");
        let peer_id = &response[response.len() - 20..response.len()];
        self.send_message(&PeerMessage::Bitfield(self.own_bitfield.bitmap.clone()))
            .await?;
        Ok(hex::encode(peer_id))
    }
}
