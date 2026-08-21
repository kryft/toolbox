use std::fs;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::Serialize;
use sha2::{Digest, Sha256};

const SIDECAR_EXT: &str = "json";

pub fn data_dir() -> PathBuf {
    std::env::var("TOOLBOX_DATA")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("data"))
}

#[derive(Serialize)]
struct DocMeta {
    url: String,
    content_type: String,
    bytes: u64,
    fetched_at: u64, // unix epoch
}

fn sha256_hex(s: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(s.as_bytes());
    let hash = hasher.finalize();
    hash.iter().map(|b| format!("{:02x}", b)).collect()
}

/// Store `body` under the sha256 of `url`; returns the id (hex string).
pub fn save(url: &str, content_type: &str, body: &[u8]) -> std::io::Result<String> {
    let dir = data_dir();
    fs::create_dir_all(&dir)?;

    let id = sha256_hex(url);
    fs::write(dir.join(&id), body)?;

    let meta = DocMeta {
        url: url.to_string(),
        content_type: content_type.to_string(),
        bytes: body.len() as u64,
        fetched_at: SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs(),
    };
    let meta_path = dir.join(format!("{}.{}", id, SIDECAR_EXT));
    fs::write(&meta_path, serde_json::to_string_pretty(&meta)?)?;

    Ok(id)
}
