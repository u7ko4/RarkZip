use std::path::PathBuf;

use rarkzip_crypt::algorithm::Algorithm;
use rarkzip_crypt::file::{encrypt_file, FileEncryptOptions};
use rarkzip_crypt::progress::NoopReporter;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 5 {
        eprintln!("usage: encrypt_file <input> <password> <algorithm-code> <output-dir>");
        std::process::exit(2);
    }
    let input = PathBuf::from(&args[1]);
    let password = &args[2];
    let code: u8 = args[3].parse().expect("algorithm code");
    let output_dir = PathBuf::from(&args[4]);
    let algorithm = Algorithm::from_code(code).expect("algorithm code");
    match encrypt_file(
        &input,
        password,
        &FileEncryptOptions {
            algorithm,
            compression: Default::default(),
            output_dir: Some(output_dir),
        },
        &NoopReporter,
    ) {
        Ok(path) => println!("{}", path.display()),
        Err(error) => {
            eprintln!("{error}");
            std::process::exit(1);
        }
    }
}
