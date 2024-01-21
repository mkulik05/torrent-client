use serde_json::{self, Value};
use std::env;
fn decode_bencoded_value(mut encoded_value: &str) -> (Value, usize) {
    // If encoded_value starts with a digit, it's a number
    match encoded_value.chars().next().unwrap() {
        x if x.is_ascii_digit() => {
            let colon_index = encoded_value.find(':').unwrap();
            let number_string = &encoded_value[..colon_index];
            let number = number_string.parse::<i64>().unwrap();
            let string = &encoded_value[colon_index + 1..colon_index + 1 + number as usize];
            return (string.into(), colon_index + 1 + number as usize);
        },
        'l' => {
            let mut list = Vec::new();
            let mut total_len = 0;
            encoded_value = encoded_value.strip_prefix('l').unwrap().strip_suffix('e').unwrap();
            loop {
                let (list_part, len) = decode_bencoded_value(encoded_value);
                total_len += len;
                list.push(list_part.clone());
                encoded_value = &encoded_value[len..];
                if encoded_value.is_empty() {break}
            }
            return(Value::Array(list), total_len);
                        
        },
        'i' => {
            let num_end = encoded_value.find('e').unwrap();
            let int_res = encoded_value[1..num_end].parse::<i64>();
            if let Ok(n) = int_res {
                return (n.into(), num_end + 1)
            }   
        },
        _ => {}
    }
    panic!("Unhandled encoded value: {}", encoded_value)
}

// Usage: your_bittorrent.sh decode "<encoded_value>"
fn main() {
    let args: Vec<String> = env::args().collect();
    let command = &args[1];

    if command == "decode" {
        let encoded_value = &args[2];
        let decoded_value = decode_bencoded_value(encoded_value).0;
        println!("{}", decoded_value);
    } else {
        println!("unknown command: {}", args[1])
    }
}
