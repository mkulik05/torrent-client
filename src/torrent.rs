use crate::bencode::BencodeValue;
use sha1::{Digest, Sha1};
use std::fs::File;
use std::io::Read;

pub struct Torrent {
    pub tracker_url: String,
    pub info: TorrentInfo,
    pub info_hash: Vec<u8>,
}
pub struct TorrentInfo {
    pub length: i64,
    pub piece_length: i64,
    pub piece_hashes: Vec<Vec<u8>>,
}

impl Torrent {
    pub fn new(path: &str) -> Self {
        let parsed_file = Torrent::parse_torrent_file(path);
        let BencodeValue::Num(length) = parsed_file["info"]["length"] else {
            panic!("")
        };
        let BencodeValue::Num(piece_length) = parsed_file["info"]["piece length"] else {
            panic!("")
        };
        let BencodeValue::Bytes(ref byte_pieces) = parsed_file["info"]["pieces"] else {
            panic!("ERROR")
        };
        let mut piece_hashes = Vec::new();
        let n = byte_pieces.len() / 20;
        for i in 0..n {
            piece_hashes.push(byte_pieces[i * 20..(i + 1) * 20].to_vec());
        }
        let torrent_info = TorrentInfo {
            length,
            piece_length,
            piece_hashes,
        };

        Torrent {
            tracker_url: parsed_file["announce"].to_lossy_string(),
            info: torrent_info,
            info_hash: Torrent::get_hash_bytes(&parsed_file["info"]),
        }
    }
    fn parse_torrent_file(path: &str) -> BencodeValue {
        let mut torrent_file = File::open(path).unwrap();
        let mut bytes = Vec::new();
        torrent_file.read_to_end(&mut bytes).unwrap();
        BencodeValue::decode_bencoded_value(&bytes).0
    }
    pub fn get_hash_bytes(src: &BencodeValue) -> Vec<u8> {
        let mut hasher = Sha1::new();
        let mut bytes: Vec<u8> = Vec::new();
        src.encode(&mut bytes);
        hasher.update(bytes);
        let info_hash = hasher.finalize().to_vec();
        info_hash
    }
}
