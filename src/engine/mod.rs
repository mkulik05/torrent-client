use crate::UiMsg;

use std::collections::VecDeque;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use tokio::sync::broadcast::Sender;
use tokio::sync::mpsc;
use tokio::time::timeout;

use download::tasks::{ChunksTask, PieceTask, MAX_CHUNKS_TASKS};
use download::DownloadReq;
use peers::Peer;
use torrent::Torrent;
use tracker::TrackerReq;

use self::download::tasks::CHUNK_SIZE;
use self::download::DataPiece;
use crate::logger::{log, LogLevel};

mod bencode;
mod download;
pub mod logger;
mod peers;
mod saver;
pub mod torrent;
mod tracker;

enum DownloadEvents {
    ChunksFail(ChunksTask),
    InvalidHash(u64),
    Finished,
    PeerAdd(Peer),
}

pub fn parse_torrent(torrent_path: &str) -> anyhow::Result<Torrent> {
    Ok(Torrent::new(torrent_path)?)
}

pub async fn download_torrent(
    torrent_path: String,
    path: &str,
    ui_sender: Sender<UiMsg>,
) -> anyhow::Result<()> {
    let torrent = Torrent::new(torrent_path.as_str())?;
    let file_path = if Path::new(path).is_dir() {
        Path::new(path).join(&torrent.info.name)
    } else {
        Path::new(path).to_path_buf()
    };
    let file_path = file_path.to_str().unwrap();
    let tracker_req = TrackerReq::init(&torrent);
    let tracker_resp = Arc::new(tracker_req.send(&torrent).await?);
    let torrent = Arc::new(torrent);
    let (send_status, mut get_status) = mpsc::channel(270);
    let (send_data, get_data) = mpsc::channel::<DataPiece>(50);
    let mut peers = Vec::new();

    // tokio task spawns inside
    let peer_discovery_handle = tracker_resp
        .clone()
        .find_working_peers(send_data.clone(), send_status.clone());

    let pieces_done =
        saver::find_downloaded_pieces(torrent.clone(), file_path, ui_sender.clone()).await;

    let saver_task = saver::spawn_saver(
        file_path.to_string(),
        torrent.clone(),
        get_data,
        send_status.clone(),
        pieces_done.len(),
        ui_sender.clone(),
    );
    let mut pieces_tasks = download::tasks::get_piece_tasks(torrent.clone(), pieces_done);
    let mut chunks_tasks = VecDeque::new();

    download::tasks::add_chunks_tasks(&mut pieces_tasks, &mut chunks_tasks, MAX_CHUNKS_TASKS - 1);

    let mut no_free_peers = false;
    loop {
        // checking that saver task is alive
        if saver_task.is_finished() {
            if let Ok(res) = saver_task.await {
                res?;
            }
            break;
        }

        // if no free peers found, waiting for any message for some time,
        // if none appeared, searching for peers again
        let download_status = if no_free_peers {
            let res = timeout(Duration::from_secs(20), get_status.recv()).await;
            match res {
                Err(_) => {
                    tracker_resp
                        .clone()
                        .find_working_peers(send_data.clone(), send_status.clone());
                    continue;
                }
                Ok(value) => value,
            }
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
                    let total_chunks =
                        (torrent.info.piece_length as f64 / CHUNK_SIZE as f64).ceil() as u64;
                    pieces_tasks.push_front(PieceTask {
                        piece_i,
                        chunks_done: 0,
                        total_chunks: if piece_i as usize == (torrent.info.piece_hashes.len() - 1) {
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
                    chunks_tasks.push_front(chunk);
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

        download::tasks::add_chunks_tasks(&mut pieces_tasks, &mut chunks_tasks, 1);
        if !chunks_tasks.is_empty() {
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
            if !peer.have_piece(task.piece_i as usize) {
                peers[some_pos] = Some(peer);
                let peers_len = peers.len();
                peers.swap(some_pos, peers_len - 1);
                chunks_tasks.push_front(task);
                continue;
            }
            let downloader = DownloadReq::new(torrent.clone(), peer, task);
            tokio::spawn(async move {
                log!(
                    LogLevel::Debug,
                    "Peer {}, Curr task: {:?}",
                    downloader.peer.peer_addr,
                    downloader.task
                );
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
                };
            });
        } else {
            log!(LogLevel::Info, "Finished downloading");
            break;
        }
    }
    peer_discovery_handle.abort();
    Ok(())
}
