use std::collections::HashMap;
use std::io::{IoSlice, Write};
use std::ops::Index;

use serde::Serialize;

#[derive(Clone, Debug, Serialize)]
pub enum BencodeValue {
    Dict(HashMap<String, Self>),
    Bytes(Vec<u8>),
    Num(i64),
    List(Vec<Self>),
    Null,
}

impl BencodeValue {
    pub fn decode_bencoded_value(mut encoded_value: &[u8]) -> anyhow::Result<(Self, usize)> {
        match encoded_value[0] {
            x if x.is_ascii_digit() => {
                let colon_index = if let Some(e) = encoded_value.iter().position(|b| *b == b':') {
                    e
                } else {
                    anyhow::bail!("Invalid bencoded value, can't process it")
                };
                let number_string = &encoded_value[..colon_index];
                let number = std::str::from_utf8(number_string)?.parse::<i64>()?;
                let string = &encoded_value[colon_index + 1..colon_index + 1 + number as usize];
                return Ok((
                    BencodeValue::Bytes(string.into()),
                    colon_index + 1 + number as usize,
                ));
            }
            b'l' => {
                let mut list = Vec::new();
                let mut total_len = 2; // counting symbols 'l' and 'e'
                encoded_value = &encoded_value[1..];
                loop {
                    if encoded_value.is_empty() || encoded_value.starts_with(&[b'e']) {
                        break;
                    }
                    let (list_part, len) = BencodeValue::decode_bencoded_value(encoded_value)?;
                    total_len += len;
                    list.push(list_part.clone());
                    encoded_value = &encoded_value[len..];
                }
                return Ok((BencodeValue::List(list), total_len));
            }
            b'i' => {
                let num_end = if let Some(e) = encoded_value.iter().position(|b| *b == b'e') {
                    e
                } else {
                    anyhow::bail!("Invalid bencoded value, can't process it")
                };
                let int_res = std::str::from_utf8(&encoded_value[1..num_end])?.parse::<i64>();
                if let Ok(n) = int_res {
                    return Ok((BencodeValue::Num(n), num_end + 1));
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
                    let (BencodeValue::Bytes(key), key_len) =
                        BencodeValue::decode_bencoded_value(encoded_value)?
                    else {
                        panic!("Invalid torrent structure")
                    };
                    let key = String::from_utf8_lossy(&key);
                    let (value, val_len) =
                        BencodeValue::decode_bencoded_value(&encoded_value[key_len..])?;
                    total_len += val_len + key_len;
                    dict.insert(key.into_owned(), value);
                    encoded_value = &encoded_value[val_len + key_len..];
                }
                return Ok((BencodeValue::Dict(dict), total_len));
            }
            _ => {}
        }
        panic!(
            "Unhandled encoded value: {}",
            String::from_utf8_lossy(encoded_value)
        )
    }

    pub fn to_lossy_string(&self) -> String {
        if let BencodeValue::Bytes(arr) = self {
            String::from_utf8_lossy(arr).into()
        } else {
            "Null".to_string()
        }
    }
    pub fn to_string(&self) -> String {
        if let BencodeValue::List(_) = self {
            self.to_string_with_sep(", ")
        } else {
            self.to_string_with_sep(",")
        }
    }
    fn to_string_with_sep(&self, arr_sep: &str) -> String {
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
                    buf += d.get(*key).expect("Key exist").to_string_with_sep(arr_sep).as_str();
                    buf += ","
                }
                if !keys.is_empty() {
                    buf = buf.strip_suffix(',').expect("Suffix is added anyway").to_owned();
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
                    buf = buf.strip_suffix(arr_sep).expect("Suffix is added anyway").to_string();
                }
                buf + "]"
            }
        }
    }
    pub fn encode(&self, buffer: &mut Vec<u8>) -> anyhow::Result<()>{
        match self {
            BencodeValue::Bytes(bytes) => {
                buffer
                    .write_vectored(&[
                        IoSlice::new(bytes.len().to_string().as_bytes()),
                        IoSlice::new(b":"),
                        IoSlice::new(bytes),
                    ])?;
            }
            BencodeValue::Num(n) => {
                buffer
                    .write_vectored(&[
                        IoSlice::new(b"i"),
                        IoSlice::new(n.to_string().as_bytes()),
                        IoSlice::new(b"e"),
                    ])?;
            }
            BencodeValue::List(arr) => {
                buffer.write_all(b"l")?;
                for el in arr {
                    el.encode(buffer)?;
                }
                buffer.write_all(b"e")?;
            }
            BencodeValue::Dict(dict) => {
                buffer.write_all(b"d")?;

                let mut keys = dict.keys().collect::<Vec<_>>();
                keys.sort();
                for key in keys {
                    buffer
                        .write_vectored(&[
                            IoSlice::new(key.as_bytes().len().to_string().as_bytes()),
                            IoSlice::new(b":"),
                            IoSlice::new(key.as_bytes()),
                        ])?;
                    dict.get(key).expect("Key exists").encode(buffer)?;
                }
                buffer.write_all(b"e")?;
            }
            _ => {}
        }
        Ok(())
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
