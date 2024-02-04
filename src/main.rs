use crate::download::DataPiece;
use std::collections::HashMap;
use std::env;
use std::fs::File;
use std::io::ErrorKind;
use std::io::Read;
use std::io::Seek;
use std::io::Write;
use std::time::Duration;
mod bencode;
mod download;
mod logger;
mod peers;
mod torrent;
mod tracker;
use crate::logger::{log, LogLevel};
use download::{ChunksTask, DownloadReq, PieceTask};
use peers::Peer;
use std::collections::VecDeque;
use std::sync::Arc;
use tokio::sync::mpsc;
use tokio::sync::Semaphore;
use torrent::Torrent;
use tracker::TrackerReq;

const MAX_CHUNKS_TASKS: usize = 100;
const CHUNKS_PER_TASK: u64 = 10;
const CHUNK_SIZE: u64 = 16384;
const SEMAPHORE_N: usize = 10;

enum DownloadEvents {
    ChunksFail(ChunksTask),
    InvalidHash(u64),
    Finished,
    PeerAdd(Peer),
}

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
        let mut last_chunk_mask = 0xff;
        let mut mask = 1;
        for _ in 0..(chunks_n % 8 - 1) {
            mask <<= 1;
            last_chunk_mask ^= mask;
        }
        PieceChunksBitmap {
            bitmap: vec![0; (chunks_n as f64 / 8.0).ceil() as usize],
            last_chunk_mask,
        }
    }
    fn add_chunk(&mut self, begin: usize) {
        let chunk_i = begin / crate::CHUNK_SIZE as usize;
        let bitmap_cell_i = chunk_i / 8;
        let mut mask = 128;
        mask >>= chunk_i % 8;
        self.bitmap[bitmap_cell_i] |= mask;
    }
    fn chunk_exist(&self, begin: usize) -> bool {
        let chunk_i = begin / crate::CHUNK_SIZE as usize;
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

fn handle_result<T>(res: anyhow::Result<T>) -> T {
    match res {
        Ok(v) => v,
        Err(err) => {
            log!(LogLevel::Fatal, "error {:?}", err);
            panic!("error {:?}", err);
        }
    }
}

fn add_chunks_tasks(
    pieces_tasks: &mut VecDeque<PieceTask>,
    chunks_tasks: &mut VecDeque<ChunksTask>,
    chunks_to_add: usize,
) {
    for _ in 0..chunks_to_add {
        if pieces_tasks.is_empty() {
            break;
        }
        let mut task = pieces_tasks.get_mut(0).expect("We checked it's not empty");
        if task.chunks_done >= task.total_chunks {
            let _ = pieces_tasks.pop_front();
            if pieces_tasks.is_empty() {
                break;
            }
            task = pieces_tasks.get_mut(0).expect("We checked it's not empty");
        }
        let chunks_up_border = if (task.chunks_done + CHUNKS_PER_TASK) > task.total_chunks {
            task.total_chunks
        } else {
            task.chunks_done + CHUNKS_PER_TASK
        };
        chunks_tasks.push_back(ChunksTask {
            piece_i: task.piece_i,
            chunks: task.chunks_done..chunks_up_border,
            includes_last_chunk: chunks_up_border == task.total_chunks,
        });
        task.chunks_done += CHUNKS_PER_TASK;
    }
}

#[tokio::main]
async fn main() {
    handle_result(logger::Logger::init(format!(
        "/tmp/log{}.txt",
        chrono::Local::now().format("%d-%m-%Y_%H-%M-%S")
    )));
    let args: Vec<String> = env::args().collect();
    let command = &args[1];

    match command.as_str() {
        "info" => {
            let torrent = Torrent::new(&args[2]).unwrap();
            let hashes: Vec<String> = torrent
                .info
                .piece_hashes
                .iter()
                .map(|bytes| hex::encode(bytes))
                .collect();
            println!(
                "Tracker URL: {}\nLength: {}\nInfo Hash: {}\nPiece Length: {}\nPiece Hashes:\n{}",
                torrent.tracker_url,
                torrent.info.length,
                hex::encode(torrent.info_hash),
                torrent.info.piece_length,
                hashes.join("\n")
            );
        }
        "download" => {
            let torrent = handle_result(Torrent::new(&args[4]));
            let tracker_req = TrackerReq::init(&torrent);
            let tracker_resp = handle_result(tracker_req.send(&torrent).await);
            let torrent = Arc::new(torrent);
            let (send_status, mut get_status) = mpsc::channel(50);
            let (send_data, mut get_data) = mpsc::channel::<DataPiece>(50);
            let mut peers = Vec::new();

            // Discover task - found working peers, sep task to immidiatly
            // start working with discovered peers
            {
                let send_status = send_status.clone();
                let send_data = send_data.clone();
                tokio::spawn(async move {
                    let mut n = 0;
                    for peer in tracker_resp.peers {
                        if let Ok(peer) = Peer::new(&peer, send_data.clone(), Duration::from_secs(2)).await {
                            n += 1;
                            send_status
                                .send(DownloadEvents::PeerAdd(peer))
                                .await
                                .unwrap();
                        }
                    }
                    log!(LogLevel::Info, "Connected to {} peer(s)", n);
                });
            }

            let mut pieces_done = Vec::new();
            if std::path::Path::new(&args[3]).exists() {
                let mut file = File::options().read(true).open(&args[3]).unwrap();
                let pieces_i = torrent.info.piece_hashes.len();
                let mut piece_buf = vec![0; torrent.info.piece_length as usize];
                for i in 0..pieces_i {
                    let read_res = file.read_exact(&mut piece_buf);
                    if let Err(e) = read_res {
                        if e.kind() == ErrorKind::UnexpectedEof {
                            break;
                        } else {
                            panic!("Unexpected error: {:?}", e);
                        }
                    }
                    let hash = Torrent::bytes_hash(&piece_buf);
                    if hash == torrent.info.piece_hashes[i] {
                        pieces_done.push(i);
                        log!(LogLevel::Info, "Piece {} is already downloaded", i);
                    }
                }
            }

            // Saver task - save downloaded chunks to disk, verify piece hash,
            // notify about finishing donwload
            {
                let torrent = torrent.clone();
                let send_status = send_status.clone();
                let pieces_done = pieces_done.len();
                tokio::spawn(async move {
                    let mut file = File::options()
                        .read(true)
                        .write(true)
                        .create(true)
                        .open(&args[3])
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
                                        torrent.info.length
                                            - data.piece_i as u64 * torrent.info.piece_length
                                    } else {
                                        torrent.info.piece_length
                                    };
                                    file.seek(std::io::SeekFrom::Start(addr)).unwrap();
                                    let mut piece_buf = vec![0; piece_length as usize];
                                    file.read_exact(&mut piece_buf).unwrap();
                                    let hash = Torrent::bytes_hash(&piece_buf);
                                    if hash != torrent.info.piece_hashes[data.piece_i as usize] {
                                        log!(
                                            LogLevel::Error,
                                            "Piece {} hash didn't match",
                                            data.piece_i
                                        );
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
                                            log!(
                                                LogLevel::Info,
                                                "Whole file downloaded and verified"
                                            );
                                            send_status
                                                .send(DownloadEvents::Finished)
                                                .await
                                                .unwrap();
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

            let mut pieces_tasks = VecDeque::new();
            let mut chunks_tasks = VecDeque::new();

            let total_chunks = (torrent.info.piece_length as f64 / CHUNK_SIZE as f64).ceil() as u64;
            for i in 0..torrent.info.piece_hashes.len() {
                if pieces_done.iter().position(|&x| x == i).is_some() {
                    continue;
                }
                pieces_tasks.push_back(PieceTask {
                    piece_i: i as u64,
                    total_chunks: if i == (torrent.info.piece_hashes.len() - 1) {
                        ((torrent.info.length
                            - (torrent.info.piece_hashes.len() - 1) as u64
                                * torrent.info.piece_length) as f64
                            / CHUNK_SIZE as f64)
                            .ceil() as u64
                    } else {
                        total_chunks
                    },
                    chunks_done: 0,
                })
            }
            add_chunks_tasks(&mut pieces_tasks, &mut chunks_tasks, MAX_CHUNKS_TASKS - 1);
            let semaphore = Arc::new(Semaphore::new(SEMAPHORE_N));
            let mut no_free_peers = false;
            loop {
                let download_status = if no_free_peers {
                    get_status.recv().await
                } else {
                    let download_status = get_status.try_recv();
                    if let Ok(val) = download_status {
                        Some(val)
                    } else {
                        None
                    }
                };
                if let Some(download_status) = download_status {
                    match download_status {
                        DownloadEvents::Finished => break,
                        DownloadEvents::InvalidHash(piece_i) => {
                            let total_chunks = (torrent.info.piece_length as f64
                                / CHUNK_SIZE as f64)
                                .ceil() as u64;
                            pieces_tasks.push_back(PieceTask {
                                piece_i,
                                chunks_done: 0,
                                total_chunks: if piece_i as usize
                                    == (torrent.info.piece_hashes.len() - 1)
                                {
                                    ((torrent.info.length
                                        - (torrent.info.piece_hashes.len() - 1) as u64
                                            * torrent.info.piece_length)
                                        as f64
                                        / CHUNK_SIZE as f64)
                                        .ceil() as u64
                                } else {
                                    total_chunks
                                },
                            })
                        }
                        DownloadEvents::ChunksFail(chunk) => {
                            log!(LogLevel::Debug, "chunk failed: {:?}", chunk);
                            log!(
                                LogLevel::Debug,
                                "[0] Chunks tasks: {:?}",
                                &Vec::from(chunks_tasks.clone())[0..10]
                            );
                            chunks_tasks.push_front(chunk);
                            log!(
                                LogLevel::Debug,
                                "[1] Chunks tasks: {:?}",
                                &Vec::from(chunks_tasks.clone())[0..10]
                            );
                        }
                        DownloadEvents::PeerAdd(peer) => {
                            no_free_peers = false;
                            let pos = peers.iter().position(|x: &Option<Peer>| x.is_none());
                            if let Some(i) = pos {
                                peers[i] = Some(peer);
                            } else {
                                peers.push(Some(peer))
                            }
                        }
                    }
                }

                add_chunks_tasks(&mut pieces_tasks, &mut chunks_tasks, 1);
                log!(
                    LogLevel::Debug,
                    "[2] Chunks tasks: {:?}",
                    &Vec::from(chunks_tasks.clone())[0..10]
                );
                if !chunks_tasks.is_empty() {
                    let permit = semaphore.clone().acquire_owned().await.unwrap();
                    let send_status = send_status.clone();
                    let send_data = send_data.clone();
                    let some_pos = peers.iter().position(|x| x.is_some());
                    let some_pos = if some_pos.is_none() {
                        log!(LogLevel::Debug, "No free peers, skipping iteration");
                        no_free_peers = true;
                        continue;
                    } else {
                        some_pos.unwrap()
                    };
                    let peer = peers[some_pos].take().unwrap();
                    let task = chunks_tasks.pop_front().unwrap();
                    // if !peer.have_piece(task.piece_i as usize) {
                    //     peers[some_pos] = Some(peer);
                    //     let peers_len = peers.len();
                    //     peers.swap(some_pos, peers_len - 1);
                    //     chunks_tasks.push_front(task);
                    //     continue;
                    // }
                    let downloader = DownloadReq::new(torrent.clone(), peer, task);
                    tokio::spawn(async move {
                        log!(LogLevel::Debug, "Peer {}, Curr task: {:?}", downloader.peer.peer_addr, downloader.task);
                        let task = downloader.task.clone();
                        let addr = downloader.peer.peer_addr.clone();
                        let send_status2 = send_status.clone();
                        if let Err(e) = downloader.request_data(send_status).await {
                            log!(
                                LogLevel::Error,
                                "Request data error: {}, peer addr {}",
                                e,
                                addr
                            );
                            send_status2
                                .send(DownloadEvents::ChunksFail(task))
                                .await
                                .unwrap();

                            //  TO FIX
                            let mut attempt_n = 0;
                            let mut delay = 1;
                            let mut peer = Peer::new(&addr, send_data.clone(), Duration::from_secs(delay)).await;
                            while attempt_n < 3 {
                                if let Err(e) = peer {
                                    if e.to_string() == "Connection timeout" {
                                        attempt_n += 1;
                                        delay += 1;
                                        peer = Peer::new(&addr, send_data.clone(), Duration::from_secs(delay)).await;
                                    } else {
                                        // failed to connect to peer, it's lost...
                                        log!(LogLevel::Fatal, "Can't connect to peer, it's lost... {}", addr);
                                        return;
                                    }
                                } else {
                                    break;
                                }
                            }
                            if peer.is_ok() {
                                send_status2
                                .send(DownloadEvents::PeerAdd(peer.unwrap()))
                                .await
                                .unwrap();
                            } else {
                                log!(LogLevel::Fatal, "Failed to connect to peer after several attemplts, it's lost... {}", addr);
                            }
                        };
                        drop(permit);
                    });
                }
            }
            // println!("Downloaded {} to {}.", &args[4], &args[3]);
        }
        _ => println!("unknown command: {}", args[1]),
    }
}
