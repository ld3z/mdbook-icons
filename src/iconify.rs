use anyhow::{Context, Result, anyhow};
use once_cell::sync::OnceCell;
use serde::Deserialize;
use std::collections::{HashMap, HashSet};
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

pub(crate) const VERSION_FILE_NAME: &str = "iconify-json.version";

/// Version pinned in the crate's own `iconify-json.version`, used as the
/// default when no override is found at runtime.
const DEFAULT_ICONIFY_VERSION: &str = include_str!("../iconify-json.version");

const DOWNLOAD_LIMIT: u64 = 50 * 1024 * 1024;

/// Where icon collections come from at runtime.
///
/// Resolution order:
/// 1. `ICONIFY_JSON_DIR` — a local checkout of `@iconify/json` (offline use).
/// 2. A pinned remote version, resolved from `ICONIFY_JSON_VERSION`, then an
///    `iconify-json.version` file found walking up from the current directory,
///    then the version compiled into this binary. Downloads are cached on disk
///    per version, so each collection is fetched at most once per machine.
#[derive(Debug)]
enum IconSource {
    LocalDir(PathBuf),
    Remote { version: String, cache_dir: PathBuf },
}

impl IconSource {
    fn from_env() -> Self {
        if let Ok(dir) = env::var("ICONIFY_JSON_DIR") {
            return IconSource::LocalDir(normalize_local_dir(Path::new(&dir)));
        }

        let version = env::var("ICONIFY_JSON_VERSION")
            .ok()
            .or_else(|| {
                let path = find_version_file()?;
                fs::read_to_string(path).ok()
            })
            .unwrap_or_else(|| DEFAULT_ICONIFY_VERSION.to_string())
            .trim()
            .to_string();

        IconSource::Remote {
            cache_dir: cache_root().join(&version),
            version,
        }
    }

    fn prefixes(&self) -> Result<HashSet<String>> {
        match self {
            IconSource::LocalDir(dir) => {
                let mut prefixes = HashSet::new();
                for entry in
                    fs::read_dir(dir).with_context(|| format!("reading {}", dir.display()))?
                {
                    let path = entry?.path();
                    if path.extension().and_then(|s| s.to_str()) == Some("json")
                        && let Some(stem) = path.file_stem().and_then(|s| s.to_str())
                    {
                        prefixes.insert(stem.to_string());
                    }
                }
                Ok(prefixes)
            }
            IconSource::Remote { .. } => {
                let bytes = self.fetch("collections.json", "collections.json")?;
                let map: HashMap<String, serde_json::Value> = serde_json::from_slice(&bytes)
                    .context("parsing Iconify collections.json")?;
                Ok(map.into_keys().collect())
            }
        }
    }

    fn collection_bytes(&self, prefix: &str) -> Result<Vec<u8>> {
        match self {
            IconSource::LocalDir(dir) => {
                let path = dir.join(format!("{prefix}.json"));
                fs::read(&path).with_context(|| format!("reading {}", path.display()))
            }
            IconSource::Remote { .. } => {
                self.fetch(&format!("{prefix}.json"), &format!("json/{prefix}.json"))
            }
        }
    }

    /// Returns the cached copy of a remote file, downloading it first if needed.
    fn fetch(&self, cache_name: &str, url_path: &str) -> Result<Vec<u8>> {
        let IconSource::Remote { version, cache_dir } = self else {
            unreachable!("fetch is only used for the remote source");
        };

        let cached = cache_dir.join(cache_name);
        if let Ok(bytes) = fs::read(&cached) {
            return Ok(bytes);
        }

        let url =
            format!("https://raw.githubusercontent.com/iconify/icon-sets/{version}/{url_path}");
        let bytes = download(&url).with_context(|| {
            format!(
                "fetching Iconify data (version {version}); \
                 set ICONIFY_JSON_DIR to a local @iconify/json checkout for offline builds"
            )
        })?;

        // Caching is best-effort: a read-only cache dir shouldn't fail the build.
        let _ = write_cache_atomic(&cached, &bytes);
        Ok(bytes)
    }
}

fn download(url: &str) -> Result<Vec<u8>> {
    let mut response = ureq::get(url)
        .call()
        .with_context(|| format!("downloading {url}"))?;
    response
        .body_mut()
        .with_config()
        .limit(DOWNLOAD_LIMIT)
        .read_to_vec()
        .with_context(|| format!("reading response from {url}"))
}

fn write_cache_atomic(path: &Path, bytes: &[u8]) -> Result<()> {
    let parent = path.parent().context("cache path has no parent")?;
    fs::create_dir_all(parent)?;
    let tmp = parent.join(format!(
        ".{}.tmp{}",
        path.file_name().unwrap_or_default().to_string_lossy(),
        std::process::id()
    ));
    fs::write(&tmp, bytes)?;
    fs::rename(&tmp, path)?;
    Ok(())
}

fn cache_root() -> PathBuf {
    if let Ok(dir) = env::var("MDBOOK_ICONS_CACHE_DIR") {
        return PathBuf::from(dir);
    }
    dirs::cache_dir()
        .unwrap_or_else(env::temp_dir)
        .join("mdbook-icons")
        .join("iconify")
}

fn normalize_local_dir(dir: &Path) -> PathBuf {
    if dir.join("json").is_dir() {
        dir.join("json")
    } else {
        dir.to_path_buf()
    }
}

/// Finds an `iconify-json.version` file by walking up from the current
/// directory, so books can pin their own Iconify version.
pub(crate) fn find_version_file() -> Option<PathBuf> {
    let mut dir = env::current_dir().ok()?;
    loop {
        let candidate = dir.join(VERSION_FILE_NAME);
        if candidate.is_file() {
            return Some(candidate);
        }
        if !dir.pop() {
            return None;
        }
    }
}

#[derive(Debug)]
pub struct IconStore {
    source: IconSource,
    prefixes: OnceCell<HashSet<String>>,
    cache: Mutex<HashMap<String, Collection>>,
    aliases: HashMap<String, String>,
}

impl IconStore {
    #[cfg(test)]
    pub fn new() -> Self {
        Self::with_aliases(HashMap::new())
    }

    pub fn with_aliases(aliases: HashMap<String, String>) -> Self {
        Self {
            source: IconSource::from_env(),
            prefixes: OnceCell::new(),
            cache: Mutex::new(HashMap::new()),
            aliases,
        }
    }

    pub fn render_shortcode(&self, shortcode: &str) -> Option<String> {
        match self.render_shortcode_result(shortcode) {
            Ok(rendered) => rendered,
            Err(err) => {
                eprintln!("mdbook-icons: warning: failed to render :{shortcode}:: {err:#}");
                None
            }
        }
    }

    pub(crate) fn render_shortcode_result(&self, shortcode: &str) -> Result<Option<String>> {
        let resolved = self.resolve_user_alias(shortcode)?;
        let Some((prefix, name)) = self.split_shortcode(&resolved)? else {
            return Ok(None);
        };
        let icon = self.resolve_icon(&prefix, &name)?;
        Ok(Some(icon.to_svg()))
    }

    fn known_prefixes(&self) -> Result<&HashSet<String>> {
        self.prefixes.get_or_try_init(|| self.source.prefixes())
    }

    fn split_shortcode(&self, shortcode: &str) -> Result<Option<(String, String)>> {
        let prefixes = self.known_prefixes()?;
        let mut best: Option<(String, String)> = None;

        for i in shortcode.match_indices('-').map(|(i, _)| i) {
            let prefix = &shortcode[..i];
            let name = &shortcode[i + 1..];
            if name.is_empty() {
                continue;
            }
            if prefixes.contains(prefix) {
                best = Some((prefix.to_string(), name.to_string()));
            }
        }

        Ok(best)
    }

    fn resolve_user_alias(&self, shortcode: &str) -> Result<String> {
        let mut current = shortcode.to_string();
        let mut visited = HashSet::new();

        while let Some(next) = self.aliases.get(&current) {
            if !visited.insert(current.clone()) {
                return Err(anyhow!("user alias cycle detected: {shortcode}"));
            }
            current = next.clone();
        }

        Ok(current)
    }

    fn resolve_icon(&self, prefix: &str, name: &str) -> Result<ResolvedIcon> {
        let collection = self.load_collection(prefix)?;
        let mut visited = HashSet::new();
        resolve_from_collection(&collection, name, &mut visited)
            .with_context(|| format!("failed to resolve icon {prefix}-{name}"))
    }

    fn load_collection(&self, prefix: &str) -> Result<Collection> {
        if let Some(collection) = self.cache.lock().unwrap().get(prefix).cloned() {
            return Ok(collection);
        }

        let bytes = self.source.collection_bytes(prefix)?;
        let collection: Collection = serde_json::from_slice(&bytes)
            .with_context(|| format!("failed to parse icon collection: {prefix}"))?;

        self.cache
            .lock()
            .unwrap()
            .insert(prefix.to_string(), collection.clone());

        Ok(collection)
    }
}

#[derive(Debug, Clone, Deserialize)]
struct Collection {
    #[serde(default)]
    width: Option<f32>,
    #[serde(default)]
    height: Option<f32>,
    #[serde(default)]
    icons: HashMap<String, IconEntry>,
    #[serde(default)]
    aliases: HashMap<String, AliasEntry>,
}

#[derive(Debug, Clone, Deserialize)]
struct IconEntry {
    body: String,
    #[serde(default)]
    width: Option<f32>,
    #[serde(default)]
    height: Option<f32>,
    #[serde(default, rename = "hFlip")]
    h_flip: bool,
    #[serde(default, rename = "vFlip")]
    v_flip: bool,
    #[serde(default)]
    rotate: u8,
}

#[derive(Debug, Clone, Deserialize)]
struct AliasEntry {
    parent: String,
    #[serde(default)]
    width: Option<f32>,
    #[serde(default)]
    height: Option<f32>,
    #[serde(default, rename = "hFlip")]
    h_flip: bool,
    #[serde(default, rename = "vFlip")]
    v_flip: bool,
    #[serde(default)]
    rotate: u8,
}

#[derive(Debug, Clone)]
struct ResolvedIcon {
    body: String,
    width: f32,
    height: f32,
    rotate: u8,
    h_flip: bool,
    v_flip: bool,
}

impl ResolvedIcon {
    fn to_svg(&self) -> String {
        let mut svg = String::new();
        let view_box = format!("0 0 {} {}", self.width, self.height);
        let transform = icon_transform(
            self.width,
            self.height,
            self.rotate,
            self.h_flip,
            self.v_flip,
        );

        svg.push_str(
            "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"1em\" height=\"1em\" viewBox=\"",
        );
        svg.push_str(&view_box);
        svg.push_str("\" role=\"img\" aria-hidden=\"true\" style=\"vertical-align: -0.125em; display: inline-block;\"");
        if transform.is_none() {
            svg.push_str(">");
            svg.push_str(&self.body);
            svg.push_str("</svg>");
            return svg;
        }

        svg.push_str("><g transform=\"");
        svg.push_str(transform.as_ref().unwrap());
        svg.push_str("\">");
        svg.push_str(&self.body);
        svg.push_str("</g></svg>");
        svg
    }
}

fn resolve_from_collection(
    collection: &Collection,
    name: &str,
    visited: &mut HashSet<String>,
) -> Result<ResolvedIcon> {
    if !visited.insert(name.to_string()) {
        return Err(anyhow!("icon alias cycle detected: {name}"));
    }

    if let Some(icon) = collection.icons.get(name) {
        return Ok(ResolvedIcon {
            body: icon.body.clone(),
            width: icon.width.or(collection.width).unwrap_or(16.0),
            height: icon.height.or(collection.height).unwrap_or(16.0),
            rotate: icon.rotate % 4,
            h_flip: icon.h_flip,
            v_flip: icon.v_flip,
        });
    }

    let alias = collection
        .aliases
        .get(name)
        .with_context(|| format!("missing icon or alias: {name}"))?;

    let mut resolved = resolve_from_collection(collection, &alias.parent, visited)?;
    if let Some(width) = alias.width {
        resolved.width = width;
    }
    if let Some(height) = alias.height {
        resolved.height = height;
    }
    resolved.rotate = (resolved.rotate + alias.rotate) % 4;
    resolved.h_flip ^= alias.h_flip;
    resolved.v_flip ^= alias.v_flip;
    Ok(resolved)
}

fn icon_transform(
    width: f32,
    height: f32,
    rotate: u8,
    h_flip: bool,
    v_flip: bool,
) -> Option<String> {
    if rotate == 0 && !h_flip && !v_flip {
        return None;
    }

    let mut transforms = Vec::new();
    let cx = width / 2.0;
    let cy = height / 2.0;

    if h_flip || v_flip {
        let sx = if h_flip { -1.0 } else { 1.0 };
        let sy = if v_flip { -1.0 } else { 1.0 };
        transforms.push(format!(
            "translate({cx} {cy}) scale({sx} {sy}) translate(-{cx} -{cy})"
        ));
    }

    if rotate != 0 {
        let degrees = 90 * (rotate % 4) as i32;
        transforms.push(format!("rotate({degrees} {cx} {cy})"));
    }

    Some(transforms.join(" "))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_user_alias_shortcode() {
        let mut aliases = HashMap::new();
        aliases.insert("star".to_string(), "twemoji-glowing-star".to_string());
        let store = IconStore::with_aliases(aliases);

        let rendered = store
            .render_shortcode("star")
            .expect("user alias should render");

        assert!(rendered.contains("<svg"), "alias should return svg");
    }
}
