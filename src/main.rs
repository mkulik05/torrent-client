use std::env;
mod bencode;
mod download;
mod peers;
mod torrent;
use bencode::BencodeValue;
use download::Downloader;
use peers::{Peer, TrackerReq};
use torrent::Torrent;
use std::sync::Arc;

#[tokio::main]
async fn main() {
    let args: Vec<String> = env::args().collect();
    let command = &args[1];

    match command.as_str() {
        "decode" => {
            let encoded_value = &args[2];
            let decoded_value = BencodeValue::decode_bencoded_value(encoded_value.as_bytes()).0;
            println!("{}", decoded_value.to_string());
        }
        "info" => {
            let torrent = Torrent::new(&args[2]);
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
        "peers" => {
            let torrent = Torrent::new(&args[2]);
            let tracker_req = TrackerReq::init(&torrent);
            let tracker_resp = tracker_req.send(&torrent).await;
            println!("{}", tracker_resp.peers.join("\n"));
        }
        "handshake" => {
            let torrent = Torrent::new(&args[2]);
            let peer = Peer::new(&args[3], &torrent, true).await;
            println!("Peer ID: {}", peer.peer_id);
        }
        "download_piece" => {
            let torrent = Torrent::new(&args[4]);
            let tracker_req = TrackerReq::init(&torrent);
            let tracker_resp = tracker_req.send(&torrent).await;
            let mut peer = Peer::new(&tracker_resp.peers[0], &torrent, false).await;
            peer.send_interested_msg().await;
            let piece_i = args[5].parse::<u32>().unwrap();
            let torrent = Arc::new(torrent);
            
            let mut downloader = Downloader::new(torrent, peer, Some(piece_i), &args[3]); 
            downloader.download().await;
            println!("Piece {} downloaded to {}.", piece_i, &args[3]);
        }
        "download" => {
            let torrent = Torrent::new(&args[4]);
            let tracker_req = TrackerReq::init(&torrent);
            let tracker_resp = tracker_req.send(&torrent).await;
            let mut peer = Peer::new(&tracker_resp.peers[0], &torrent, false).await;
            peer.send_interested_msg().await;
            let torrent = Arc::new(torrent);
            let mut downloader = Downloader::new(torrent, peer, None, &args[3]);
            downloader.download().await;
            println!("Downloaded {} to {}.", &args[4], &args[3]);
        }
        _ => println!("unknown command: {}", args[1]),
    }
}
