//! What the core makes of a file the UI hands it: a photo from the system
//! picker, say.
//!
//! `kind` is read from the file's first bytes, not from its name: the name is
//! what someone typed, the magic is what the bytes are.

use std::io::Read;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// What a file is.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileInfo {
    pub name: String,
    pub bytes: i64,
    pub kind: String,
    pub sha256: String,
}

/// Reads a file and reports its size, its format and its SHA-256.
///
/// `Read` and not a path: the bridge hands over a descriptor the system already
/// opened, and a test hands over a buffer. The core never opens anything.
pub fn inspect<R: Read>(reader: &mut R, name: &str) -> std::io::Result<FileInfo> {
    let mut hasher = Sha256::new();
    let mut buf = [0u8; 64 << 10];
    let mut head = Vec::with_capacity(32);
    let mut bytes: i64 = 0;
    loop {
        let got = reader.read(&mut buf)?;
        if got == 0 {
            break;
        }
        if head.len() < 32 {
            let take = got.min(32 - head.len());
            head.extend_from_slice(&buf[..take]);
        }
        hasher.update(&buf[..got]);
        bytes += got as i64;
    }
    Ok(FileInfo {
        name: name.to_owned(),
        bytes,
        kind: kind_of(&head).to_owned(),
        sha256: format!("{:x}", hasher.finalize()),
    })
}

/// Names a format from its first bytes.
fn kind_of(head: &[u8]) -> &'static str {
    let starts_with = |what: &[u8]| head.len() >= what.len() && &head[..what.len()] == what;
    if starts_with(&[0xFF, 0xD8, 0xFF]) {
        "image/jpeg"
    } else if starts_with(b"\x89PNG\r\n\x1a\n") {
        "image/png"
    } else if head.len() >= 12 && &head[4..8] == b"ftyp" {
        // HEIF/HEIC, what HarmonyOS cameras save by default.
        "image/heif"
    } else if starts_with(b"GIF87a") || starts_with(b"GIF89a") {
        "image/gif"
    } else if starts_with(b"%PDF") {
        "application/pdf"
    } else if starts_with(b"PK\x03\x04") {
        "application/zip"
    } else {
        "application/octet-stream"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    fn info(bytes: &[u8]) -> FileInfo {
        inspect(&mut Cursor::new(bytes), "photo").unwrap()
    }

    #[test]
    fn names_formats_from_their_magic() {
        assert_eq!(info(&[0xFF, 0xD8, 0xFF, 0xE0, 0, 0]).kind, "image/jpeg");
        assert_eq!(info(b"\x89PNG\r\n\x1a\n0000").kind, "image/png");
        assert_eq!(info(b"\x00\x00\x00\x20ftypheic").kind, "image/heif");
        assert_eq!(info(b"GIF89a0000").kind, "image/gif");
        assert_eq!(info(b"%PDF-1.7").kind, "application/pdf");
        assert_eq!(info(b"PK\x03\x04rest").kind, "application/zip");
        assert_eq!(info(b"nothing to see").kind, "application/octet-stream");
    }

    #[test]
    fn hashes_the_whole_file() {
        // The published SHA-256 of "abc".
        let got = info(b"abc");
        assert_eq!(got.bytes, 3);
        assert_eq!(
            got.sha256,
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );

        // 200 KB of zeros spans three reads of the 64 KB buffer: a loop that
        // only hashed its first chunk would still pass the case above.
        // `head -c 200000 /dev/zero | shasum -a 256`.
        let long = vec![0u8; 200_000];
        let got = info(&long);
        assert_eq!(got.bytes, 200_000);
        assert_eq!(
            got.sha256,
            "4cbbd9be0cba685835755f827758705db5a413c5494c34262cd25946a73e7582"
        );
    }
}
