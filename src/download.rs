use crate::bencode::BencodeValue;
use crate::peers::{BlockRequest, Peer};
use crate::torrent::Torrent;
use std::io::{Write, Read, Seek, SeekFrom};
use std::sync::Arc;
use tokio::time::{sleep, Duration};

pub struct Downloader<W: Write + Read + Seek> {
    pub torrent: Arc<Torrent>,
    pub peer: Peer,
    pub piece_i: Option<u32>,
    pub buf: W,
}

impl<W: Write + Read + Seek> Downloader<W> {
    pub async fn download(&mut self) {
        if let Some(piece_i) = self.piece_i {
            let bytes_n = self.download_piece().await;
            assert!(self.verify_hash(0, bytes_n, piece_i as usize))
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
    fn verify_hash(&mut self, start_byte: u64, piece_length: u32, piece_n: usize) -> bool {
        self.buf.seek(SeekFrom::Start(start_byte)).unwrap();
        let mut buf = vec![0u8; piece_length as usize];
        self.buf.read_exact(&mut buf[..]).unwrap();
        let hash = Torrent::get_hash_bytes(&BencodeValue::Bytes(buf));
        hash == self.torrent.info.piece_hashes[piece_n]
    }
}
