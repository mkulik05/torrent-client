use std::{env, io::Write};
mod bencode;
mod peers;
mod torrent;
use bencode::decode_bencoded_value;
use peers::{BlockRequest, Peer, TrackerReq};
use tokio::time::{sleep, Duration};
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
            let pieces_n = torrent.info.piece_length / 16384 + 1; // 16kiB block
            let mut begin = 0;
            let mut f = std::fs::File::create(&args[3]).unwrap();
            for n in 0..pieces_n {
                let length = if n == (pieces_n - 1) {
                    torrent.info.piece_length - pieces_n * 16384
                } else {
                    16384
                };
                if length <= 0 {
                    break;
                }
                let req = BlockRequest {
                    piece_i,
                    begin,
                    length: length as u32,
                };
                let bytes = peer.fetch(&req).await;
                f.write_all(&bytes).unwrap();
                begin += 16384;
                sleep(Duration::from_millis(50)).await;
            }
            println!("Piece {} downloaded to {}.", piece_i, &args[3]);
        }
        _ => println!("unknown command: {}", args[1]),
    }
}
