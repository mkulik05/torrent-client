use crate::peers::Peer;
use crate::torrent::Torrent;
use std::fs::File;
use std::io::Write;
use std::sync::Arc;
use tokio::time::{sleep, Duration};
use crate::logger::{log, LogLevel};

#[derive(Debug)]
pub struct BlockRequest {
    pub piece_i: u32,
    pub begin: u32,
    pub length: u32,
}

#[derive(Debug)]
pub struct Downloader {
    pub torrent: Arc<Torrent>,
    pub peer: Peer,
    pub piece_i: Option<u64>,
    pub file: File,
    buf: Vec<u8>,
    chunk_size: u64
}

impl Downloader {
    pub fn new(
        torrent: Arc<Torrent>,
        peer: Peer,
        piece_i: Option<u64>,
        path: &str,
    ) -> anyhow::Result<Self> {
        Ok(Downloader {
            torrent,
            peer,
            piece_i,
            file: std::fs::File::create(path)?,
            buf: Vec::new(),
            chunk_size: 16384 // on a greater value connection will be reseted by peer
        })
    }
    pub async fn download(&mut self) -> anyhow::Result<()> {
        if let Some(piece_i) = self.piece_i {
            log!(LogLevel::Debug, "Downloading 1 piece");
            self.download_piece().await?;
            assert!(self.verify_hash(piece_i as usize));
            self.file.write_all(&self.buf)?;
            self.buf.clear();
        } else {
            let pieces_n = self.torrent.info.piece_hashes.len();
            log!(LogLevel::Debug, "Downloading whole file ({pieces_n}) pieces");
            for i in 0..pieces_n {
                self.piece_i = Some(i as u64);
                self.download_piece().await?;
                assert!(self.verify_hash(i as usize));
                self.file.write_all(&self.buf)?;
                self.buf.clear();
            }
            self.piece_i = None;
        }
        Ok(())
    }

    // peer that already sent you unchoke msg
    async fn download_piece(&mut self) -> anyhow::Result<u64> {
        let piece_i = self.piece_i.expect("Called only when value is Some");
        let piece_length = if piece_i as usize == self.torrent.info.piece_hashes.len() - 1 {
            self.torrent.info.length - self.torrent.info.piece_length * piece_i
        } else {
            self.torrent.info.piece_length
        };
        let blocks_n = piece_length / self.chunk_size;
        log!(LogLevel::Debug, "Download piece: piece_i={} piece_length={}, blocks_n={}", piece_i, piece_length, blocks_n);
        let mut begin = 0;
        for _ in 0..blocks_n {
            let req = BlockRequest {
                piece_i: piece_i as u32,
                begin,
                length: self.chunk_size as u32,
            };
            let bytes = self.peer.fetch(&req).await?;
            self.buf.write_all(&bytes)?;
            begin += self.chunk_size as u32;
            sleep(Duration::from_millis(2)).await;
        }
        if piece_length - blocks_n * self.chunk_size > 0 {
            let req = BlockRequest {
                piece_i: piece_i as u32,
                begin,
                length: (piece_length - blocks_n * self.chunk_size) as u32,
            };
            let bytes = self.peer.fetch(&req).await?;
            self.buf.write_all(&bytes)?;
        }
        log!(LogLevel::Debug, "Download piece: finished");
        Ok(piece_length)
    }
    fn verify_hash(&mut self, piece_n: usize) -> bool {
        let hash = Torrent::bytes_hash(&self.buf);
        hash == self.torrent.info.piece_hashes[piece_n]
    }
}
