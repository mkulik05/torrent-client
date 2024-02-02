use crate::logger::{log, LogLevel};
use crate::peers::{Peer, PeerMessage, PeerStatus};
use crate::torrent::Torrent;
use crate::DownloadStatus;
use std::io::ErrorKind;
use std::ops::Range;
use std::sync::Arc;
use tokio::sync::mpsc::Sender;

#[derive(Debug)]
pub struct PieceTask {
    pub piece_i: u64,
    pub total_chunks: u64,
    pub chunks_done: u64,
}

#[derive(Debug, Clone)]
pub struct ChunksTask {
    pub piece_i: u64,
    pub chunks: Range<u64>,
    pub includes_last_chunk: bool,
}

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
        error_sender: Sender<DownloadStatus>,
    ) -> anyhow::Result<()> {
        if let PeerStatus::NotConnected | PeerStatus::Choked = self.peer.status {
            log!(LogLevel::Debug, "Peer is not ready for downloading yet");
            self.peer.connect(&self.torrent).await?;
        }
        log!(
            LogLevel::Debug,
            "Downloading piece {}, chunks {:?}",
            self.task.piece_i,
            self.task.chunks
        );
        let mut begin = crate::CHUNK_SIZE * self.task.chunks.start;
        for i in self.task.chunks.clone() {
            let length = if i + 1 == self.task.chunks.end && self.task.includes_last_chunk {
                if self.task.piece_i as usize == self.torrent.info.piece_hashes.len() - 1 {
                    log!(LogLevel::Debug, "Got there!!!");
                    self.torrent.info.length
                        - (self.torrent.info.piece_hashes.len() - 1) as u64
                            * self.torrent.info.piece_length
                        - i * crate::CHUNK_SIZE
                } else {
                    self.torrent.info.piece_length - i * crate::CHUNK_SIZE
                }
            } else {
                crate::CHUNK_SIZE
            };
            let mut buf = Vec::new();
            buf.extend_from_slice(&(self.task.piece_i as u32).to_be_bytes());
            buf.extend_from_slice(&(begin as u32).to_be_bytes());
            buf.extend_from_slice(&(length as u32).to_be_bytes());
            if let Err(e) = self.peer.send_message(&PeerMessage::Request(buf)).await {
                log!(LogLevel::Error, "Failed to request download: {}, peers addr: {}", e, self.peer.peer_addr);
                match e.downcast_ref::<std::io::Error>() {
                    Some(e) => {
                        if e.kind() == ErrorKind::BrokenPipe {
                            let peer = Peer::new(&self.peer.peer_addr, self.peer.data_sender).await?;
                            log!(LogLevel::Debug, "Reconnected to peer");
                            error_sender.send(DownloadStatus::PeerFreed(peer)).await?;
                        } else {
                            error_sender.send(DownloadStatus::PeerFreed(self.peer)).await?;
                        }
                    },
                    None => {
                        error_sender.send(DownloadStatus::PeerFreed(self.peer)).await?;
                    }  
                }
                error_sender
                    .send(DownloadStatus::ChunksFail(self.task.clone()))
                    .await?;
                return Ok(());
            }
            begin += crate::CHUNK_SIZE;
            tokio::time::sleep(tokio::time::Duration::from_millis(200)).await;
        }
        log!(LogLevel::Debug, "Sended all requests");
        if let Err(e) = self
            .peer
            .wait_for_msg(
                &PeerMessage::Piece(Vec::new()),
                (self.task.chunks.end - self.task.chunks.start) as u32,
                Some(std::time::Duration::from_secs(60)),
            )
            .await
        {
            log!(LogLevel::Error, "Failed to download: {}, peer: {}", e, self.peer.peer_addr);
            error_sender.send(DownloadStatus::PeerFreed(self.peer)).await?;
            error_sender
                .send(DownloadStatus::ChunksFail(self.task.clone()))
                .await?;
        }
        Ok(())
    }
}
