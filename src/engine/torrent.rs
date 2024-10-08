use std::fs::File;
use std::io::Read;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use sha1::{Digest, Sha1};

use super::bencode::BencodeValue;
use super::logger::{LogLevel, log};


#[derive(Debug, Clone)]
pub struct PieceBitmap {
    pieces_n: usize,
    pub bitmap: Vec<u8>
}

impl PieceBitmap {
    pub fn new(pieces_n: usize) -> Self {
        PieceBitmap { pieces_n, bitmap: vec![0; (pieces_n as f64 / 8.0).ceil() as usize] }
    }
    pub fn add(&mut self, piece_i: usize) {
        if piece_i > self.pieces_n {return}
        let cell_i = piece_i as usize / 8;
        let bit_i = piece_i % 8;
        let mut mask = 0b1000_0000;
        for _ in 0..bit_i {
            mask >>= 1;
        }
        self.bitmap[cell_i] |= mask;
    }
    pub fn diff(&self, b2: &PieceBitmap) -> Vec<usize> {
        let mut res = Vec::new();
        if self.bitmap.len() != b2.bitmap.len() {
            panic!("Length should be same")
        }
        for i in 0..self.bitmap.len() {
            if self.bitmap[i] != b2.bitmap[i] {
                let mut bits = self.bitmap[i] ^ b2.bitmap[i];
                for j in 0..8 {
                    if i * 8 + j > self.pieces_n {break}
                    if bits & 0b0000_0001 != 0 {
                        res.push(i * 8 + j);
                    }
                    bits >>= 1;
                }
            }
        }
        res 
    }
}


#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TorrentFile {
    pub length: u64,
    pub path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Torrent {
    pub tracker_url: String,
    pub tracker_urls: Option<Vec<String>>,
    pub info: TorrentInfo,
    pub info_hash: Vec<u8>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TorrentInfo {
    // total length
    pub length: u64,
    pub files: Option<Vec<TorrentFile>>,
    pub name: String,
    pub piece_length: u64,
    pub piece_hashes: Vec<Vec<u8>>,
}

impl Torrent {
    pub fn new(path: &str) -> anyhow::Result<Self> {
        log!(LogLevel::Debug, "Parsing torrent file");
        let parsed_file = Torrent::parse_torrent_file(path)?;
        let length = if let BencodeValue::Num(n) = parsed_file["info"]["length"] {
            Some(n as u64)
        } else {
            None
        };
        let mut length = length.unwrap_or(0);
        let files = if let BencodeValue::List(list) = &parsed_file["info"]["files"] {
            let mut res = Vec::new();
            for el in list {
                let BencodeValue::Dict(dict) = el else {
                    anyhow::bail!("wrong torrent file structure");
                };
                let BencodeValue::Num(len) = dict["length"] else {
                    anyhow::bail!("wrong torrent file structure");
                };
                let BencodeValue::List(ref path) = dict["path"] else {
                    anyhow::bail!("wrong torrent file structure");
                };

                let mut res_path = PathBuf::new();
                for subpath in path {
                    res_path = res_path.join(subpath.to_lossy_string());
                }
                let file = TorrentFile {
                    length: len as u64,
                    path: res_path.to_str().unwrap().to_owned(),
                };
                length += len as u64;
                res.push(file)
            }
            Some(res)
        } else {
            None
        };
        let BencodeValue::Num(piece_length) = parsed_file["info"]["piece length"] else {
            anyhow::bail!("Invalid torrent file structure");
        };
        let BencodeValue::Bytes(ref byte_pieces) = parsed_file["info"]["pieces"] else {
            println!("nooo");
            anyhow::bail!("Invalid torrent file structure");
        };
        log!(LogLevel::Debug, "Parsed successfully");
        let mut piece_hashes = Vec::new();
        let n = byte_pieces.len() / 20;
        for i in 0..n {
            piece_hashes.push(byte_pieces[i * 20..(i + 1) * 20].to_vec());
        }
        let torrent_info = TorrentInfo {
            length,
            piece_length: piece_length as u64,
            piece_hashes,
            name: parsed_file["info"]["name"].to_lossy_string(),
            files,
        };

        let mut tracker_urls = None;

        if let BencodeValue::List(ref be_trackers) = parsed_file["announce-list"] {
            let mut trackers = Vec::new();
            for tracker in be_trackers {
                log!(LogLevel::Info, "{:?}", tracker[0].to_lossy_string());
                trackers.push(tracker[0].to_lossy_string());
            }
            tracker_urls = Some(trackers);
        }

        Ok(Torrent {
            tracker_url: parsed_file["announce"].to_lossy_string(),
            tracker_urls,
            info: torrent_info,
            info_hash: Torrent::bencode_hash(&parsed_file["info"])?,
        })
    }
    fn parse_torrent_file(path: &str) -> anyhow::Result<BencodeValue> {
        let mut torrent_file = File::open(path)?;
        let mut bytes = Vec::new();
        torrent_file.read_to_end(&mut bytes)?;
        Ok(BencodeValue::decode_bencoded_value(&bytes)?.0)
    }
    fn bencode_hash(src: &BencodeValue) -> anyhow::Result<Vec<u8>> {
        let mut hasher = Sha1::new();
        let mut bytes: Vec<u8> = Vec::new();
        src.encode(&mut bytes)?;
        hasher.update(bytes);
        let info_hash = hasher.finalize().to_vec();
        Ok(info_hash)
    }
    pub fn bytes_hash(src: &Vec<u8>) -> Vec<u8> {
        let mut hasher = Sha1::new();
        hasher.update(src);
        hasher.finalize().to_vec()
    }

    pub fn get_piece_length(&self, piece_i: usize) -> u64 {
        if piece_i == self.info.piece_hashes.len() - 1 {
            self.info.length - piece_i as u64 * self.info.piece_length
        } else {
            self.info.piece_length
        }
    }
}
