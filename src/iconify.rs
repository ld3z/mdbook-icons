use anyhow::{Context, Result, anyhow};
use regex::Regex;
use serde::Deserialize;
use serde::de::IgnoredAny;
use std::collections::{HashMap, HashSet};
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, LazyLock, Mutex, OnceLock};
use std::time::Duration;

pub(crate) const VERSION_FILE_NAME: &str = "iconify-json.version";

/// Version pinned in the crate's own `iconify-json.version`, used as the
/// default when no override is found at runtime.
const DEFAULT_ICONIFY_VERSION: &str = include_str!("../iconify-json.version");

const DOWNLOAD_LIMIT: u64 = 50 * 1024 * 1024;
const HTTP_TIMEOUT: Duration = Duration::from_secs(30);
const USER_AGENT: &str = concat!(
    env!("CARGO_PKG_NAME"),
    "/",
    env!("CARGO_PKG_VERSION"),
    " (+",
    env!("CARGO_PKG_REPOSITORY"),
    ")"
);

static TMP_FILE_SEQ: AtomicU64 = AtomicU64::new(0);
static SVG_ID_SEQ: AtomicU64 = AtomicU64::new(0);

static HTTP_AGENT: LazyLock<ureq::Agent> = LazyLock::new(|| {
    ureq::Agent::config_builder()
        .timeout_global(Some(HTTP_TIMEOUT))
        .user_agent(USER_AGENT)
        .build()
        .into()
});

static SVG_ID_DEF_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"\bid=(?:"([^"]+)"|'([^']+)')"#).unwrap());
static SVG_ID_TOKEN_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r#"(?P<attr>\b(?:id|(?:xlink:)?href)=)(?P<q>["'])(?P<hash>#?)(?P<id>[^"']+)(?P<q2>["'])|url\(\s*(?P<uq>["']?)#(?P<uid>[^)"'\s]+)(?P<uq2>["']?)\s*\)"#,
    )
    .unwrap()
});

pub(crate) fn http_agent() -> &'static ureq::Agent {
    &HTTP_AGENT
}

pub(crate) fn validate_version(version: &str) -> Result<&str> {
    let trimmed = version.trim();
    if trimmed.is_empty()
        || trimmed.len() > 64
        || trimmed.starts_with(['.', '-'])
        || trimmed.contains("..")
        || !trimmed
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_'))
    {
        return Err(anyhow!("invalid Iconify version: {version:?}"));
    }
    Ok(trimmed)
}

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
    fn resolve(root: Option<&Path>, configured_version: Option<&str>) -> Result<Self> {
        if let Ok(dir) = env::var("ICONIFY_JSON_DIR") {
            return Ok(IconSource::LocalDir(normalize_local_dir(Path::new(&dir))));
        }

        let raw_version = env::var("ICONIFY_JSON_VERSION")
            .ok()
            .or_else(|| configured_version.map(ToOwned::to_owned))
            .or_else(|| {
                let path = find_version_file_from(root)?;
                fs::read_to_string(path).ok()
            })
            .unwrap_or_else(|| DEFAULT_ICONIFY_VERSION.to_string());
        let version = validate_version(&raw_version)?.to_string();

        Ok(IconSource::Remote {
            cache_dir: cache_root().join(&version),
            version,
        })
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
                let map: HashMap<String, IgnoredAny> =
                    serde_json::from_slice(&bytes).context("parsing Iconify collections.json")?;
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
    let mut response = http_agent()
        .get(url)
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
    let seq = TMP_FILE_SEQ.fetch_add(1, Ordering::Relaxed);
    let tmp = parent.join(format!(
        ".{}.tmp.{}.{}",
        path.file_name().unwrap_or_default().to_string_lossy(),
        std::process::id(),
        seq
    ));
    fs::write(&tmp, bytes)?;
    if let Err(err) = fs::rename(&tmp, path) {
        let _ = fs::remove_file(&tmp);
        return Err(err.into());
    }
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

/// Finds an `iconify-json.version` file by walking up from `start_dir` (if
/// given) and falling back to walking up from the current working directory.
pub(crate) fn find_version_file_from(start_dir: Option<&Path>) -> Option<PathBuf> {
    if let Some(start) = start_dir {
        let resolved = if start.is_absolute() {
            start.to_path_buf()
        } else if let Ok(cwd) = env::current_dir() {
            cwd.join(start)
        } else {
            start.to_path_buf()
        };
        if let Some(found) = walk_up_for_version_file(resolved) {
            return Some(found);
        }
    }

    walk_up_for_version_file(env::current_dir().ok()?)
}

pub(crate) fn find_version_file() -> Option<PathBuf> {
    find_version_file_from(None)
}

fn walk_up_for_version_file(mut dir: PathBuf) -> Option<PathBuf> {
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

#[derive(Debug, Clone)]
enum CachedShortcode {
    UnknownPrefix,
    StaticSvg(String),
    DynamicIcon(Arc<ResolvedIcon>),
}

#[derive(Debug)]
pub struct IconStore {
    source: IconSource,
    prefixes: OnceLock<HashSet<String>>,
    cache: Mutex<HashMap<String, Arc<Collection>>>,
    rendered: Mutex<HashMap<String, CachedShortcode>>,
    aliases: HashMap<String, String>,
}

impl IconStore {
    #[cfg(test)]
    pub(crate) fn new() -> Self {
        Self::with_test_aliases(HashMap::new())
    }

    #[cfg(test)]
    pub(crate) fn with_test_aliases(aliases: HashMap<String, String>) -> Self {
        let fixtures_dir = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests")
            .join("fixtures")
            .join("iconify");
        Self::from_local_dir(&fixtures_dir, aliases)
    }

    #[cfg(test)]
    pub(crate) fn from_local_dir(dir: &Path, aliases: HashMap<String, String>) -> Self {
        Self {
            source: IconSource::LocalDir(normalize_local_dir(dir)),
            prefixes: OnceLock::new(),
            cache: Mutex::new(HashMap::new()),
            rendered: Mutex::new(HashMap::new()),
            aliases,
        }
    }

    pub fn from_config(
        root: Option<&Path>,
        configured_version: Option<&str>,
        aliases: HashMap<String, String>,
    ) -> Result<Self> {
        Ok(Self {
            source: IconSource::resolve(root, configured_version)?,
            prefixes: OnceLock::new(),
            cache: Mutex::new(HashMap::new()),
            rendered: Mutex::new(HashMap::new()),
            aliases,
        })
    }

    pub(crate) fn has_alias(&self, shortcode: &str) -> bool {
        self.aliases.contains_key(shortcode)
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
        if let Some(cached) = self.rendered.lock().unwrap().get(shortcode).cloned() {
            return match cached {
                CachedShortcode::UnknownPrefix => Ok(None),
                CachedShortcode::StaticSvg(svg) => Ok(Some(svg)),
                CachedShortcode::DynamicIcon(icon) => Ok(Some(icon.to_svg())),
            };
        }

        let resolved = self.resolve_user_alias(shortcode)?;
        let Some((prefix, name)) = self.split_shortcode(&resolved)? else {
            self.rendered
                .lock()
                .unwrap()
                .insert(shortcode.to_string(), CachedShortcode::UnknownPrefix);
            return Ok(None);
        };
        let icon = self.resolve_icon(&prefix, &name)?;
        if has_internal_svg_ids(&icon.body) {
            let icon = Arc::new(icon);
            let svg = icon.to_svg();
            self.rendered
                .lock()
                .unwrap()
                .insert(shortcode.to_string(), CachedShortcode::DynamicIcon(icon));
            Ok(Some(svg))
        } else {
            let svg = icon.to_svg();
            self.rendered.lock().unwrap().insert(
                shortcode.to_string(),
                CachedShortcode::StaticSvg(svg.clone()),
            );
            Ok(Some(svg))
        }
    }

    fn known_prefixes(&self) -> Result<&HashSet<String>> {
        if let Some(prefixes) = self.prefixes.get() {
            return Ok(prefixes);
        }
        let loaded = self.source.prefixes()?;
        Ok(self.prefixes.get_or_init(|| loaded))
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

    fn load_collection(&self, prefix: &str) -> Result<Arc<Collection>> {
        if let Some(collection) = self.cache.lock().unwrap().get(prefix).cloned() {
            return Ok(collection);
        }

        let bytes = self.source.collection_bytes(prefix)?;
        let collection: Arc<Collection> = Arc::new(
            serde_json::from_slice(&bytes)
                .with_context(|| format!("failed to parse icon collection: {prefix}"))?,
        );

        self.cache
            .lock()
            .unwrap()
            .insert(prefix.to_string(), Arc::clone(&collection));

        Ok(collection)
    }
}

#[derive(Debug, Clone, Deserialize)]
struct Collection {
    #[serde(default)]
    left: Option<f32>,
    #[serde(default)]
    top: Option<f32>,
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
    left: Option<f32>,
    #[serde(default)]
    top: Option<f32>,
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
    left: Option<f32>,
    #[serde(default)]
    top: Option<f32>,
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
    left: f32,
    top: f32,
    width: f32,
    height: f32,
    rotate: u8,
    h_flip: bool,
    v_flip: bool,
}

impl ResolvedIcon {
    fn to_svg(&self) -> String {
        let (view_box, out_width, out_height, transform) = icon_geometry(
            self.left,
            self.top,
            self.width,
            self.height,
            self.rotate,
            self.h_flip,
            self.v_flip,
        );
        let width_attr = format_em_width(out_width, out_height);
        let body = scope_svg_ids(&self.body);

        let mut svg = String::with_capacity(body.len() + 180);
        svg.push_str("<svg xmlns=\"http://www.w3.org/2000/svg\" class=\"mdbook-icon\" width=\"");
        svg.push_str(&width_attr);
        svg.push_str("\" height=\"1em\" viewBox=\"");
        svg.push_str(&view_box);
        svg.push_str(
            "\" aria-hidden=\"true\" focusable=\"false\" style=\"vertical-align: -0.125em; display: inline-block;\"",
        );

        if let Some(transform) = transform {
            svg.push_str("><g transform=\"");
            svg.push_str(&transform);
            svg.push_str("\">");
            svg.push_str(&body);
            svg.push_str("</g></svg>");
        } else {
            svg.push('>');
            svg.push_str(&body);
            svg.push_str("</svg>");
        }

        svg
    }
}

fn format_em_width(width: f32, height: f32) -> String {
    if height <= 0.0 || (width - height).abs() < f32::EPSILON {
        return "1em".to_string();
    }
    let ratio = ((width / height) * 10_000.0).round() / 10_000.0;
    format!("{ratio}em")
}

fn has_internal_svg_ids(body: &str) -> bool {
    body.contains("id=") && SVG_ID_DEF_RE.is_match(body)
}

fn scope_svg_ids(body: &str) -> String {
    if !body.contains("id=") {
        return body.to_string();
    }

    let defined_ids: HashSet<&str> = SVG_ID_DEF_RE
        .captures_iter(body)
        .filter_map(|caps| caps.get(1).or_else(|| caps.get(2)).map(|m| m.as_str()))
        .collect();

    if defined_ids.is_empty() {
        return body.to_string();
    }

    let scope = SVG_ID_SEQ.fetch_add(1, Ordering::Relaxed);
    SVG_ID_TOKEN_RE
        .replace_all(body, |caps: &regex::Captures| {
            if let Some(attr) = caps.name("attr") {
                let q = &caps["q"];
                let q2 = &caps["q2"];
                let hash = &caps["hash"];
                let id = &caps["id"];
                let attr_str = attr.as_str();
                let is_valid_id = attr_str == "id=" && hash.is_empty();
                let is_valid_href = attr_str != "id=" && hash == "#";
                if q == q2 && (is_valid_id || is_valid_href) && defined_ids.contains(id) {
                    return format!("{attr_str}{q}{hash}mdbi-{scope}-{id}{q2}");
                }
            } else if let Some(uid_match) = caps.name("uid") {
                let uq = &caps["uq"];
                let uq2 = &caps["uq2"];
                let uid = uid_match.as_str();
                if uq == uq2 && defined_ids.contains(uid) {
                    return format!("url({uq}#mdbi-{scope}-{uid}{uq2})");
                }
            }
            caps[0].to_string()
        })
        .into_owned()
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
            left: icon.left.or(collection.left).unwrap_or(0.0),
            top: icon.top.or(collection.top).unwrap_or(0.0),
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
    if let Some(left) = alias.left {
        resolved.left = left;
    }
    if let Some(top) = alias.top {
        resolved.top = top;
    }
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

fn icon_geometry(
    left: f32,
    top: f32,
    width: f32,
    height: f32,
    rotate: u8,
    mut h_flip: bool,
    mut v_flip: bool,
) -> (String, f32, f32, Option<String>) {
    let mut rotate = rotate % 4;
    if h_flip && v_flip {
        rotate = (rotate + 2) % 4;
        h_flip = false;
        v_flip = false;
    }

    if rotate == 0 && !h_flip && !v_flip {
        let view_box = format!("{left} {top} {width} {height}");
        return (view_box, width, height, None);
    }

    let (out_width, out_height) = if rotate % 2 == 1 {
        (height, width)
    } else {
        (width, height)
    };

    // SVG transforms apply right-to-left:
    // 1. Shift `[left, left+width] x [top, top+height]` into `[0, width] x [0, height]`
    // 2. Rotate into `[0, out_width] x [0, out_height]`
    // 3. Flip across `out_width` / `out_height`
    let mut transforms = Vec::with_capacity(3);

    if h_flip {
        transforms.push(format!("translate({out_width} 0) scale(-1 1)"));
    } else if v_flip {
        transforms.push(format!("translate(0 {out_height}) scale(1 -1)"));
    }

    match rotate {
        1 => transforms.push(format!("translate({height} 0) rotate(90)")),
        2 => transforms.push(format!("translate({width} {height}) rotate(180)")),
        3 => transforms.push(format!("translate(0 {width}) rotate(-90)")),
        _ => {}
    }

    if left != 0.0 || top != 0.0 {
        transforms.push(format!("translate({} {})", -left, -top));
    }

    let view_box = format!("0 0 {out_width} {out_height}");
    (view_box, out_width, out_height, Some(transforms.join(" ")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::thread;

    #[test]
    fn renders_user_alias_shortcode() {
        let mut aliases = HashMap::new();
        aliases.insert("star".to_string(), "twemoji-glowing-star".to_string());
        let store = IconStore::with_test_aliases(aliases);

        let rendered = store
            .render_shortcode("star")
            .expect("user alias should render");

        assert!(rendered.contains("<svg"), "alias should return svg");
        assert!(
            rendered.contains("class=\"mdbook-icon\""),
            "svg should include mdbook-icon class"
        );
        assert!(
            rendered.contains("aria-hidden=\"true\" focusable=\"false\""),
            "svg should have decorative accessibility attributes"
        );
        assert!(
            !rendered.contains("role=\"img\""),
            "decorative aria-hidden svg should not declare role=img"
        );
    }

    #[test]
    fn scopes_internal_svg_ids_uniquely_per_render() {
        let icon = ResolvedIcon {
            body: concat!(
                "<defs>",
                "<linearGradient id=\"a\"><stop offset=\"0\" stop-color=\"#fff\"/></linearGradient>",
                "<linearGradient id=\"b\" xlink:href=\"#a\"/>",
                "<clipPath id=\"clip\"><rect width=\"16\" height=\"16\"/></clipPath>",
                "</defs>",
                "<g clip-path=\"url(#clip)\"><use href=\"#b\" fill=\"url('#a')\"/></g>",
            )
            .to_string(),
            left: 0.0,
            top: 0.0,
            width: 16.0,
            height: 16.0,
            rotate: 0,
            h_flip: false,
            v_flip: false,
        };

        let first = icon.to_svg();
        let second = icon.to_svg();

        assert_ne!(first, second, "each render must get unique internal IDs");
        assert!(!first.contains("id=\"a\""), "raw id=a should be rewritten");
        assert!(
            !first.contains("url(#clip)"),
            "url(#clip) should be rewritten"
        );
        assert!(
            !first.contains("xlink:href=\"#a\""),
            "xlink:href=#a should be rewritten"
        );
        assert!(
            !first.contains("href=\"#b\""),
            "href=#b should be rewritten"
        );
    }

    #[test]
    fn handles_left_top_offsets_and_nonsquare_rotation() {
        let offset_icon = ResolvedIcon {
            body: "<path d=\"M0 0h10v10H0z\"/>".to_string(),
            left: -2.0,
            top: -4.0,
            width: 20.0,
            height: 24.0,
            rotate: 0,
            h_flip: false,
            v_flip: false,
        };
        let offset_svg = offset_icon.to_svg();
        assert!(
            offset_svg.contains("viewBox=\"-2 -4 20 24\""),
            "untransformed icon should preserve left/top in viewBox: {offset_svg}"
        );

        let rotated = ResolvedIcon {
            body: "<path d=\"M0 0h32v16H0z\"/>".to_string(),
            left: -2.0,
            top: 4.0,
            width: 32.0,
            height: 16.0,
            rotate: 1,
            h_flip: false,
            v_flip: false,
        };
        let rotated_svg = rotated.to_svg();
        assert!(
            rotated_svg.contains("viewBox=\"0 0 16 32\""),
            "90deg rotated 32x16 icon should swap viewBox to 16x32: {rotated_svg}"
        );
        assert!(
            rotated_svg.contains("width=\"0.5em\" height=\"1em\""),
            "rotated 32x16 icon should have 0.5em width: {rotated_svg}"
        );
        assert!(
            rotated_svg.contains("transform=\"translate(16 0) rotate(90) translate(2 -4)\""),
            "rotated icon with offset should normalize left/top then rotate: {rotated_svg}"
        );
    }

    #[test]
    fn memoizes_static_icons_while_keeping_gradient_ids_unique() {
        let store = IconStore::new();

        let home_a = store.render_shortcode("mdi-home").expect("mdi-home");
        let home_b = store.render_shortcode("mdi-home").expect("mdi-home cached");
        assert_eq!(home_a, home_b, "static icon renders should be identical");

        let grad_a = store
            .render_shortcode("twemoji-gradient-star")
            .expect("gradient icon first render");
        let grad_b = store
            .render_shortcode("twemoji-gradient-star")
            .expect("gradient icon second render");
        assert_ne!(
            grad_a, grad_b,
            "memoized gradient icon must still emit distinct scoped IDs per call"
        );
    }

    #[test]
    fn validates_iconify_versions() {
        assert_eq!(validate_version("2.2.556").unwrap(), "2.2.556");
        assert_eq!(
            validate_version("  v2.2.556-rc.1\n").unwrap(),
            "v2.2.556-rc.1"
        );

        for bad in [
            "",
            "../etc",
            "2.2.556/../../x",
            "2.2.556?foo=1",
            ".hidden",
            "-flag",
        ] {
            assert!(
                validate_version(bad).is_err(),
                "expected {bad:?} to be rejected"
            );
        }
    }

    #[test]
    fn concurrent_atomic_cache_writes_do_not_clobber_or_leak_tmp_files() {
        let test_dir = env::temp_dir().join(format!(
            "mdbook-icons-atomic-test-{}-{}",
            std::process::id(),
            TMP_FILE_SEQ.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&test_dir).expect("create temp dir");
        let target = test_dir.join("collections.json");
        let payload = br#"{"mdi":{},"twemoji":{}}"#;

        thread::scope(|s| {
            let mut handles = Vec::new();
            for _ in 0..8 {
                let target = target.clone();
                handles.push(s.spawn(move || write_cache_atomic(&target, payload)));
            }
            let successes = handles
                .into_iter()
                .filter_map(|h| h.join().expect("thread panicked").ok())
                .count();
            assert!(successes >= 1, "at least one concurrent write must succeed");
        });

        let written = fs::read(&target).expect("target file should exist");
        assert_eq!(written, payload);

        let remaining_entries: Vec<_> = fs::read_dir(&test_dir)
            .expect("read temp dir")
            .map(|e| {
                e.expect("dir entry")
                    .file_name()
                    .to_string_lossy()
                    .into_owned()
            })
            .collect();
        assert_eq!(
            remaining_entries,
            vec!["collections.json".to_string()],
            "no temporary files should remain"
        );

        let _ = fs::remove_dir_all(&test_dir);
    }

    #[test]
    fn finds_version_file_from_explicit_book_root() {
        let base = env::temp_dir().join(format!(
            "mdbook-icons-root-test-{}-{}",
            std::process::id(),
            TMP_FILE_SEQ.fetch_add(1, Ordering::Relaxed)
        ));
        let nested = base.join("sub").join("book");
        fs::create_dir_all(&nested).expect("create nested book root");
        let version_file = base.join(VERSION_FILE_NAME);
        fs::write(&version_file, "2.2.500\n").expect("write version file");

        let found = find_version_file_from(Some(&nested)).expect("should find version file");
        assert_eq!(found, version_file);

        let _ = fs::remove_dir_all(&base);
    }
}
