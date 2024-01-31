use crate::logger::{log, LogLevel};
use crate::peers::{Peer, PeerStatus};
use crate::torrent::Torrent;
use crate::DownloadStatus;
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
pub struct BlockRequest {
    pub piece_i: u32,
    pub begin: u32,
    pub length: u32,
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
        &mut self,
        error_sender: Sender<DownloadStatus>,
        data_sender: Sender<DataPiece>,
    ) -> anyhow::Result<()> {
        if let PeerStatus::NotConnected | PeerStatus::WaitingForInterestedMsg = self.peer.status {
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
            log!(
                LogLevel::Fatal,
                "{} {} {}",
                self.torrent.info.length,
                self.torrent.info.piece_hashes.len(),
                self.torrent.info.piece_length
            );
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
            let req = BlockRequest {
                piece_i: self.task.piece_i as u32,
                begin: begin as u32,
                length: length as u32,
            };
            if let Err(e) = self.peer.request_block(&req).await {
                log!(LogLevel::Error, "Failed to download: {}", e);
                error_sender
                    .send(DownloadStatus::ChunksFail(self.task.clone()))
                    .await?;
            }
            begin += crate::CHUNK_SIZE;
        }
        log!(LogLevel::Debug, "Sended all requests");
        if let Err(e) = self
            .peer
            .receive_block(self.task.clone(), data_sender)
            .await
        {
            log!(LogLevel::Error, "Failed to download: {}", e);
            error_sender
                .send(DownloadStatus::ChunksFail(self.task.clone()))
                .await?;
        }
        Ok(())
    }

    // fn verify_hash(&mut self, piece_n: usize) -> bool {
    //     let hash = Torrent::bytes_hash(&self.buf);
    //     hash == self.torrent.info.piece_hashes[piece_n]
    // }
}
