use crate::logger::{log, LogLevel};
use std::collections::HashMap;
use std::fs::File;
use std::io::ErrorKind;
use std::io::Read;
use std::io::Seek;
use std::io::Write;
use std::sync::Arc;
use tokio::sync::mpsc;
use tokio::sync::mpsc::Receiver;
use tokio::sync::mpsc::Sender;
use tokio::time::timeout;

use crate::download::DataPiece;
use crate::{DownloadEvents, Torrent};

#[derive(Debug)]
struct PieceChunksBitmap {
    bitmap: Vec<u8>,
    last_chunk_mask: u8,
}

impl PieceChunksBitmap {
    fn new(torrent: &Torrent, piece_i: usize) -> Self {
        let piece_length = if piece_i == torrent.info.piece_hashes.len() - 1 {
            torrent.info.length - piece_i as u64 * torrent.info.piece_length
        } else {
            torrent.info.piece_length
        };
        let chunks_n = (piece_length as f64 / crate::CHUNK_SIZE as f64).ceil() as i32;
        let mut last_chunk_mask = 0;
        let mut mask = 128;
        if chunks_n % 8 == 0 {
            last_chunk_mask = 255;
        }
        for _ in 0..(chunks_n % 8) {
            last_chunk_mask |= mask;
            mask >>= 1;
        }
        PieceChunksBitmap {
            bitmap: vec![0; (chunks_n as f64 / 8.0).ceil() as usize],
            last_chunk_mask,
        }
    }
    fn add_chunk(&mut self, begin: usize) {
        let chunk_i = begin / crate::download_tasks::CHUNK_SIZE as usize;
        let bitmap_cell_i = chunk_i / 8;
        let mut mask = 128;
        mask >>= chunk_i % 8;
        self.bitmap[bitmap_cell_i] |= mask;
    }
    fn chunk_exist(&self, begin: usize) -> bool {
        let chunk_i = begin / crate::download_tasks::CHUNK_SIZE as usize;
        let bitmap_cell_i = chunk_i / 8;
        let mut mask = 128;
        mask >>= chunk_i % 8;
        self.bitmap[bitmap_cell_i] & mask == mask
    }
    fn is_piece_ready(&self) -> bool {
        for i in 0..self.bitmap.len() {
            if i == self.bitmap.len() - 1 {
                if self.last_chunk_mask != self.bitmap[i] {
                    return false;
                }
            } else {
                if self.bitmap[i] != 255 {
                    return false;
                }
            }
        }
        true
    }
}

pub fn spawn_saver(
    path: String,
    torrent: Arc<Torrent>,
    mut get_data: Receiver<DataPiece>,
    send_status: Sender<DownloadEvents>,
    pieces_done: usize,
) {
    // Saver task - save downloaded chunks to disk, verify piece hash,
    // notify about finishing donwload
    tokio::spawn(async move {
        let mut file = File::options()
            .read(true)
            .write(true)
            .create(true)
            .open(&path)
            .unwrap();
        let mut pieces_chunks: HashMap<u64, PieceChunksBitmap> = HashMap::new();
        let mut pieces_finished = pieces_done;
        loop {
            match get_data.recv().await {
                Some(data) => {
                    log!(
                        LogLevel::Info,
                        "Saver: piece_i: {} {}",
                        data.piece_i,
                        data.begin
                    );
                    if pieces_chunks.contains_key(&data.piece_i) {
                        if pieces_chunks
                            .get(&data.piece_i)
                            .unwrap()
                            .chunk_exist(data.begin as usize)
                        {
                            log!(
                                LogLevel::Error,
                                "Saver: Chunk {}.. of piece {} is already saved!!!",
                                data.begin,
                                data.piece_i
                            );
                            continue;
                        }
                    }
                    let addr = data.piece_i * torrent.info.piece_length + data.begin;
                    file.seek(std::io::SeekFrom::Start(addr)).unwrap();
                    file.write_all(&data.buf).unwrap();
                    if !pieces_chunks.contains_key(&data.piece_i) {
                        pieces_chunks.insert(
                            data.piece_i,
                            PieceChunksBitmap::new(&torrent, data.piece_i as usize),
                        );
                        log!(LogLevel::Debug, "Just added key");
                    }
                    log!(
                        LogLevel::Debug,
                        "{:?}",
                        pieces_chunks.get(&data.piece_i).unwrap()
                    );
                    let chunks_bitmap = pieces_chunks.get_mut(&data.piece_i).unwrap();
                    chunks_bitmap.add_chunk(data.begin as usize);
                    if chunks_bitmap.is_piece_ready() {
                        let addr = data.piece_i * torrent.info.piece_length;
                        let piece_length = if data.piece_i as usize
                            == torrent.info.piece_hashes.len() - 1
                        {
                            torrent.info.length - data.piece_i as u64 * torrent.info.piece_length
                        } else {
                            torrent.info.piece_length
                        };
                        file.seek(std::io::SeekFrom::Start(addr)).unwrap();
                        let mut piece_buf = vec![0; piece_length as usize];
                        file.read_exact(&mut piece_buf).unwrap();
                        let hash = Torrent::bytes_hash(&piece_buf);
                        if hash != torrent.info.piece_hashes[data.piece_i as usize] {
                            log!(LogLevel::Error, "Piece {} hash didn't match", data.piece_i);
                            send_status
                                .send(DownloadEvents::InvalidHash(data.piece_i))
                                .await
                                .unwrap();
                            *chunks_bitmap =
                                PieceChunksBitmap::new(&torrent, data.piece_i as usize);
                        } else {
                            log!(
                                LogLevel::Info,
                                "Piece {} hash matched, downloaded: {}",
                                data.piece_i,
                                pieces_finished + 1
                            );
                            pieces_finished += 1;
                            if pieces_finished == torrent.info.piece_hashes.len() {
                                log!(LogLevel::Info, "Whole file downloaded and verified");
                                send_status.send(DownloadEvents::Finished).await.unwrap();
                                break;
                            }
                        }
                    }
                }
                None => break,
            }
        }
    });
}

pub async fn find_downloaded_pieces(torrent: Arc<Torrent>, path: &str) -> Vec<usize> {
    let mut downloaded_pieces = Vec::new();
    let mut pieces_processed = 0;
    let mut pieces_done = 0; 
    if std::path::Path::new(path).exists() {
        let mut file = File::options().read(true).open(path).unwrap();
        let pieces_i = torrent.info.piece_hashes.len();
        let (sender, mut receiver) = mpsc::channel(200);
        for i in 0..pieces_i {
            let mut piece_buf = vec![0; torrent.info.piece_length as usize];
            let read_res = file.read_exact(&mut piece_buf);
            if let Err(e) = read_res {
                if e.kind() == ErrorKind::UnexpectedEof {
                    break;
                } else {
                    panic!("Unexpected error: {:?}", e);
                }
            }
            pieces_processed += 1;

            if let Ok((i, have)) = receiver.try_recv() {
                pieces_done += 1;
                if have {
                    downloaded_pieces.push(i);
                }
            }
            {
                let sender = sender.clone();
                let torrent = torrent.clone();
                tokio::task::spawn_blocking(move || {
                    let hash = Torrent::bytes_hash(&piece_buf);
                    if hash == torrent.info.piece_hashes[i] {
                        sender.try_send((i, true)).unwrap();
                        log!(LogLevel::Info, "Piece {} is already downloaded", i);
                    } else {
                        sender.try_send((i, false)).unwrap();
                    }
                });
            }
        }
        while pieces_done < pieces_processed {
            if let Ok(Some((i, have))) = timeout(std::time::Duration::from_secs(10), receiver.recv()).await {
                pieces_done += 1;
                if have {
                    downloaded_pieces.push(i);
                }
            } else {
                log!(LogLevel::Fatal, "Not all tasks joined");
                panic!("Not all tasks joined");
            }
        }
    }
    downloaded_pieces
}
