pub mod tasks;

use std::io::ErrorKind;
use std::sync::Arc;
use std::time::Duration;

use tokio::sync::mpsc::Sender;

use crate::logger::{LogLevel, log};
use super::peers::{Peer, PeerMessage, PeerStatus};
use super::torrent::Torrent;
use super::DownloadEvents;
pub use tasks::{ChunksTask, PieceTask};

#[derive(Debug)]
pub struct DownloadReq {
    pub torrent: Arc<Torrent>,
    pub peer: Peer,
    pub task: ChunksTask,
}
pub struct DataPiece {
    pub buf: Vec<u8>,
    pub piece_i: u64,
    pub begin: u64,
}

impl DownloadReq {
    pub fn new(torrent: Arc<Torrent>, peer: Peer, task: ChunksTask) -> Self {
        DownloadReq {
            torrent,
            peer,
            task,
        }
    }

    pub async fn request_data(
        mut self,
        error_sender: Sender<DownloadEvents>,
    ) -> anyhow::Result<()> {
        if let PeerStatus::NotConnected | PeerStatus::Choked = self.peer.status {
            log!(LogLevel::Debug, "Peer is not ready for downloading yet");
            if let Err(e) = self.peer.connect(&self.torrent).await {
                match e.downcast_ref::<std::io::Error>() {
                    Some(e) => {
                        if let ErrorKind::BrokenPipe | ErrorKind::NotConnected = e.kind() {
                            self.peer
                                .reconnect(&self.torrent, Duration::from_secs(2))
                                .await?;
                            log!(LogLevel::Debug, "Reconnected to peer");
                        } else {
                            
                            error_sender
                                .send(DownloadEvents::PeerAdd(self.peer))
                                .await?;
                            anyhow::bail!("Peer error: {}", e);
                        }
                    }
                    None => {
                        log!(LogLevel::Error, "{e}");
                        match e.downcast_ref::<tokio::time::error::Elapsed>() {
                            Some(_) => {
                                log!(LogLevel::Debug, "Delay eror");
                                anyhow::bail!("Peer: {} is removed", self.peer.peer_addr);
                            },
                            None => {
                                if e.to_string() != "Failed to unchoke peer" {
                                    error_sender
                                    .send(DownloadEvents::PeerAdd(self.peer))
                                    .await?;
                                    anyhow::bail!("Unknown peer error: {}", e);
                                } else {
                                    anyhow::bail!("Peer: {} is removed", self.peer.peer_addr);
                                }
                            }
                        };
                        
                    }
                }
            };
        }
        log!(
            LogLevel::Debug,
            "Downloading piece {}, chunks {:?}",
            self.task.piece_i,
            self.task.chunks
        );
        let mut begin = super::CHUNK_SIZE * self.task.chunks.start;
        for i in self.task.chunks.clone() {
            let length = if i + 1 == self.task.chunks.end && self.task.includes_last_chunk {
                if self.task.piece_i as usize == self.torrent.info.piece_hashes.len() - 1 {
                    self.torrent.info.length
                        - (self.torrent.info.piece_hashes.len() - 1) as u64
                            * self.torrent.info.piece_length
                        - i * super::CHUNK_SIZE
                } else {
                    self.torrent.info.piece_length - i * super::CHUNK_SIZE
                }
            } else {
                super::CHUNK_SIZE
            };
            let mut buf = Vec::new();
            buf.extend_from_slice(&(self.task.piece_i as u32).to_be_bytes());
            buf.extend_from_slice(&(begin as u32).to_be_bytes());
            buf.extend_from_slice(&(length as u32).to_be_bytes());
            if let Err(e) = self.peer.send_message(&PeerMessage::Request(buf)).await {
                log!(
                    LogLevel::Error,
                    "Failed to request download: {}, peers addr: {}",
                    e,
                    self.peer.peer_addr
                );
                match e.downcast_ref::<std::io::Error>() {
                    Some(e) => {
                        if let ErrorKind::BrokenPipe | ErrorKind::NotConnected = e.kind() {
                            self.peer
                                .reconnect(&self.torrent, Duration::from_secs(2))
                                .await?;
                            log!(LogLevel::Debug, "Reconnected to peer");
                        } else {
                            error_sender
                                .send(DownloadEvents::PeerAdd(self.peer))
                                .await?;
                            anyhow::bail!("Peer error: {}", e)
                        }
                    }
                    None => {
                        error_sender
                            .send(DownloadEvents::PeerAdd(self.peer))
                            .await?;
                        anyhow::bail!("Peer error: {}", e);
                    }
                }
                error_sender
                    .send(DownloadEvents::ChunksFail(self.task.clone()))
                    .await?;
                return Ok(());
            }
            begin += super::CHUNK_SIZE;
            // tokio::time::sleep(tokio::time::Duration::from_millis(200)).await;
        }
        log!(LogLevel::Debug, "Sended all requests");
        if let Err(e) = self
            .peer
            .wait_for_msg(
                &PeerMessage::Piece(Vec::new()),
                (self.task.chunks.end - self.task.chunks.start) as u32,
                Some(Duration::from_secs(60)),
            )
            .await
        {
            log!(
                LogLevel::Error,
                "Failed to download: {}, peer: {}",
                e,
                self.peer.peer_addr
            );
            if let Some(e) = e.downcast_ref::<std::io::Error>() {
                log!(LogLevel::Debug, "error:kind : {}", e.kind());
                if let ErrorKind::ConnectionRefused
                | ErrorKind::ConnectionReset
                | ErrorKind::NotConnected
                | ErrorKind::UnexpectedEof = e.kind()
                {
                    self.peer
                        .reconnect(&self.torrent, Duration::from_secs(2))
                        .await?;
                }
            }

            error_sender
                .send(DownloadEvents::ChunksFail(self.task.clone()))
                .await?;
        } else {
            log!(LogLevel::Debug, "downloaded");
        }
        error_sender
            .send(DownloadEvents::PeerAdd(self.peer))
            .await?;
        Ok(())
    }
}
