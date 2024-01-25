use sha1::{Digest, Sha1};
use std::collections::HashMap;
use std::env;
use std::fs::File;
use std::io::{Read, Write};
use std::ops::Index;
use std::string::ToString;

#[derive(Clone, Debug)]
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
    fn encode<W: Write>(&self, writer: &mut W) {
        match self {
            BencodeValue::Bytes(bytes) => {
                writer.write_all(&bytes.len().to_le_bytes()).unwrap();
                writer.write_all(b":").unwrap();
                writer.write_all(bytes).unwrap();
            }
            BencodeValue::Num(n) => {
                writer.write_all(b"i").unwrap();
                writer.write_all(&n.to_le_bytes()).unwrap();
                writer.write_all(b"e").unwrap();
            },
            BencodeValue::List(arr) => {
                writer.write_all(b"l").unwrap();
                for el in arr {
                    el.encode(writer);
                }
                writer.write_all(b"e").unwrap();
            },
            BencodeValue::Dict(dict) => {
                writer.write_all(b"d").unwrap();
                let mut keys = dict.keys().collect::<Vec<_>>();
                keys.sort();
                for key in keys {
                    writer.write_all(key.as_bytes()).unwrap();
                    dict.get(key).unwrap().encode(writer);
                }
                writer.write_all(b"e").unwrap();
            }
            _ => {}
        }
    }
}

impl ToString for BencodeValue {
    fn to_string(&self) -> String {
        match self {
            BencodeValue::Bytes(_) => format!("{:?}", self.to_lossy_string()),
            BencodeValue::Num(n) => n.to_string(),
            BencodeValue::Null => "Null".to_string(),
            BencodeValue::Dict(d) => {
                let mut buf = String::from("{");
                let mut keys = d.keys().collect::<Vec<_>>();
                keys.sort();
                for key in keys {
                    buf += format!("\"{}\":", key).as_str(); 
                    buf += d.get(key).unwrap().to_string().as_str();
                    buf += ","
                }
                if !keys.is_empty() {
                    buf = buf.strip_suffix(",").unwrap().to_owned();
                }
                buf + "}" 
            },
            BencodeValue::List(arr) => {
                let mut buf = String::from("[");
                for el in arr {
                    buf += &el.to_string();
                    buf += ", "
                }
                if !arr.is_empty() {
                    buf = buf.strip_suffix(", ").unwrap().to_string();
                }
                buf + "]"
            }
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
            &dict.get(key).unwrap_or(&BencodeValue::Null)
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

fn main() {
    let args: Vec<String> = env::args().collect();
    let command = &args[1];

    match command.as_str() {
        "decode" => {
            let encoded_value = &args[2];
            let decoded_value = decode_bencoded_value(encoded_value.as_bytes()).0;
            println!("{}", decoded_value.to_string());
        }
        "info" => {
            let mut torrent_file = File::open(&args[2]).unwrap();
            let mut bytes = Vec::new();
            torrent_file.read_to_end(&mut bytes).unwrap();
            let metainfo = decode_bencoded_value(&bytes).0;
            let mut hasher = Sha1::new();
            
            let mut bytes: Vec<u8> = Vec::new();
            metainfo["info"].encode(&mut bytes);
            hasher.update(bytes);
            let result = hasher.finalize();
            println!(
                "Tracker URL: {}\nLength: {}\nInfo Hash: {}",
                metainfo["announce"].to_lossy_string(),
                metainfo["info"]["length"].to_string(),
                hex::encode(&result)
            );
        }
        _ => println!("unknown command: {}", args[1]),
    }
}
