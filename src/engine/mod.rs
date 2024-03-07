use crate::backup;
use crate::{TorrentBackupInfo, UiHandle, UiMsg};
use std::collections::VecDeque;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
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
pub mod download;
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

struct DownloaderInfo {
    handle: Option<JoinHandle<()>>,
    task: ChunksTask,
}

// Represents peer lifecycle in peers array
enum DownloaderPeer {
    // Peer is ready for work, can spawn new task for it
    Free(Peer),

    // Peer info including tak and handle
    Busy(DownloaderInfo),

    // Downloading finished, peer can be replaced with another (sent through PeerAdd msg)
    Finished,
}

pub enum TorrentInfo {
    Torrent(Torrent),
    Backup(TorrentBackupInfo),
}

pub async fn download_torrent(
    torrent_info: TorrentInfo,
    path: &str,
    ui_handle: UiHandle,
) -> anyhow::Result<()> {
    let torrent = match torrent_info {
        TorrentInfo::Torrent(ref torrent) => torrent.clone(),
        TorrentInfo::Backup(ref backup) => backup.torrent.clone(),
    };

    if torrent.info.piece_hashes.len() > u16::MAX as usize {
        anyhow::bail!("Too many pieces");
    }

    if torrent.info.piece_length / CHUNK_SIZE > u16::MAX as u64 {
        anyhow::bail!("Too many chunks");
    }

    let save_path = if let TorrentInfo::Backup(ref backup) = torrent_info {
        Path::new(&backup.save_path).to_path_buf()
    } else {
        if Path::new(path).is_dir() {
            Path::new(path).join(&torrent.info.name)
        } else {
            Path::new(path).to_path_buf()
        }
    };

    let save_path = save_path.to_str().unwrap();
    let tracker_req = TrackerReq::init(&torrent);
    let tracker_resp = Arc::new(tracker_req.send(&torrent).await?);
    let torrent = Arc::new(torrent);
    let (send_status, mut get_status) = mpsc::channel(270);
    let (send_data, get_data) = mpsc::channel::<DataPiece>(50);
    let mut peers: Vec<DownloaderPeer> = Vec::new();

    // tokio task spawns inside
    let peer_discovery_handle = tracker_resp
        .clone()
        .find_working_peers(send_data.clone(), send_status.clone());

    let pieces_done = if let TorrentInfo::Torrent(_) = torrent_info {
        Some(saver::find_downloaded_pieces(torrent.clone(), save_path, ui_handle.clone()).await)
    } else {
        None
    };

    let pieces_done_n = if let TorrentInfo::Backup(ref backup) = torrent_info {
        backup.pieces_done
    } else {
        pieces_done.as_ref().unwrap().len()
    };

    let mut pieces_tasks;
    let mut chunks_tasks;

    match torrent_info {
        TorrentInfo::Torrent(_) => {
            pieces_tasks = download::tasks::get_piece_tasks(torrent.clone(), pieces_done.unwrap());
            chunks_tasks = VecDeque::new();
            download::tasks::add_chunks_tasks(
                &mut pieces_tasks,
                &mut chunks_tasks,
                MAX_CHUNKS_TASKS - 1,
            );
        }
        TorrentInfo::Backup(ref info) => {
            pieces_tasks = info.pieces_tasks.clone();
            chunks_tasks = info.chunks_tasks.clone();
            let len: usize = chunks_tasks.len();
            if MAX_CHUNKS_TASKS as i32 - chunks_tasks.len() as i32 > 0 {
                download::tasks::add_chunks_tasks(
                    &mut pieces_tasks,
                    &mut chunks_tasks,
                    MAX_CHUNKS_TASKS - len,
                );
            }
        }
    }

    let saver_task = saver::spawn_saver(
        save_path.to_string(),
        torrent.clone(),
        get_data,
        send_status.clone(),
        pieces_done_n,
        ui_handle.clone(),
        if let TorrentInfo::Backup(backup) = torrent_info {
            Some(backup)
        } else {
            None
        },
    );

    let mut no_free_peers = false;
    let mut ui_reader = ui_handle.ui_sender.subscribe();
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
            let output;
            tokio::select! {
                res = timeout(Duration::from_secs(20), get_status.recv()) => {
                    match res {
                        Err(_) => {
                            tracker_resp
                                .clone()
                                .find_working_peers(send_data.clone(), send_status.clone());
                            continue;
                        }
                        Ok(value) => {
                            output = value;
                        },
                    }
                }
                msg @ Ok(UiMsg::ForceOff | UiMsg::Pause(_)) = ui_reader.recv() => {
                    let msg = msg.unwrap();
                    for peer in peers {
                        if let DownloaderPeer::Busy(DownloaderInfo {
                            handle: Some(handle),
                            task
                        }) = peer
                        {
                            chunks_tasks.push_front(task);
                            handle.abort();
                        }
                    }
                    match msg {
                        UiMsg::ForceOff => {
                            log!(LogLevel::Debug, "Gor off msg, shutting down..");
                        },
                        UiMsg::Pause(done) => {
                            log!(LogLevel::Debug, "Gor pause msg, shutting down..");
                            backup::backup_torrent(
                                TorrentBackupInfo { 
                                    pieces_tasks, 
                                    chunks_tasks, 
                                    torrent: Arc::as_ref(&torrent).clone(), 
                                    save_path: save_path.to_string(), 
                                    pieces_done: done as usize, 
                                    status: crate::DownloadStatus::Paused 
                                },
                            )?
                        },
                        _ => {}
                    }
                    break;
                }
            }
            output
        } else {
            let ui_msg = ui_reader.try_recv();
            if let Ok(msg) = ui_msg {
                match msg {
                    UiMsg::ForceOff => {
                        log!(LogLevel::Debug, "Gor off msg, shutting down..");
                        for peer in peers {
                            if let DownloaderPeer::Busy(DownloaderInfo {
                                handle: Some(handle),
                                ..
                            }) = peer
                            {
                                handle.abort();
                            }
                        }
                        break;
                    }
                    _ => {}
                }
            };
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
                        piece_i: piece_i as u16,
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
                        } as u16,
                    })
                }
                DownloadEvents::ChunksFail(chunk) => {
                    log!(LogLevel::Debug, "chunk failed: {:?}", chunk);
                    chunks_tasks.push_front(chunk);
                }
                DownloadEvents::PeerAdd(peer) => {
                    no_free_peers = false;

                    // Checking did peer task finished or not
                    for el in &mut peers {
                        if let DownloaderPeer::Busy(DownloaderInfo {
                            handle: Some(handle),
                            ..
                        }) = el
                        {
                            if handle.is_finished() {
                                let _ = std::mem::replace(el, DownloaderPeer::Finished);
                            }
                        }
                    }

                    let pos = peers.iter().position(|x| {
                        if let DownloaderPeer::Finished = x {
                            true
                        } else {
                            false
                        }
                    });
                    if let Some(i) = pos {
                        peers[i] = DownloaderPeer::Free(peer);
                    } else {
                        peers.push(DownloaderPeer::Free(peer))
                    }
                }
            }
        }

        download::tasks::add_chunks_tasks(&mut pieces_tasks, &mut chunks_tasks, 1);
        if !chunks_tasks.is_empty() {
            let send_status = send_status.clone();
            let some_pos = peers.iter().position(|x| {
                if let DownloaderPeer::Free(_) = x {
                    true
                } else {
                    false
                }
            });
            let some_pos = if some_pos.is_none() {
                log!(LogLevel::Debug, "No free peers, skipping iteration");
                no_free_peers = true;
                continue;
            } else {
                some_pos.unwrap()
            };
            let DownloaderPeer::Free(ref mut peer) = peers[some_pos] else {
                panic!("not possible")
            };
            let task = chunks_tasks.pop_front().unwrap();
            if !peer.have_piece(task.piece_i as usize) {
                let peers_len = peers.len();
                peers.swap(some_pos, peers_len - 1);
                chunks_tasks.push_front(task);
                continue;
            }
            let DownloaderPeer::Free(peer) = std::mem::replace(
                &mut peers[some_pos],
                DownloaderPeer::Busy(DownloaderInfo {
                    handle: None,
                    task: task.clone(),
                }),
            ) else {
                panic!("not possible, we checked it")
            };
            let downloader = DownloadReq::new(torrent.clone(), peer, task);
            let handle = tokio::spawn(async move {
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
            if let DownloaderPeer::Busy(ref mut info) = peers[some_pos] {
                info.handle = Some(handle);
            }
        } else {
            log!(LogLevel::Info, "Finished downloading");
            break;
        }
    }
    peer_discovery_handle.abort();

    Ok(())
}
