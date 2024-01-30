use std::env;
mod bencode;
mod download;
mod peers;
mod torrent;
mod logger;
mod tracker;
use download::Downloader;
use peers::Peer;
use tracker::TrackerReq;
use torrent::Torrent;
use std::sync::Arc;
use crate::logger::{log, LogLevel};
use tokio::sync::mpsc;

fn handle_result<T>(res: anyhow::Result<T>) -> T {
    match res {
        Ok(v) => v,
        Err(err) => {
            log!(LogLevel::Fatal, "error {:?}", err);
            panic!("error {:?}", err);
        }
    }
  }

#[tokio::main]
async fn main() {
    handle_result(logger::Logger::init(format!("/tmp/log{}.txt", chrono::Local::now().format("%d-%m-%Y_%H-%M-%S"))));
    let args: Vec<String> = env::args().collect();
    let command = &args[1];

    match command.as_str() {
        "info" => {
            let torrent = Torrent::new(&args[2]).unwrap();
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
        "download" => {
            let torrent = handle_result(Torrent::new(&args[4]));
            let tracker_req = TrackerReq::init(&torrent);
            let tracker_resp = handle_result(tracker_req.send(&torrent).await);
            let mut peers = Vec::new();
            for peer in tracker_resp.peers {
                peers.push(handle_result(Peer::new(&peer).await));
            }
            let torrent = Arc::new(torrent);
            let mut downloader = handle_result(Downloader::new(torrent.clone(), peers.swap_remove(0), 0..(torrent.info.piece_hashes.len() as u64), &args[3]));
            handle_result(downloader.download().await);
            println!("Downloaded {} to {}.", &args[4], &args[3]);
        }
        _ => println!("unknown command: {}", args[1]),
    }
}
