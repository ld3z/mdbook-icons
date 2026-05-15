use anyhow::{Context, Result, anyhow};
use serde::Deserialize;
use std::collections::{HashMap, HashSet};
use std::sync::Mutex;

mod generated {
    include!(concat!(env!("OUT_DIR"), "/generated_iconify.rs"));
}

#[derive(Debug)]
pub struct IconStore {
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
            cache: Mutex::new(HashMap::new()),
            aliases,
        }
    }

    pub fn render_shortcode(&self, shortcode: &str) -> Option<String> {
        self.render_shortcode_result(shortcode).ok().flatten()
    }

    pub(crate) fn render_shortcode_result(&self, shortcode: &str) -> Result<Option<String>> {
        let resolved = self.resolve_user_alias(shortcode)?;
        let Some((prefix, name)) = self.split_shortcode(&resolved) else {
            return Ok(None);
        };
        let icon = self.resolve_icon(&prefix, &name)?;
        Ok(Some(icon.to_svg()))
    }

    fn split_shortcode(&self, shortcode: &str) -> Option<(String, String)> {
        let mut best: Option<(String, String)> = None;

        for i in shortcode.match_indices('-').map(|(i, _)| i) {
            let prefix = &shortcode[..i];
            let name = &shortcode[i + 1..];
            if name.is_empty() {
                continue;
            }
            if generated::has_prefix(prefix) {
                best = Some((prefix.to_string(), name.to_string()));
            }
        }

        best
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

        let bytes = generated::collection_bytes(prefix).map_err(anyhow::Error::msg)?;
        let text = std::str::from_utf8(&bytes).context("embedded icon collection is not utf-8")?;
        let collection: Collection = serde_json::from_str(text)
            .with_context(|| format!("failed to parse embedded icon collection: {prefix}"))?;

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
