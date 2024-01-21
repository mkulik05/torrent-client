use serde_json::{self, Map, Value};
use std::env;
use std::fs::File;
use std::io::Read;
fn decode_bencoded_value(mut encoded_value: &[u8]) -> (Value, usize) {
    match encoded_value[0] {
        x if x.is_ascii_digit() => {
            let colon_index = encoded_value.iter().position(|b| *b == b':').unwrap();
            let number_string = &encoded_value[..colon_index];
            let number = std::str::from_utf8(number_string)
                .unwrap()
                .parse::<i64>()
                .unwrap();
            let string = &encoded_value[colon_index + 1..colon_index + 1 + number as usize];
            let string = String::from_utf8_lossy(string);
            return (string.into(), colon_index + 1 + number as usize);
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
            return (Value::Array(list), total_len);
        }
        b'i' => {
            let num_end = encoded_value.iter().position(|b| *b == b'e').unwrap();
            let int_res = std::str::from_utf8(&encoded_value[1..num_end])
                .unwrap()
                .parse::<i64>();
            if let Ok(n) = int_res {
                return (n.into(), num_end + 1);
            }
        }
        b'd' => {
            let mut dict = Map::new();
            encoded_value = &encoded_value[1..];
            let mut total_len = 2;
            loop {
                if encoded_value.is_empty() || encoded_value.starts_with(&[b'e']) {
                    break;
                }
                let (key, key_len) = decode_bencoded_value(encoded_value);
                let (value, val_len) = decode_bencoded_value(&encoded_value[key_len..]);
                total_len += val_len + key_len;
                dict.insert(key.as_str().unwrap().to_owned(), value);
                encoded_value = &encoded_value[val_len + key_len..];
            }
            return (dict.into(), total_len);
        }
        _ => {}
    }
    panic!("Unhandled encoded value: {}", String::from_utf8_lossy(encoded_value))
}

// Usage: your_bittorrent.sh decode "<encoded_value>"
fn main() {
    let args: Vec<String> = env::args().collect();
    let command = &args[1];

    match command.as_str() {
        "decode" => {
            let encoded_value = &args[2];
            let decoded_value = decode_bencoded_value(encoded_value.as_bytes()).0;
            println!("{}", decoded_value);
        }
        "info" => {
            let mut torrent_file = File::open(&args[2]).unwrap();
            let mut bytes = Vec::new();
            torrent_file.read_to_end(&mut bytes).unwrap();
            let metainfo = decode_bencoded_value(&bytes).0;
            println!("Tracker URL: {}\nLength: {}", metainfo["announce"].as_str().unwrap(), metainfo["info"]["length"]);
        }
        _ => println!("unknown command: {}", args[1]),
    }
}
