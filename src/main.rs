mod iconify;

use anyhow::{Context, Result, anyhow};
use iconify::IconStore;
use once_cell::sync::Lazy;
use regex::Regex;
use serde_json::Value;
use std::{env, io};

static SHORTCODE_RE: Lazy<Regex> = Lazy::new(|| Regex::new(r":([A-Za-z0-9_-]+):").unwrap());

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

        out.push_str(&replace_shortcodes(line, store));
        out.push('\n');
    }

    out
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

fn main() -> Result<()> {
    if let Some(arg1) = env::args().nth(1) {
        if arg1 == "supports" {
            let renderer = env::args().nth(2).unwrap_or_else(|| String::from("html"));
            std::process::exit(if renderer == "html" { 0 } else { 1 });
        }
    }

    let stdin = io::stdin();
    let mut input: Value =
        serde_json::from_reader(stdin).context("failed to parse mdbook preprocessing input")?;

    let book = input
        .as_array_mut()
        .and_then(|items| items.get_mut(1))
        .ok_or_else(|| anyhow!("mdBook input must be a tuple of (context, book)"))?;

    let store = IconStore::new();
    replace_in_book(book, &store).context("failed to process book contents")?;

    serde_json::to_writer(io::stdout(), book).context("failed to write processed book")?;
    Ok(())
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
}
