mod iconify;

use anyhow::{Context, Result, anyhow};
use clap::{Parser, Subcommand};
use iconify::IconStore;
use once_cell::sync::Lazy;
use regex::Regex;
use serde::Deserialize;
use serde_json::Value;
use std::collections::HashMap;
use std::fs;
use std::io;
use std::path::PathBuf;

static SHORTCODE_RE: Lazy<Regex> = Lazy::new(|| Regex::new(r":([A-Za-z0-9_-]+):").unwrap());
static TABLE_SHORTCODE_ONLY_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"^:([A-Za-z0-9_-]+):$").unwrap());

fn normalize_shortcode(value: &str) -> String {
    value.trim().trim_matches(':').to_string()
}

fn load_aliases(context: &Value) -> HashMap<String, String> {
    let mut aliases_map = HashMap::new();
    let aliases = context
        .get("config")
        .and_then(|config| config.get("preprocessor"))
        .and_then(|preprocessor| preprocessor.get("icons"))
        .and_then(|icons| icons.get("aliases"))
        .and_then(Value::as_object);

    if let Some(aliases) = aliases {
        for (key, value) in aliases {
            if let Some(value) = value.as_str() {
                let alias = normalize_shortcode(key);
                let target = normalize_shortcode(value);
                if !alias.is_empty() && !target.is_empty() {
                    aliases_map.insert(alias, target);
                }
            }
        }
    }

    aliases_map
}

fn replace_in_book(book: &mut Value, store: &IconStore) -> Result<()> {
    let items = if let Some(items) = book.get_mut("sections").and_then(Value::as_array_mut) {
        items
    } else if let Some(items) = book.get_mut("items").and_then(Value::as_array_mut) {
        items
    } else {
        return Err(anyhow!("mdBook input missing top-level book items array"));
    };

    for item in items {
        replace_in_book_item(item, store)?;
    }

    Ok(())
}

fn replace_in_book_item(item: &mut Value, store: &IconStore) -> Result<()> {
    let Some(obj) = item.as_object_mut() else {
        return Ok(());
    };

    if let Some(chapter) = obj.get_mut("Chapter") {
        let chapter = chapter
            .as_object_mut()
            .ok_or_else(|| anyhow!("chapter item is not an object"))?;

        if let Some(content) = chapter.get_mut("content") {
            let content_str = content
                .as_str()
                .ok_or_else(|| anyhow!("chapter content is not a string"))?;
            *content = Value::String(transform_markdown(content_str, store));
        }

        if let Some(sub_items) = chapter.get_mut("sub_items").and_then(Value::as_array_mut) {
            for sub_item in sub_items {
                replace_in_book_item(sub_item, store)?;
            }
        }
    }

    Ok(())
}

fn transform_markdown(input: &str, store: &IconStore) -> String {
    let mut out = String::new();
    let mut in_fence = false;

    for line in input.lines() {
        let trimmed = line.trim_start();
        if trimmed.starts_with("```") || trimmed.starts_with("~~~") {
            in_fence = !in_fence;
            out.push_str(line);
            out.push('\n');
            continue;
        }

        if in_fence {
            out.push_str(line);
            out.push('\n');
            continue;
        }

        if looks_like_table_row(line) {
            out.push_str(&replace_shortcodes_in_table_row(line, store));
        } else {
            out.push_str(&replace_shortcodes(line, store));
        }
        out.push('\n');
    }

    out
}

fn looks_like_table_row(line: &str) -> bool {
    line.trim_start().starts_with('|') && line.contains('|')
}

fn replace_shortcodes_in_table_row(line: &str, store: &IconStore) -> String {
    line.split('|')
        .map(|cell| replace_shortcodes_in_table_cell(cell, store))
        .collect::<Vec<_>>()
        .join("|")
}

fn replace_shortcodes_in_table_cell(cell: &str, store: &IconStore) -> String {
    let trimmed = cell.trim();
    if let Some(shortcode) = TABLE_SHORTCODE_ONLY_RE
        .captures(trimmed)
        .and_then(|caps| caps.get(1).map(|m| m.as_str()))
    {
        if let Some(svg) = store.render_shortcode(shortcode) {
            return format!(
                "<span style=\"display:inline-flex; width: 100%; justify-content: center;\">{svg}</span>"
            );
        }
    }

    replace_shortcodes(cell, store)
}

fn replace_shortcodes(line: &str, store: &IconStore) -> String {
    let mut result = String::new();
    let mut rest = line;
    let mut in_inline_code = false;

    while let Some(pos) = rest.find('`') {
        let (before, after) = rest.split_at(pos);
        if !in_inline_code {
            result.push_str(&replace_shortcodes_in_text(before, store));
        } else {
            result.push_str(before);
        }
        result.push('`');
        in_inline_code = !in_inline_code;
        rest = &after[1..];
    }

    if !in_inline_code {
        result.push_str(&replace_shortcodes_in_text(rest, store));
    } else {
        result.push_str(rest);
    }

    result
}

fn replace_shortcodes_in_text(text: &str, store: &IconStore) -> String {
    SHORTCODE_RE
        .replace_all(text, |caps: &regex::Captures| {
            let shortcode = caps.get(1).unwrap().as_str();
            store
                .render_shortcode(shortcode)
                .unwrap_or_else(|| caps.get(0).unwrap().as_str().to_string())
        })
        .into_owned()
}

#[derive(Parser)]
#[command(
    name = "mdbook-icons",
    about = "Replace Iconify shortcodes with inline SVG in mdBook content.",
    version
)]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand)]
enum Command {
    /// Check whether the renderer is supported by this preprocessor.
    Supports {
        /// The renderer name passed by mdBook.
        renderer: String,
    },
    /// Update the pinned Iconify pack version used by this project.
    Update {
        /// Use a specific Iconify JSON version instead of the latest release.
        #[arg(long)]
        version: Option<String>,
        /// Check whether the pinned Iconify JSON version is already up to date.
        #[arg(long)]
        check: bool,
    },
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Some(Command::Supports { renderer }) => {
            std::process::exit(if renderer == "html" { 0 } else { 1 });
        }
        Some(Command::Update { version, check }) => {
            let target_version = match version {
                Some(version) => version,
                None if check => fetch_latest_iconify_version()?,
                None => fetch_latest_iconify_version()?,
            };

            if check {
                let path = find_iconify_version_file();
                let current_version = read_iconify_version_file(&path)?;
                if current_version == target_version {
                    println!("{} is up to date ({})", path.display(), current_version);
                    return Ok(());
                }

                println!(
                    "{} is out of date: current {}, latest {}",
                    path.display(),
                    current_version,
                    target_version
                );
                std::process::exit(1);
            }

            let path = write_iconify_version_file(&target_version)?;
            println!("Updated {} to {}", path.display(), target_version);
            return Ok(());
        }
        None => {}
    }

    let stdin = io::stdin();
    let mut input: Value =
        serde_json::from_reader(stdin).context("failed to parse mdbook preprocessing input")?;

    let context = input
        .as_array()
        .and_then(|items| items.get(0))
        .cloned()
        .ok_or_else(|| anyhow!("mdBook input must be a tuple of (context, book)"))?;

    let book = input
        .as_array_mut()
        .and_then(|items| items.get_mut(1))
        .ok_or_else(|| anyhow!("mdBook input must be a tuple of (context, book)"))?;

    let aliases = load_aliases(&context);
    let store = IconStore::with_aliases(aliases);
    replace_in_book(book, &store).context("failed to process book contents")?;

    serde_json::to_writer(io::stdout(), book).context("failed to write processed book")?;
    Ok(())
}

#[derive(Debug, Deserialize)]
struct NpmLatestMeta {
    version: String,
}

fn fetch_latest_iconify_version() -> Result<String> {
    let url = "https://registry.npmjs.org/@iconify/json/latest";
    let mut response = ureq::get(url)
        .call()
        .with_context(|| format!("downloading Iconify package metadata from {url}"))?;
    let meta: NpmLatestMeta = response
        .body_mut()
        .read_json()
        .context("parsing latest Iconify package metadata")?;
    Ok(meta.version)
}

fn write_iconify_version_file(version: &str) -> Result<PathBuf> {
    let path = find_iconify_version_file();
    fs::write(&path, format!("{version}\n"))
        .with_context(|| format!("writing {}", path.display()))?;
    Ok(path)
}

fn read_iconify_version_file(path: &PathBuf) -> Result<String> {
    let version =
        fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
    Ok(version.trim().to_string())
}

fn find_iconify_version_file() -> PathBuf {
    let mut dir = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    loop {
        let candidate = dir.join("iconify-json.version");
        if candidate.is_file() {
            return candidate;
        }
        if !dir.pop() {
            break;
        }
    }

    PathBuf::from("iconify-json.version")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn replaces_shortcodes_but_skips_inline_code_and_fences() {
        let store = IconStore::new();
        let input = "Hello :mdi-home: and `:mdi-home:`\n\n```md\n:mdi-home:\n```\n";
        let output = transform_markdown(input, &store);

        assert!(output.contains("<svg"), "expected inline svg replacement");
        assert!(
            output.contains("`:mdi-home:`"),
            "inline code should be untouched"
        );
        assert!(
            output.contains("```md\n:mdi-home:\n```"),
            "fenced code should be untouched"
        );
    }

    #[test]
    fn resolves_aliases_to_the_same_icon() {
        let store = IconStore::new();
        let alias = store
            .render_shortcode("mdi-1password")
            .expect("alias should render");
        let parent = store
            .render_shortcode("mdi-onepassword")
            .expect("parent should render");

        assert_eq!(alias, parent);
    }

    #[test]
    fn renders_twemoji_glowing_star() {
        let store = IconStore::new();
        let rendered = store
            .render_shortcode_result("twemoji-glowing-star")
            .expect("twemoji shortcode should resolve");
        assert!(rendered.is_some(), "twemoji icon should render");
    }

    #[test]
    fn centers_icon_only_table_cells() {
        let store = IconStore::new();
        let input = "| Status |\n| --- |\n| :twemoji-check-mark-button: |\n";
        let output = transform_markdown(input, &store);

        assert!(
            output.contains("display:inline-flex; width: 100%; justify-content: center;"),
            "icon-only table cells should be centered"
        );
    }
}
