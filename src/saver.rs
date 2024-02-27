use crate::logger::{log, LogLevel};
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
    send_status: Sender<DownloadEvents>,
    pieces_done: usize,
) {
    let file_inc_length = if let Some(ref files) = torrent.info.files {
        log!(LogLevel::Info, "{:?}", files);
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
            let mut addr = data.piece_i * torrent.info.piece_length + data.begin;
            if let Some(ref size_progression) = file_inc_length {
                let file_i_l = bin_search(addr, &size_progression, 0, size_progression.len());
                let r_addr = addr + data.buf.len() as u64;
                let file_i_r = bin_search(r_addr, &size_progression, 0, size_progression.len());
                println!("fdsfds {:?} {}", r_addr, addr);
                println!("{:?} {}", data.piece_i, data.begin);
                if file_i_l == file_i_r {
                    save_piece_to_file(
                        &torrent,
                        &src_path,
                        file_i_l - 1,
                        &data.buf,
                        addr - size_progression[file_i_l - 1],
                    );
                } else {
                    log!(LogLevel::Info, "files_i: {file_i_l} {file_i_r}");
                    log!(
                        LogLevel::Info,
                        "progression: {:?} {}",
                        size_progression,
                        size_progression.len()
                    );
                    let mut bytes_saved = 0;
                    save_piece_to_file(
                        &torrent,
                        &src_path,
                        file_i_l - 1,
                        &data.buf[bytes_saved..(bytes_saved + {
                            let mut border_r =
                                torrent.info.files.as_ref().unwrap()[file_i_l - 1].length as usize;
                            border_r = border_r - (addr - size_progression[file_i_l - 1]) as usize;
                            if (bytes_saved + border_r) > data.buf.len() {
                                println!("{} {}", data.buf.len(), bytes_saved);
                                data.buf.len() - bytes_saved
                            } else {
                                border_r
                            }
                        })],
                        addr - size_progression[file_i_l - 1],
                    );
                    // println!("{} {}", torrent.info.files.as_ref().unwrap()[file_i].length as usize, size_progression[file_i] as usize, size_progression[file_i] as usize);
                    bytes_saved += torrent.info.files.as_ref().unwrap()[file_i_l - 1].length
                        as usize
                        - (addr as usize - size_progression[file_i_l - 1] as usize);
                    addr = 0;
                    for file_i in file_i_l..=(file_i_r - 2) {
                        log!(
                            LogLevel::Info,
                            "info: {file_i} {bytes_saved} {} addr: {}, file_i_l - 1={}",
                            torrent.info.files.as_ref().unwrap()[file_i].length,
                            0,
                            file_i_l - 1
                        );
                        save_piece_to_file(
                            &torrent,
                            &src_path,
                            file_i,
                            &data.buf[bytes_saved..(bytes_saved + {
                                let border_r =
                                    torrent.info.files.as_ref().unwrap()[file_i].length as usize;
                                if (bytes_saved + border_r) > data.buf.len() {
                                    println!("{} {}", data.buf.len(), bytes_saved);
                                    data.buf.len() - bytes_saved
                                } else {
                                    border_r
                                }
                            })],
                            addr,
                        );
                        // println!("{} {}", torrent.info.files.as_ref().unwrap()[file_i].length as usize, size_progression[file_i] as usize, size_progression[file_i] as usize);
                        bytes_saved += torrent.info.files.as_ref().unwrap()[file_i].length as usize;
                        //addr = 0;
                    }
                    save_piece_to_file(
                        &torrent,
                        &src_path,
                        file_i_r - 1,
                        &data.buf[bytes_saved..(bytes_saved + {
                            let border_r =
                                torrent.info.files.as_ref().unwrap()[file_i_r - 1].length as usize;
                            if (bytes_saved + border_r) > data.buf.len() {
                                println!("{} {}", data.buf.len(), bytes_saved);
                                data.buf.len() - bytes_saved
                            } else {
                                border_r
                            }
                        })],
                        0,
                    );
                }
            } else {
                let mut file = File::options()
                    .read(true)
                    .write(true)
                    .create(true)
                    .open(&src_path)
                    .unwrap();
                file.seek(std::io::SeekFrom::Start(addr)).unwrap();
                file.write_all(&data.buf).unwrap();
            };
            if let std::collections::hash_map::Entry::Vacant(e) = pieces_chunks.entry(data.piece_i)
            {
                e.insert(PieceChunksBitmap::new(&torrent, data.piece_i as usize));
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
                //panic!("");
                let addr = data.piece_i * torrent.info.piece_length;
                let piece_length = if data.piece_i as usize == torrent.info.piece_hashes.len() - 1 {
                    torrent.info.length - data.piece_i * torrent.info.piece_length
                } else {
                    torrent.info.piece_length
                };
                let mut piece_buf;
                if let Some(ref size_progression) = file_inc_length {
                    log!(
                        LogLevel::Info,
                        "INFO: {} {}\nNEXT INFO {:?}",
                        addr,
                        addr + if data.piece_i as usize == (torrent.info.piece_hashes.len() - 1) {
                            torrent.info.length - torrent.info.piece_length * data.piece_i
                        } else {
                            torrent.info.piece_length
                        },
                        size_progression
                    );
                    let file_i_l = bin_search(addr, &size_progression, 0, size_progression.len());
                    let file_i_r = bin_search(
                        addr + if data.piece_i as usize == (torrent.info.piece_hashes.len() - 1) {
                            torrent.info.length - torrent.info.piece_length * data.piece_i
                        } else {
                            torrent.info.piece_length
                        },
                        &size_progression,
                        0,
                        size_progression.len(),
                    );
                    if file_i_l == file_i_r {
                        let mut path = Path::new(&src_path);
                        let new_path =
                            path.join(&torrent.info.files.as_ref().unwrap()[file_i_l - 1].path);
                        path = &new_path;
                        let mut file = File::options()
                            .read(true)
                            .write(true)
                            .create(true)
                            .open(&path)
                            .unwrap();
                        log!(
                            LogLevel::Info,
                            "piece num: {}\n {:?}",
                            data.piece_i,
                            chunks_bitmap
                        );
                        log!(
                            LogLevel::Info,
                            "{} {} {} {} {}",
                            addr,
                            file_i_l - 1,
                            torrent.info.files.as_ref().unwrap()[file_i_l - 1].length,
                            torrent.info.files.as_ref().unwrap()[file_i_l - 1].path,
                            data.piece_i
                        );
                        let m = file.metadata().unwrap();
                        file.seek(std::io::SeekFrom::Start(
                            addr - size_progression[file_i_l - 1],
                        ))
                        .unwrap();
                        piece_buf = vec![0; piece_length as usize];
                        log!(
                            LogLevel::Info,
                            "{} {} {} {} {:?}",
                            addr,
                            size_progression[file_i_l - 1],
                            addr - size_progression[file_i_l - 1],
                            piece_buf.len(),
                            m.len()
                        );
                        file.read_exact(&mut piece_buf).unwrap();
                        println!(
                            "WOOWOWO0: {} {:?}",
                            piece_buf.len(),
                            piece_buf
                                .iter()
                                .filter(|&&x| x != 0)
                                .collect::<Vec<&u8>>()
                                .len()
                        );
                        if piece_buf.len()
                            != piece_buf
                                .iter()
                                .filter(|&&x| x != 0)
                                .collect::<Vec<&u8>>()
                                .len()
                        {
                            println!("WOOWOWO n= {}", path.to_str().unwrap());
                        }
                    } else {
                        piece_buf = Vec::new();
                        let mut readed_bytes = 0;
                        let mut path = Path::new(&src_path);
                        let new_path =
                            path.join(&torrent.info.files.as_ref().unwrap()[file_i_l - 1].path);
                        path = &new_path;
                        let mut file = File::options().read(true).write(true).open(path).unwrap();
                        file.seek(std::io::SeekFrom::Start(
                            addr - size_progression[file_i_l - 1],
                        ))
                        .unwrap();
                        let bytes_n = file.read_to_end(&mut piece_buf).unwrap();
                        println!(
                            "WOOWOWO1: {} {:?}",
                            bytes_n,
                            piece_buf
                                .iter()
                                .filter(|&&x| x != 0)
                                .collect::<Vec<&u8>>()
                                .len()
                        );
                        if bytes_n
                            != piece_buf
                                .iter()
                                .filter(|&&x| x != 0)
                                .collect::<Vec<&u8>>()
                                .len()
                        {
                            println!("WOOWOWO {:?} n= {}", &piece_buf, path.to_str().unwrap());
                        }
                        readed_bytes += bytes_n;
                        println!("{}", piece_buf.len());
                        for file_i in file_i_l..=(file_i_r - 2) {
                            let mut path = Path::new(&src_path);
                            let new_path =
                                path.join(&torrent.info.files.as_ref().unwrap()[file_i].path);
                            path = &new_path;
                            let mut file =
                                File::options().read(true).write(true).open(&path).unwrap();
                            let mut buf = Vec::new();
                            let bytes_n = file.read_to_end(&mut buf).unwrap();
                            println!(
                                "WOOWOWO2: {} {:?}",
                                bytes_n,
                                buf.iter().filter(|&&x| x != 0).collect::<Vec<&u8>>().len()
                            );
                            if bytes_n
                                != buf.iter().filter(|&&x| x != 0).collect::<Vec<&u8>>().len()
                            {
                                println!("WOOWOWO n= {}", path.to_str().unwrap());
                            }
                            log!(
                                LogLevel::Info,
                                "bla bla {} {} {}",
                                file_i,
                                readed_bytes,
                                bytes_n,
                            );
                            readed_bytes += bytes_n;
                            piece_buf.extend_from_slice(&buf[..bytes_n]);
                        }
                        let mut path = Path::new(&src_path);
                        let new_path =
                            path.join(&torrent.info.files.as_ref().unwrap()[file_i_r - 1].path);
                        path = &new_path;
                        let mut file = File::options().read(true).write(true).open(path).unwrap();
                        let piece_size =
                            if data.piece_i as usize == (torrent.info.piece_hashes.len() - 1) {
                                torrent.info.length - torrent.info.piece_length * data.piece_i
                            } else {
                                torrent.info.piece_length
                            };
                        log!(
                            LogLevel::Info,
                            "bla bla {} {} {}",
                            piece_size,
                            readed_bytes,
                            piece_buf.len()
                        );
                        log!(
                            LogLevel::Info,
                            "{file_i_l} {file_i_r} {addr} {:?} \n {:?}",
                            &size_progression[(file_i_l - 1)..(file_i_r + 1)],
                            size_progression
                        );
                        //panic!("");
                        let mut buf = vec![0u8; piece_size as usize - readed_bytes];
                        if !buf.is_empty() {
                            file.read_exact(&mut buf).unwrap();
                            println!(
                                "WOOWOWO22: {} {:?}",
                                buf.len(),
                                buf.iter().filter(|&&x| x != 0).collect::<Vec<&u8>>().len()
                            );
                            if buf.len()
                                != buf.iter().filter(|&&x| x != 0).collect::<Vec<&u8>>().len()
                            {
                                println!("WOOWOWO n= {}", path.to_str().unwrap());
                            }
                            piece_buf.extend_from_slice(&buf);
                        }
                    }
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
                if data.piece_i == 0 {
                    // log!(LogLevel::Info, "{:?}", piece_buf);
                    // log!(LogLevel::Info, "{:?}", torrent.info.files.as_ref().unwrap());
                    // let res = piece_buf.iter().filter(|&&x| x != 0).collect::<Vec<&u8>>();
                    // println!("{}", res.len());
                    //panic!("");
                    let mut file = File::create("/home/mkul1k/Videos/code.bin").unwrap();
                    file.write_all(&piece_buf).unwrap();
                    file.flush().unwrap();
                    // panic!("");
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
    });
}

// pub async fn find_downloaded_pieces(torrent: Arc<Torrent>, path: &str) -> Vec<usize> {
//     let mut downloaded_pieces = Vec::new();
//     let mut pieces_processed = 0;
//     let mut pieces_done = 0;
//     if std::path::Path::new(path).exists() {
//         let mut file = File::options().read(true).open(path).unwrap();
//         let pieces_i = torrent.info.piece_hashes.len();
//         let (sender, mut receiver) = mpsc::channel(200);
//         for i in 0..pieces_i {
//             let mut piece_buf = vec![0; torrent.info.piece_length as usize];
//             let read_res = file.read_exact(&mut piece_buf);
//             if let Err(e) = read_res {
//                 if e.kind() == ErrorKind::UnexpectedEof {
//                     break;
//                 } else {
//                     panic!("Unexpected error: {:?}", e);
//                 }
//             }
//             pieces_processed += 1;

//             if let Ok((i, have)) = receiver.try_recv() {
//                 pieces_done += 1;
//                 if have {
//                     downloaded_pieces.push(i);
//                 }
//             }
//             {
//                 let sender = sender.clone();
//                 let torrent = torrent.clone();
//                 tokio::task::spawn_blocking(move || {
//                     let hash = Torrent::bytes_hash(&piece_buf);
//                     if hash == torrent.info.piece_hashes[i] {
//                         sender.try_send((i, true)).unwrap();
//                         log!(LogLevel::Info, "Piece {} is already downloaded", i);
//                     } else {
//                         sender.try_send((i, false)).unwrap();
//                     }
//                 });
//             }
//         }
//         while pieces_done < pieces_processed {
//             if let Ok(Some((i, have))) =
//                 timeout(std::time::Duration::from_secs(10), receiver.recv()).await
//             {
//                 pieces_done += 1;
//                 if have {
//                     downloaded_pieces.push(i);
//                 }
//             } else {
//                 log!(LogLevel::Fatal, "Not all tasks joined");
//                 panic!("Not all tasks joined");
//             }
//         }
//     }
//     downloaded_pieces
// }

fn save_piece_to_file(torrent: &Arc<Torrent>, path: &String, file_i: usize, buf: &[u8], addr: u64) {
    let mut path = Path::new(path);
    let new_path = path.join(&torrent.info.files.as_ref().unwrap()[file_i].path);
    path = &new_path;
    if let Some(root) = path.parent() {
        if !root.exists() {
            log!(LogLevel::Info, "{:?}", root);
            std::fs::create_dir_all(root).unwrap();
        }
    }
    let mut file = File::options()
        .read(true)
        .write(true)
        .create(true)
        .open(path)
        .unwrap();
    log!(
        LogLevel::Info,
        "Saved {} {} {}",
        path.to_str().unwrap(),
        addr,
        buf.len()
    );
    file.seek(std::io::SeekFrom::Start(addr)).unwrap();
    file.write_all(buf).unwrap();
    file.set_len(addr + buf.len() as u64).unwrap();
}
