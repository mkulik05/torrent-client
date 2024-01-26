use std::env;
mod bencode;
mod peers;
mod torrent;
use bencode::decode_bencoded_value;
use peers::{Peer, TrackerReq};
use torrent::Torrent;

#[tokio::main]
async fn main() {
    let args: Vec<String> = env::args().collect();
    let command = &args[1];

    match command.as_str() {
        "decode" => {
            let encoded_value = &args[2];
            let decoded_value = decode_bencoded_value(encoded_value.as_bytes()).0;
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
            let peer = Peer {
                addr: args[3].clone(),
            };
            let peer_id = peer.handshake(&torrent).await;
            println!("Peer ID: {}", peer_id);
        }
        "download_piece" => {}
        _ => println!("unknown command: {}", args[1]),
    }
}
