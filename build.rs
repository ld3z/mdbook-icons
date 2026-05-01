use anyhow::{Context, Result};
use flate2::{Compression, write::GzEncoder};
use serde::Deserialize;
use std::collections::hash_map::DefaultHasher;
use std::env;
use std::fs;
use std::hash::{Hash, Hasher};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use tar::Archive;

#[derive(Debug, Deserialize)]
struct NpmMeta {
    dist: NpmDist,
}

#[derive(Debug, Deserialize)]
struct NpmDist {
    tarball: String,
}

#[derive(Debug)]
struct PackEntry {
    prefix: String,
    offset: usize,
    len: usize,
}

fn main() {
    if let Err(err) = run() {
        panic!("build script failed: {err:#}");
    }
}

fn run() -> Result<()> {
    println!("cargo:rerun-if-env-changed=ICONIFY_JSON_DIR");
    println!("cargo:rerun-if-env-changed=ICONIFY_JSON_TARBALL");

    let out_dir = PathBuf::from(env::var("OUT_DIR").context("OUT_DIR not set")?);
    let cache_dir = cache_root().join("v2");
    fs::create_dir_all(&cache_dir)?;

    let source = resolve_source()?;
    let cache_key = source.cache_key();
    let source_cache = cache_dir.join(&cache_key);
    let cached_pack = source_cache.join("iconify.pack");
    let cached_rs = source_cache.join("generated_iconify.rs");

    if cached_pack.is_file() && cached_rs.is_file() {
        fs::copy(&cached_pack, out_dir.join("iconify.pack"))?;
        fs::copy(&cached_rs, out_dir.join("generated_iconify.rs"))?;
        return Ok(());
    }

    let entries = source.load_entries()?;
    let pack_entries = build_pack(&entries, &out_dir.join("iconify.pack"))?;
    write_generated_rs(&out_dir, &pack_entries)?;

    fs::create_dir_all(&source_cache)?;
    fs::copy(out_dir.join("iconify.pack"), cached_pack)?;
    fs::copy(out_dir.join("generated_iconify.rs"), cached_rs)?;

    Ok(())
}

fn cache_root() -> PathBuf {
    if let Ok(target_dir) = env::var("CARGO_TARGET_DIR") {
        PathBuf::from(target_dir).join("iconify-cache")
    } else {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target/iconify-cache")
    }
}

fn normalize_local_dir(dir: &Path) -> PathBuf {
    if dir.join("json").is_dir() {
        dir.join("json")
    } else {
        dir.to_path_buf()
    }
}

#[derive(Debug)]
enum Source {
    Local(PathBuf),
    Remote { tarball_url: String },
}

impl Source {
    fn cache_key(&self) -> String {
        let mut hasher = DefaultHasher::new();
        match self {
            Source::Local(dir) => {
                "local".hash(&mut hasher);
                dir.to_string_lossy().hash(&mut hasher);
                if let Ok(entries) = fs::read_dir(dir) {
                    let mut files: Vec<_> = entries
                        .flatten()
                        .filter(|entry| {
                            entry.path().extension().and_then(|s| s.to_str()) == Some("json")
                        })
                        .collect();
                    files.sort_by_key(|entry| entry.file_name());
                    for entry in files {
                        let path = entry.path();
                        path.file_name().and_then(|s| s.to_str()).hash(&mut hasher);
                        if let Ok(meta) = entry.metadata() {
                            meta.len().hash(&mut hasher);
                            if let Ok(modified) = meta.modified() {
                                modified
                                    .duration_since(std::time::UNIX_EPOCH)
                                    .ok()
                                    .map(|d| d.as_secs())
                                    .hash(&mut hasher);
                            }
                        }
                    }
                }
            }
            Source::Remote { tarball_url } => {
                "remote".hash(&mut hasher);
                tarball_url.hash(&mut hasher);
            }
        }
        format!("{:016x}", hasher.finish())
    }

    fn load_entries(&self) -> Result<Vec<(String, Vec<u8>)>> {
        match self {
            Source::Local(dir) => extract_from_local_dir(dir),
            Source::Remote { tarball_url } => extract_from_tarball(tarball_url),
        }
    }
}

fn resolve_source() -> Result<Source> {
    if let Ok(dir) = env::var("ICONIFY_JSON_DIR") {
        return Ok(Source::Local(normalize_local_dir(&PathBuf::from(dir))));
    }

    let tarball = env::var("ICONIFY_JSON_TARBALL")
        .unwrap_or_else(|_| "https://registry.npmjs.org/@iconify/json/latest".to_string());

    let tarball_url = if tarball.ends_with(".tgz") {
        tarball
    } else {
        let meta: NpmMeta = ureq::get(&tarball)
            .call()
            .context("downloading Iconify package metadata")?
            .into_json()
            .context("parsing Iconify package metadata")?;
        meta.dist.tarball
    };

    Ok(Source::Remote { tarball_url })
}

fn extract_from_local_dir(dir: &Path) -> Result<Vec<(String, Vec<u8>)>> {
    let mut entries = Vec::new();

    for entry in fs::read_dir(dir).with_context(|| format!("reading {}", dir.display()))? {
        let entry = entry?;
        let path = entry.path();
        if path.extension().and_then(|s| s.to_str()) != Some("json") {
            continue;
        }
        let stem = path
            .file_stem()
            .and_then(|s| s.to_str())
            .context("invalid json file name")?;
        let bytes = fs::read(&path).with_context(|| format!("reading {}", path.display()))?;
        entries.push((stem.to_string(), bytes));
    }

    entries.sort_by(|a, b| a.0.cmp(&b.0));
    Ok(entries)
}

fn extract_from_tarball(tarball_url: &str) -> Result<Vec<(String, Vec<u8>)>> {
    let response = ureq::get(tarball_url)
        .call()
        .with_context(|| format!("downloading {tarball_url}"))?;
    let mut bytes = Vec::new();
    response
        .into_reader()
        .read_to_end(&mut bytes)
        .context("reading tarball response")?;

    let decoder = flate2::read::GzDecoder::new(bytes.as_slice());
    let mut archive = Archive::new(decoder);
    let mut entries = Vec::new();

    for entry in archive.entries().context("reading tar entries")? {
        let mut entry = entry.context("reading tar entry")?;
        let path = entry.path().context("tar entry path")?.to_path_buf();
        let path_str = path.to_string_lossy();
        if !path_str.starts_with("package/json/") || !path_str.ends_with(".json") {
            continue;
        }
        let stem = path
            .file_stem()
            .and_then(|s| s.to_str())
            .context("invalid tar entry name")?;
        let mut data = Vec::new();
        entry.read_to_end(&mut data)?;
        entries.push((stem.to_string(), data));
    }

    entries.sort_by(|a, b| a.0.cmp(&b.0));
    Ok(entries)
}

fn build_pack(entries: &[(String, Vec<u8>)], pack_path: &Path) -> Result<Vec<PackEntry>> {
    let mut pack = Vec::new();
    let mut index = Vec::new();

    for (prefix, bytes) in entries {
        let offset = pack.len();
        let mut encoder = GzEncoder::new(Vec::new(), Compression::best());
        encoder.write_all(bytes)?;
        let compressed = encoder.finish()?;
        let len = compressed.len();
        pack.extend_from_slice(&compressed);
        index.push(PackEntry {
            prefix: prefix.clone(),
            offset,
            len,
        });
    }

    fs::write(pack_path, pack)?;
    Ok(index)
}

fn write_generated_rs(out_dir: &Path, entries: &[PackEntry]) -> Result<()> {
    let mut rs = String::new();
    rs.push_str("use flate2::read::GzDecoder;\n");
    rs.push_str("use std::io::Read;\n\n");
    rs.push_str(
        "static PACK: &[u8] = include_bytes!(concat!(env!(\"OUT_DIR\"), \"/iconify.pack\"));\n\n",
    );
    rs.push_str("pub fn has_prefix(prefix: &str) -> bool {\n    match prefix {\n");

    for entry in entries {
        rs.push_str("        \"");
        rs.push_str(&entry.prefix);
        rs.push_str("\" => true,\n");
    }

    rs.push_str("        _ => false,\n    }\n}\n\n");
    rs.push_str("pub fn collection_bytes(prefix: &str) -> Result<Vec<u8>, String> {\n    let (offset, len) = match prefix {\n");

    for entry in entries {
        rs.push_str("        \"");
        rs.push_str(&entry.prefix);
        rs.push_str("\" => (");
        rs.push_str(&entry.offset.to_string());
        rs.push_str(", ");
        rs.push_str(&entry.len.to_string());
        rs.push_str("),\n");
    }

    rs.push_str("        _ => return Err(format!(\"unknown icon collection prefix: {prefix}\")),\n    };\n    let bytes = &PACK[offset..offset + len];\n    let mut decoder = GzDecoder::new(bytes);\n    let mut out = Vec::new();\n    decoder.read_to_end(&mut out).map_err(|e| e.to_string())?;\n    Ok(out)\n}\n");

    fs::write(out_dir.join("generated_iconify.rs"), rs)?;
    Ok(())
}
