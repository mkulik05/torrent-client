use crate::peers::{BlockRequest, Peer};
use crate::torrent::Torrent;
use std::io::Write;
use std::fs::File;
use std::sync::Arc;
use tokio::time::{sleep, Duration};

pub struct Downloader {
    pub torrent: Arc<Torrent>,
    pub peer: Peer,
    pub piece_i: Option<u32>,
    pub file: File,
    buf: Vec<u8>
}

impl Downloader {
    pub fn new(torrent: Arc<Torrent>, peer: Peer, piece_i: Option<u32>, path: &str) -> Self {
        Downloader {
            torrent,
            peer,
            piece_i,
            file: std::fs::File::create(path).unwrap(),
            buf: Vec::new()
        }
    }
    pub async fn download(&mut self) {
        if let Some(piece_i) = self.piece_i {
            self.download_piece().await;
            assert!(self.verify_hash(piece_i as usize));
            self.file.write_all(&self.buf).unwrap();
        } else {

        }
    }

    // peer that already sent you unchoke msg
    async fn download_piece(&mut self) -> u32 {
        let piece_i = self.piece_i.unwrap();
        let piece_length = if piece_i as usize == self.torrent.info.piece_hashes.len() - 1 {
            self.torrent.info.length - self.torrent.info.piece_length * piece_i as i64
        } else {
            self.torrent.info.piece_length
        };
        let blocks_n = piece_length / 16384; // 16kiB block
        let mut begin = 0;
        for _ in 0..blocks_n {
            let req = BlockRequest {
                piece_i,
                begin,
                length: 16384,
            };
            let bytes = self.peer.fetch(&req).await;
            self.buf.write_all(&bytes).unwrap();
            begin += 16384;
            sleep(Duration::from_millis(10)).await;
        }
        if piece_length - blocks_n * 16384 > 0 {
            let req = BlockRequest {
                piece_i,
                begin,
                length: (piece_length - blocks_n * 16384) as u32,
            };
            let bytes = self.peer.fetch(&req).await;
            self.buf.write_all(&bytes).unwrap();
        }
        piece_length as u32
    }
    fn verify_hash(&mut self, piece_n: usize) -> bool {
        let hash = Torrent::bytes_hash(&self.buf);
        hash == self.torrent.info.piece_hashes[piece_n]
    }
}
