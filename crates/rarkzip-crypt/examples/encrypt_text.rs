use std::path::PathBuf;

use rarkzip_crypt::algorithm::Algorithm;
use rarkzip_crypt::text::{encrypt_text, TextEncoding, TextEncryptOptions};

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 6 {
        eprintln!("usage: encrypt_text <text> <password> <algorithm-code> <encoding> <output>");
        eprintln!("encoding: 0=base64, 1=base32, 2=numeric");
        std::process::exit(2);
    }
    let text = &args[1];
    let password = &args[2];
    let code: u8 = args[3].parse().expect("algorithm code");
    let encoding = match args[4].as_str() {
        "0" => TextEncoding::Base64,
        "1" => TextEncoding::Base32,
        "2" => TextEncoding::Base10,
        _ => {
            eprintln!("invalid encoding");
            std::process::exit(2);
        }
    };
    let algorithm = Algorithm::from_code(code).expect("algorithm code");
    match encrypt_text(
        text,
        password,
        &TextEncryptOptions {
            algorithm,
            encoding,
            compress: true,
            space_every: 0,
        },
    ) {
        Ok(encoded) => {
            std::fs::write(PathBuf::from(&args[5]), encoded.as_bytes()).expect("write output");
            println!("{encoded}");
        }
        Err(error) => {
            eprintln!("{error}");
            std::process::exit(1);
        }
    }
}
