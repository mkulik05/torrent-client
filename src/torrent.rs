use crate::bencode::BencodeValue;
use crate::logger::{log, LogLevel};
use sha1::{Digest, Sha1};
use std::fs::File;
use std::io::Read;

#[derive(Debug)]
pub struct TorrentFiles {
    pub length: u64,
    pub path: String,
}

#[derive(Debug)]
pub struct Torrent {
    pub tracker_url: String,
    // pub tracker_list: Vec<String>,
    pub info: TorrentInfo,
    pub info_hash: Vec<u8>,
}

#[derive(Debug)]
pub struct TorrentInfo {
    pub length: u64,
    pub files: Option<Vec<TorrentFiles>>,
    pub name: String,
    pub piece_length: u64,
    pub piece_hashes: Vec<Vec<u8>>,
}

impl Torrent {
    pub fn new(path: &str) -> anyhow::Result<Self> {
        log!(LogLevel::Debug, "Parsing torrent file");
        let parsed_file = Torrent::parse_torrent_file(path)?;
        println!("{}", parsed_file["announce-list"][0][0].to_string());
        let BencodeValue::Num(length) = parsed_file["info"]["length"] else {
            anyhow::bail!("Invalid torrent file structure");
        };
        let BencodeValue::Num(piece_length) = parsed_file["info"]["piece length"] else {
            anyhow::bail!("Invalid torrent file structure");
        };
        let BencodeValue::Bytes(ref byte_pieces) = parsed_file["info"]["pieces"] else {
            anyhow::bail!("Invalid torrent file structure");
        };
        log!(LogLevel::Debug, "Parsed successfully");
        let mut piece_hashes = Vec::new();
        let n = byte_pieces.len() / 20;
        for i in 0..n {
            piece_hashes.push(byte_pieces[i * 20..(i + 1) * 20].to_vec());
        }
        let torrent_info = TorrentInfo {
            length: length as u64,
            piece_length: piece_length as u64,
            piece_hashes,
            name: parsed_file["info"]["name"].to_lossy_string(),
            files: if let BencodeValue::List(list) = &parsed_file["info"]["files"] {
                let mut res = Vec::new();
                for el in list {
                    let BencodeValue::Dict(dict) = el else {
                        anyhow::bail!("wrong torrent file structure");
                    };
                    let file = TorrentFiles {
                        length: dict["length"].to_lossy_string().parse()?,
                        path: dict["path"].to_lossy_string(),
                    };
                    res.push(file)
                }
                Some(res)
            } else {
                None
            },
        };

        Ok(Torrent {
            tracker_url: parsed_file["announce"].to_lossy_string(),
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
}
