use reqwest::Client;
use serde::Serialize;
use sha1::{Digest, Sha1};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use std::collections::HashMap;
use std::env;
use std::fs::File;
use std::io::{IoSlice, Read, Write};
use std::ops::Index;
use std::string::ToString;
use tokio::net::TcpStream;

#[derive(Clone, Debug, Serialize)]
enum BencodeValue {
    Dict(HashMap<String, Self>),
    Bytes(Vec<u8>),
    Num(i64),
    List(Vec<Self>),
    Null,
}

impl BencodeValue {
    fn to_lossy_string(&self) -> String {
        if let BencodeValue::Bytes(arr) = self {
            String::from_utf8_lossy(arr).into()
        } else {
            "Null".to_string()
        }
    }
    fn to_string(&self) -> String {
        if let BencodeValue::List(_) = self {
            self.to_string_with_sep(", ")
        } else {
            self.to_string_with_sep(",")
        }
    }
    fn to_string_with_sep(&self, arr_sep: &str) -> String {
        let a = serde_json::Value::Object(serde_json::Map::<String, serde_json::Value>::new());
        a.to_string();
        match self {
            BencodeValue::Bytes(_) => format!("{:?}", self.to_lossy_string()),
            BencodeValue::Num(n) => n.to_string(),
            BencodeValue::Null => "Null".to_string(),
            BencodeValue::Dict(d) => {
                let mut buf = String::from("{");
                let mut keys = d.keys().collect::<Vec<_>>();
                keys.sort();
                for key in &keys {
                    buf += format!("\"{}\":", key).as_str();
                    buf += d.get(*key).unwrap().to_string_with_sep(arr_sep).as_str();
                    buf += ","
                }
                if !keys.is_empty() {
                    buf = buf.strip_suffix(",").unwrap().to_owned();
                }
                buf + "}"
            }
            BencodeValue::List(arr) => {
                let mut buf = String::from("[");
                for el in arr {
                    buf += &el.to_string_with_sep(arr_sep);
                    buf += arr_sep
                }
                if !arr.is_empty() {
                    buf = buf.strip_suffix(arr_sep).unwrap().to_string();
                }
                buf + "]"
            }
        }
    }
    fn encode<W: Write>(&self, writer: &mut W) {
        match self {
            BencodeValue::Bytes(bytes) => {
                writer
                    .write_vectored(&[
                        IoSlice::new(bytes.len().to_string().as_bytes()),
                        IoSlice::new(b":"),
                        IoSlice::new(bytes),
                    ])
                    .unwrap();
            }
            BencodeValue::Num(n) => {
                writer
                    .write_vectored(&[
                        IoSlice::new(b"i"),
                        IoSlice::new(n.to_string().as_bytes()),
                        IoSlice::new(b"e"),
                    ])
                    .unwrap();
            }
            BencodeValue::List(arr) => {
                writer.write_all(b"l").unwrap();
                for el in arr {
                    el.encode(writer);
                }
                writer.write_all(b"e").unwrap();
            }
            BencodeValue::Dict(dict) => {
                writer.write_all(b"d").unwrap();

                let mut keys = dict.keys().collect::<Vec<_>>();
                keys.sort();
                for key in keys {
                    writer
                        .write_vectored(&[
                            IoSlice::new(key.as_bytes().len().to_string().as_bytes()),
                            IoSlice::new(b":"),
                            IoSlice::new(key.as_bytes()),
                        ])
                        .unwrap();
                    dict.get(key).unwrap().encode(writer);
                }
                writer.write_all(b"e").unwrap();
            }
            _ => {}
        }
    }
}

impl Index<usize> for BencodeValue {
    type Output = BencodeValue;
    fn index(&self, i: usize) -> &Self::Output {
        if let BencodeValue::List(arr) = self {
            &arr[i]
        } else {
            &BencodeValue::Null
        }
    }
}

impl Index<&str> for BencodeValue {
    type Output = BencodeValue;
    fn index(&self, key: &str) -> &Self::Output {
        if let BencodeValue::Dict(dict) = self {
            dict.get(key).unwrap_or(&BencodeValue::Null)
        } else {
            &BencodeValue::Null
        }
    }
}

fn decode_bencoded_value(mut encoded_value: &[u8]) -> (BencodeValue, usize) {
    match encoded_value[0] {
        x if x.is_ascii_digit() => {
            let colon_index = encoded_value.iter().position(|b| *b == b':').unwrap();
            let number_string = &encoded_value[..colon_index];
            let number = std::str::from_utf8(number_string)
                .unwrap()
                .parse::<i64>()
                .unwrap();
            let string = &encoded_value[colon_index + 1..colon_index + 1 + number as usize];
            return (
                BencodeValue::Bytes(string.into()),
                colon_index + 1 + number as usize,
            );
        }
        b'l' => {
            let mut list = Vec::new();
            let mut total_len = 2; // counting symbols 'l' and 'e'
            encoded_value = &encoded_value[1..];
            loop {
                if encoded_value.is_empty() || encoded_value.starts_with(&[b'e']) {
                    break;
                }
                let (list_part, len) = decode_bencoded_value(encoded_value);
                total_len += len;
                list.push(list_part.clone());
                encoded_value = &encoded_value[len..];
            }
            return (BencodeValue::List(list), total_len);
        }
        b'i' => {
            let num_end = encoded_value.iter().position(|b| *b == b'e').unwrap();
            let int_res = std::str::from_utf8(&encoded_value[1..num_end])
                .unwrap()
                .parse::<i64>();
            if let Ok(n) = int_res {
                return (BencodeValue::Num(n), num_end + 1);
            }
        }
        b'd' => {
            let mut dict = HashMap::new();
            encoded_value = &encoded_value[1..];
            let mut total_len = 2;
            loop {
                if encoded_value.is_empty() || encoded_value.starts_with(&[b'e']) {
                    break;
                }
                let (BencodeValue::Bytes(key), key_len) = decode_bencoded_value(encoded_value)
                else {
                    panic!("Invalid torrent structure")
                };
                let key = String::from_utf8_lossy(&key);
                let (value, val_len) = decode_bencoded_value(&encoded_value[key_len..]);
                total_len += val_len + key_len;
                dict.insert(key.into_owned(), value);
                encoded_value = &encoded_value[val_len + key_len..];
            }
            return (BencodeValue::Dict(dict), total_len);
        }
        _ => {}
    }
    panic!(
        "Unhandled encoded value: {}",
        String::from_utf8_lossy(encoded_value)
    )
}

fn parse_torrent_file(path: &str) -> BencodeValue {
    let mut torrent_file = File::open(path).unwrap();
    let mut bytes = Vec::new();
    torrent_file.read_to_end(&mut bytes).unwrap();
    decode_bencoded_value(&bytes).0
}

fn get_hash_bytes(src: &BencodeValue) -> Vec<u8> {
    let mut hasher = Sha1::new();
    let mut bytes: Vec<u8> = Vec::new();
    src.encode(&mut bytes);
    hasher.update(bytes);
    let info_hash = hasher.finalize().to_vec();
    info_hash
} 

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
            let metainfo = parse_torrent_file(&args[2]);
            let mut hasher = Sha1::new();
            let mut bytes: Vec<u8> = Vec::new();
            metainfo["info"].encode(&mut bytes);
            hasher.update(bytes);
            let info_hash = hasher.finalize();
            let BencodeValue::Bytes(ref byte_pieces) = metainfo["info"]["pieces"] else {
                panic!("ERROR")
            };
            let mut hashes = Vec::new();
            let n = byte_pieces.len() / 20;
            for i in 0..n {
                hashes.push(hex::encode(&byte_pieces[i * 20..(i + 1) * 20]));
            }
            println!(
                "Tracker URL: {}\nLength: {}\nInfo Hash: {}\nPiece Length: {}\nPiece Hashes:\n{}",
                metainfo["announce"].to_lossy_string(),
                metainfo["info"]["length"].to_string(),
                hex::encode(info_hash),
                metainfo["info"]["piece length"].to_string(),
                hashes.join("\n")
            );
        }
        "peers" => {
            let metainfo = parse_torrent_file(&args[2]);
            let info_hash = get_hash_bytes(&metainfo["info"]);
            let url_encoded_hash: String =
                info_hash.iter().map(|b| format!("%{:02x}", b)).collect();
            let url = format!(
                "{}?info_hash={}",
                metainfo["announce"].to_lossy_string(),
                url_encoded_hash.as_str()
            );
            let params = &[
                ("peer_id", "00112253445566770099"),
                ("port", "6881"),
                ("uploaded", "0"),
                ("downloaded", "0"),
                ("left", &metainfo["info"]["length"].to_string()),
                ("compact", "1"),
            ];
            let client = Client::new();
            let body = client
                .get(url)
                .query(params)
                .send()
                .await
                .unwrap()
                .bytes()
                .await
                .unwrap();
            let response = decode_bencoded_value(&body).0;
            println!("{}", response.to_string());
            let BencodeValue::Bytes(ref peers_bytes) = response["peers"] else {
                panic!("Error")
            };
            let peers_n = peers_bytes.len() / 6;
            let mut ips_str = Vec::new();
            for i in 0..peers_n {
                ips_str.push(format!(
                    "{}.{}.{}.{}:{}",
                    peers_bytes[i * 6],
                    peers_bytes[i * 6 + 1],
                    peers_bytes[i * 6 + 2],
                    peers_bytes[i * 6 + 3],
                    u16::from_be_bytes([peers_bytes[i * 6 + 4], peers_bytes[i * 6 + 5]])
                ))
            }
            println!("{}", ips_str.join("\n"));
        },
        "handshake" => {
            let metainfo = parse_torrent_file(&args[2]);
            let peer_addr = &args[3];
            let mut msg = Vec::new();
            msg.push(b"\x13"[0]);// 0x13 = 19
            msg.extend_from_slice(b"BitTorrent protocol");
            msg.extend_from_slice(&[0; 8]);
            msg.extend_from_slice(&get_hash_bytes(&metainfo["info"]));
            msg.extend_from_slice(b"00112233445566770099");
            let mut stream = TcpStream::connect(peer_addr).await.unwrap();
            stream.write_all(&msg).await.unwrap();
            let mut response = [0; 68];
            stream.read_exact(&mut response).await.unwrap();
            let peer_id = &response[response.len() - 20..response.len()];
            println!("Peer ID: {}", hex::encode(peer_id));
        }
        _ => println!("unknown command: {}", args[1]),
    }
}
