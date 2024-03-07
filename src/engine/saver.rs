use std::collections::HashMap;
use std::fs::File;
use std::io::ErrorKind;
use std::io::Read;
use std::io::Seek;
use std::io::Write;
use std::path::Path;
use std::sync::Arc;

use tokio::sync::mpsc;
use tokio::sync::mpsc::Receiver;
use tokio::task::JoinHandle;
use tokio::time::timeout;

use crate::TorrentBackupInfo;
use crate::{UiHandle, UiMsg};

use super::download::tasks::CHUNK_SIZE;
use super::download::DataPiece;
use super::logger::{log, LogLevel};
use super::{DownloadEvents, Torrent};

#[derive(Debug)]
struct PieceChunksBitmap {
    bitmap: Vec<u8>,
    last_chunk_mask: u8,
}

impl PieceChunksBitmap {
    fn new(torrent: &Torrent, piece_i: usize) -> Self {
        let piece_length = torrent.get_piece_length(piece_i);
        let chunks_n = (piece_length as f64 / super::CHUNK_SIZE as f64).ceil() as i32;
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

    fn from_backup(torrent: &Torrent, piece_i: usize, done: usize) -> Self {
        let mut bitmap = PieceChunksBitmap::new(torrent, piece_i);
        for i in 0..done {
            bitmap.add_chunk(CHUNK_SIZE as usize * i);
        }
        bitmap
    }

    fn remove_chunk(&mut self, begin: usize) {
        let chunk_i = begin / super::CHUNK_SIZE as usize;
        let bitmap_cell_i = chunk_i / 8;
        let mut ones_mask = 0b1111_1111;
        let mut mask = 0b1000_0000;
        mask >>= chunk_i % 8;
        ones_mask ^= mask;
        self.bitmap[bitmap_cell_i] &= ones_mask;
    }

    fn add_chunk(&mut self, begin: usize) {
        let chunk_i = begin / super::CHUNK_SIZE as usize;
        let bitmap_cell_i = chunk_i / 8;
        let mut mask = 128;
        mask >>= chunk_i % 8;
        self.bitmap[bitmap_cell_i] |= mask;
    }

    fn chunk_exist(&self, begin: usize) -> bool {
        let chunk_i = begin / super::CHUNK_SIZE as usize;
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
            } else if self.bitmap[i] != 255 {
                return false;
            }
        }
        true
    }
}

fn bin_search(value: u64, arr: &[u64], l: usize, r: usize) -> usize {
    if value == 0 {
        return 1;
    }
    if l >= r {
        return l;
    }
    let m = l + (r - l) / 2;
    if arr[m] == value {
        return m;
    }
    if arr[m] < value {
        bin_search(value, arr, m + 1, r)
    } else {
        bin_search(value, arr, l, m)
    }
}

pub fn spawn_saver(
    src_path: String,
    torrent: Arc<Torrent>,
    mut get_data: Receiver<DataPiece>,
    send_status: mpsc::Sender<DownloadEvents>,
    pieces_done: usize,
    ui_handle: UiHandle,
    backup: Option<TorrentBackupInfo>,
) -> JoinHandle<anyhow::Result<()>> {
    // variable containing increasing array starting from 0;
    // each element arr[i] equals arr[i - 1] + file_sizes[i]
    // required to quickly find to which file write input data
    let files_lengthes = if let Some(ref files) = torrent.info.files {
        let mut arr = Vec::with_capacity(files.len());
        arr.push(0);
        for (i, file) in files.iter().enumerate() {
            arr.push(arr[i] + file.length);
        }
        Some(arr)
    } else {
        None
    };
    // Saver task - save downloaded chunks to disk, verify piece hash,
    // notify about finishing donwload
    tokio::spawn(async move {
        let mut pieces_chunks: HashMap<u64, PieceChunksBitmap> = HashMap::new();
        if let Some(backup) = backup {
            // For each piece marking done pieces
            for p_task in backup.pieces_tasks {
                pieces_chunks.insert(
                    p_task.piece_i as u64,
                    PieceChunksBitmap::from_backup(
                        &torrent,
                        p_task.piece_i as usize,
                        p_task.chunks_done as usize,
                    ),
                );
            }
            log!(LogLevel::Info, "fjdsodfjios\n\n\n");
            for c_task in backup.chunks_tasks {
                if let std::collections::hash_map::Entry::Vacant(e) =
                    pieces_chunks.entry(c_task.piece_i as u64)
                {
                    e.insert(PieceChunksBitmap::from_backup(
                        &torrent,
                        c_task.piece_i as usize,
                        0,
                    ));
                } else {
                    let bitmap = pieces_chunks.get_mut(&(c_task.piece_i as u64)).unwrap();
                    for c in c_task.chunks {
                        bitmap.remove_chunk(CHUNK_SIZE as usize * c as usize);
                    }
                }
            }
        }
        let mut pieces_finished = pieces_done;
        while let Some(data) = get_data.recv().await {
            log!(
                LogLevel::Info,
                "Saver: piece_i: {} {}",
                data.piece_i,
                data.begin
            );
            if pieces_chunks.contains_key(&data.piece_i)
                && pieces_chunks
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
            let addr = data.piece_i * torrent.info.piece_length + data.begin;
            if let Some(ref size_progression) = files_lengthes {
                let file_i_l = bin_search(addr, &size_progression, 0, size_progression.len());
                let r_addr = addr + data.buf.len() as u64;
                let file_i_r = bin_search(r_addr, &size_progression, 0, size_progression.len());
                if file_i_l == file_i_r {
                    // data if from one file
                    save_piece_to_file(
                        &torrent,
                        &src_path,
                        file_i_l - 1,
                        &data.buf,
                        addr - size_progression[file_i_l - 1],
                    )?;
                } else {
                    // data is from different files
                    save_data_to_files(
                        &src_path,
                        torrent.clone(),
                        &data,
                        &size_progression,
                        file_i_l,
                        file_i_r,
                    )?;
                }
            } else {
                // one file case
                let mut file = File::options()
                    .read(true)
                    .write(true)
                    .create(true)
                    .open(&src_path)
                    .unwrap();
                file.seek(std::io::SeekFrom::Start(addr)).unwrap();
                file.write_all(&data.buf).unwrap();
            };

            // adding chunk to bitmap
            if let std::collections::hash_map::Entry::Vacant(e) = pieces_chunks.entry(data.piece_i)
            {
                e.insert(PieceChunksBitmap::new(&torrent, data.piece_i as usize));
                log!(LogLevel::Debug, "Just added key");
            }
            let chunks_bitmap = pieces_chunks.get_mut(&data.piece_i).unwrap();
            chunks_bitmap.add_chunk(data.begin as usize);
            if chunks_bitmap.is_piece_ready() {
                let addr = data.piece_i * torrent.info.piece_length;
                let piece_length = torrent.get_piece_length(data.piece_i as usize);
                let mut piece_buf = Vec::new();

                // reading piece from file(s)
                if let Some(ref size_progression) = files_lengthes {
                    read_piece_from_files(
                        &src_path,
                        &torrent,
                        data.piece_i,
                        addr,
                        piece_length,
                        &mut piece_buf,
                        &size_progression,
                    )?;
                } else {
                    let mut file = File::options()
                        .read(true)
                        .write(true)
                        .create(true)
                        .open(&src_path)
                        .unwrap();
                    piece_buf = vec![0u8; piece_length as usize];
                    file.seek(std::io::SeekFrom::Start(addr)).unwrap();
                    file.read_exact(&mut piece_buf).unwrap();
                }
                let hash = Torrent::bytes_hash(&piece_buf);
                if hash != torrent.info.piece_hashes[data.piece_i as usize] {
                    log!(LogLevel::Error, "Piece {} hash didn't match", data.piece_i);
                    send_status
                        .send(DownloadEvents::InvalidHash(data.piece_i))
                        .await
                        .unwrap();
                    *chunks_bitmap = PieceChunksBitmap::new(&torrent, data.piece_i as usize);
                } else {
                    if let Err(e) = ui_handle.send_with_update(UiMsg::PieceDone) {
                        log!(LogLevel::Error, "Failed to PieceDone msg");
                    }
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
        Ok(())
    })
}

fn save_data_to_files(
    src_path: &String,
    torrent: Arc<Torrent>,
    data: &DataPiece,
    size_progression: &Vec<u64>,
    file_i_l: usize,
    file_i_r: usize,
) -> anyhow::Result<()> {
    let mut addr = data.piece_i * torrent.info.piece_length + data.begin;
    let mut bytes_saved = 0;
    save_piece_to_file(
        &torrent,
        &src_path,
        file_i_l - 1,
        &data.buf[bytes_saved..(bytes_saved + {
            let mut border_r = torrent.info.files.as_ref().unwrap()[file_i_l - 1].length as usize;
            border_r = border_r - (addr - size_progression[file_i_l - 1]) as usize;
            if (bytes_saved + border_r) > data.buf.len() {
                data.buf.len() - bytes_saved
            } else {
                border_r
            }
        })],
        addr - size_progression[file_i_l - 1],
    )?;
    // println!("{} {}", torrent.info.files.as_ref().unwrap()[file_i].length as usize, size_progression[file_i] as usize, size_progression[file_i] as usize);
    bytes_saved += torrent.info.files.as_ref().unwrap()[file_i_l - 1].length as usize
        - (addr as usize - size_progression[file_i_l - 1] as usize);
    addr = 0;
    for file_i in file_i_l..=(file_i_r - 2) {
        save_piece_to_file(
            &torrent,
            &src_path,
            file_i,
            &data.buf[bytes_saved..(bytes_saved + {
                let border_r = torrent.info.files.as_ref().unwrap()[file_i].length as usize;
                if (bytes_saved + border_r) > data.buf.len() {
                    data.buf.len() - bytes_saved
                } else {
                    border_r
                }
            })],
            addr,
        )?;
        // println!("{} {}", torrent.info.files.as_ref().unwrap()[file_i].length as usize, size_progression[file_i] as usize, size_progression[file_i] as usize);
        bytes_saved += torrent.info.files.as_ref().unwrap()[file_i].length as usize;
        //addr = 0;
    }
    save_piece_to_file(
        &torrent,
        &src_path,
        file_i_r - 1,
        &data.buf[bytes_saved..(bytes_saved + {
            let border_r = torrent.info.files.as_ref().unwrap()[file_i_r - 1].length as usize;
            if (bytes_saved + border_r) > data.buf.len() {
                data.buf.len() - bytes_saved
            } else {
                border_r
            }
        })],
        0,
    )?;
    Ok(())
}

fn read_piece_from_files(
    src_path: &String,
    torrent: &Arc<Torrent>,
    piece_i: u64,
    addr: u64,
    piece_length: u64,
    mut piece_buf: &mut Vec<u8>,
    size_progression: &Vec<u64>,
) -> anyhow::Result<()> {
    let file_i_l = bin_search(addr, &size_progression, 0, size_progression.len());
    let file_i_r = bin_search(
        addr + torrent.get_piece_length(piece_i as usize),
        &size_progression,
        0,
        size_progression.len(),
    );
    if file_i_l == file_i_r {
        let mut path = Path::new(&src_path);
        let new_path = path.join(&torrent.info.files.as_ref().unwrap()[file_i_l - 1].path);
        path = &new_path;
        let mut file = File::options()
            .read(true)
            .write(true)
            .create(true)
            .open(&path)?;
        file.seek(std::io::SeekFrom::Start(
            addr - size_progression[file_i_l - 1],
        ))?;
        *piece_buf = vec![0u8; piece_length as usize];
        file.read_exact(&mut piece_buf)?;
    } else {
        *piece_buf = Vec::new();
        let mut readed_bytes = 0;
        let mut path = Path::new(&src_path);
        let new_path = path.join(&torrent.info.files.as_ref().unwrap()[file_i_l - 1].path);
        path = &new_path;
        let mut file = File::options().read(true).write(true).open(path)?;
        file.seek(std::io::SeekFrom::Start(
            addr - size_progression[file_i_l - 1],
        ))?;
        let bytes_n = file.read_to_end(&mut piece_buf)?;
        readed_bytes += bytes_n;
        for file_i in file_i_l..=(file_i_r - 2) {
            let mut path = Path::new(&src_path);
            let new_path = path.join(&torrent.info.files.as_ref().unwrap()[file_i].path);
            path = &new_path;
            let mut file = File::options().read(true).write(true).open(&path)?;
            let mut buf = Vec::new();
            let bytes_n = file.read_to_end(&mut buf)?;
            readed_bytes += bytes_n;
            piece_buf.extend_from_slice(&buf[..bytes_n]);
        }
        let mut path = Path::new(&src_path);
        let new_path = path.join(&torrent.info.files.as_ref().unwrap()[file_i_r - 1].path);
        path = &new_path;
        let mut file = File::options().read(true).write(true).open(path)?;
        let piece_length = torrent.get_piece_length(piece_i as usize);
        let mut buf = vec![0u8; piece_length as usize - readed_bytes];
        if !buf.is_empty() {
            file.read_exact(&mut buf)?;
            piece_buf.extend_from_slice(&buf);
        }
    }
    Ok(())
}

pub async fn find_downloaded_pieces(
    torrent: Arc<Torrent>,
    src_path: &str,
    ui_handle: UiHandle,
) -> Vec<usize> {
    let mut downloaded_pieces = Vec::new();
    let mut pieces_processed = 0;
    let mut pieces_done = 0;

    if Path::new(src_path).exists() {
        let pieces_i = torrent.info.piece_hashes.len();
        let (sender, mut receiver) = mpsc::channel(200);
        if let Some(ref files) = torrent.info.files {
            let size_progression = {
                let mut arr = Vec::with_capacity(files.len());
                arr.push(0);
                for (i, file) in files.iter().enumerate() {
                    arr.push(arr[i] + file.length);
                }
                arr
            };
            for i in 0..pieces_i {
                let piece_length = torrent.get_piece_length(i);
                let mut piece_buf = Vec::new();
                let addr = i as u64 * torrent.info.piece_length;
                if let Err(e) = read_piece_from_files(
                    &src_path.to_string(),
                    &torrent,
                    i as u64,
                    addr,
                    piece_length,
                    &mut piece_buf,
                    &size_progression,
                ) {
                    log!(
                        LogLevel::Info,
                        "{:?} {} {} {}",
                        e,
                        i,
                        piece_length,
                        addr > torrent.info.length
                    );
                } else {
                    let sender = sender.clone();
                    let ui_handle = ui_handle.clone();
                    let torrent = torrent.clone();
                    tokio::task::spawn_blocking(move || {
                        let hash = Torrent::bytes_hash(&piece_buf);
                        if hash == torrent.info.piece_hashes[i] {
                            sender.try_send((i, true)).unwrap();
                            if let Err(e) = ui_handle.send_with_update(UiMsg::PieceDone) {
                                log!(LogLevel::Error, "Failed to PieceDone msg");
                            }
                            log!(LogLevel::Info, "Piece {} is already downloaded", i);
                        } else {
                            sender.try_send((i, false)).unwrap();
                        }
                    });
                    pieces_processed += 1;
                }

                if let Ok((i, have)) = receiver.try_recv() {
                    pieces_done += 1;
                    if have {
                        downloaded_pieces.push(i);
                    }
                }
            }
        } else {
            let mut file = File::options().read(true).open(src_path).unwrap();

            for i in 0..pieces_i {
                let piece_length = torrent.get_piece_length(i);
                let mut piece_buf = vec![0; piece_length as usize];
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
                    let ui_handle = ui_handle.clone();
                    let torrent = torrent.clone();
                    tokio::task::spawn_blocking(move || {
                        let hash = Torrent::bytes_hash(&piece_buf);
                        if hash == torrent.info.piece_hashes[i] {
                            sender.try_send((i, true)).unwrap();
                            if let Err(e) = ui_handle.send_with_update(UiMsg::PieceDone) {
                                log!(LogLevel::Error, "Failed to PieceDone msg");
                            }
                            log!(LogLevel::Info, "Piece {} is already downloaded", i);
                        } else {
                            sender.try_send((i, false)).unwrap();
                        }
                    });
                }
            }
        }

        while pieces_done < pieces_processed {
            if let Ok(Some((i, have))) =
                timeout(std::time::Duration::from_secs(10), receiver.recv()).await
            {
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

fn save_piece_to_file(
    torrent: &Arc<Torrent>,
    path: &String,
    file_i: usize,
    buf: &[u8],
    addr: u64,
) -> anyhow::Result<()> {
    let mut path = Path::new(path);
    let new_path = path.join(&torrent.info.files.as_ref().unwrap()[file_i].path);
    path = &new_path;
    if let Some(root) = path.parent() {
        if !root.exists() {
            std::fs::create_dir_all(root)?;
        }
    }
    let mut file = File::options()
        .read(true)
        .write(true)
        .create(true)
        .open(path)
        .unwrap();
    file.seek(std::io::SeekFrom::Start(addr))?;
    file.write_all(buf)?;
    file.set_len(torrent.info.files.as_ref().unwrap()[file_i].length)?;
    Ok(())
}
