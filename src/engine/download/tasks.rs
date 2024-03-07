use std::collections::VecDeque;
use std::ops::Range;
use std::sync::Arc;

use serde::{Deserialize, Serialize};

use super::super::torrent::Torrent;

const CHUNKS_PER_TASK: u16 = 60;
pub const MAX_CHUNKS_TASKS: usize = 100;
pub const CHUNK_SIZE: u64 = 16384;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PieceTask {
    pub piece_i: u16,
    pub total_chunks: u16,
    pub chunks_done: u16,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChunksTask {
    pub piece_i: u16,
    pub chunks: Range<u16>,
    pub includes_last_chunk: bool,
}

pub fn get_piece_tasks(torrent: Arc<Torrent>, pieces_done: Vec<usize>) -> VecDeque<PieceTask> {
    let mut pieces_tasks = VecDeque::with_capacity(torrent.info.piece_hashes.len());
    let total_chunks = (torrent.info.piece_length as f64 / CHUNK_SIZE as f64).ceil() as u64;
    for i in 0..torrent.info.piece_hashes.len() {
        if pieces_done.iter().any(|&x| x == i) {
            continue;
        }
        pieces_tasks.push_back(PieceTask {
            piece_i: i as u16,
            total_chunks: if i == (torrent.info.piece_hashes.len() - 1) {
                ((torrent.info.length
                    - (torrent.info.piece_hashes.len() - 1) as u64 * torrent.info.piece_length)
                    as f64
                    / CHUNK_SIZE as f64)
                    .ceil() as u16
            } else {
                total_chunks as u16
            },
            chunks_done: 0,
        })
    }
    pieces_tasks
}

pub fn add_chunks_tasks(
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
        task.chunks_done = chunks_up_border;
    }
}
