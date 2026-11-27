# rarkzip-crypt

File, text and JPEG-steganography encryption engine used by the RarkZip
"文件与文本加密" tool window.

The engine is compatible with the legacy `.enc` container and text-ciphertext
formats used by earlier RarkZip encryption tools. It is independent of the GUI:
all entry points are plain Rust functions and can also be used from tests or a
future CLI.

## Features

- File/folder encryption into `.enc` containers (current v4 format; reads v2 and
  v3 for compatibility).
- Text encryption (current v3 format; reads v2 for compatibility) with LZMA
  compression and Base64, Base32 or numeric output.
- JPEG steganography using the F5 algorithm.
- Algorithms: AES-256, RC6-256, Serpent-256, Twofish-256, GOST28147-256,
  Blowfish-448, Threefish-1024, SHACAL-2-512 and the nested C4-2048 composite.
- Streaming CTR for files, EAX for text, keyed BLAKE3/BLAKE2b authentication
  and Argon2id + HKDF key derivation.

## Safety properties

- File MACs are verified before the ZIP payload is extracted into a temporary
  directory; the output is published only after successful verification.
- Argon2 cost parameters read from a file header are capped before derivation.
- Passwords, keys and intermediate KDF output use `zeroize` where practical.
- Archive entry names are validated before writing, and extraction never uses
  attacker-controlled absolute paths.

## Example

```rust
use rarkzip_crypt::algorithm::Algorithm;
use rarkzip_crypt::file::{encrypt_file, FileEncryptOptions};
use rarkzip_crypt::progress::NoopReporter;

let output = encrypt_file(
    std::path::Path::new("input.txt"),
    "password",
    &FileEncryptOptions {
        algorithm: Algorithm::Aes256,
        ..Default::default()
    },
    &NoopReporter,
)?;
println!("{}", output.display());
```

## Tests

```bash
cargo test -p rarkzip-crypt
```

The test suite includes known-answer vectors, legacy containers and
cross-implementation samples for every algorithm, text encoding and
steganography round trips.
