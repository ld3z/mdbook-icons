mod iconify;

use anyhow::{Context, Result, anyhow};
use clap::{CommandFactory, Parser, Subcommand};
use iconify::IconStore;
use pulldown_cmark::{Event, Options, Parser as MarkdownParser, Tag, TagEnd};
use regex::Regex;
use serde::Deserialize;
use serde_json::Value;
use std::collections::HashMap;
use std::fs;
use std::io::{self, IsTerminal};
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::sync::LazyLock;

static SHORTCODE_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r":([A-Za-z0-9_-]+):").unwrap());
static TABLE_SHORTCODE_ONLY_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^:([A-Za-z0-9_-]+):$").unwrap());

#[derive(Debug, Clone, Copy)]
struct TransformOptions {
    center_table_icons: bool,
    strict: bool,
}

impl Default for TransformOptions {
    fn default() -> Self {
        Self {
            center_table_icons: true,
            strict: false,
        }
    }
}

#[derive(Debug, Clone, Default)]
struct PreprocessorConfig {
    root: Option<PathBuf>,
    version: Option<String>,
    aliases: HashMap<String, String>,
    options: TransformOptions,
}

fn normalize_shortcode(value: &str) -> String {
    value.trim().trim_matches(':').to_string()
}

fn load_config(context: &Value) -> Result<PreprocessorConfig> {
    let root = context
        .get("root")
        .and_then(Value::as_str)
        .map(PathBuf::from);

    let mut cfg = PreprocessorConfig {
        root,
        ..PreprocessorConfig::default()
    };

    let Some(icons) = context
        .get("config")
        .and_then(|config| config.get("preprocessor"))
        .and_then(|preprocessor| preprocessor.get("icons"))
        .and_then(Value::as_object)
    else {
        return Ok(cfg);
    };

    if let Some(version_val) = icons.get("version") {
        let version = version_val
            .as_str()
            .ok_or_else(|| anyhow!("[preprocessor.icons] `version` must be a string"))?;
        cfg.version = Some(iconify::validate_version(version)?.to_string());
    }

    if let Some(center_val) = icons
        .get("center-table-icons")
        .or_else(|| icons.get("center_table_icons"))
    {
        cfg.options.center_table_icons = center_val.as_bool().ok_or_else(|| {
            anyhow!("[preprocessor.icons] `center-table-icons` must be a boolean")
        })?;
    }

    if let Some(strict_val) = icons.get("strict") {
        cfg.options.strict = strict_val
            .as_bool()
            .ok_or_else(|| anyhow!("[preprocessor.icons] `strict` must be a boolean"))?;
    }

    if let Some(aliases) = icons.get("aliases").and_then(Value::as_object) {
        for (key, value) in aliases {
            if let Some(value) = value.as_str() {
                let alias = normalize_shortcode(key);
                let target = normalize_shortcode(value);
                if !alias.is_empty() && !target.is_empty() {
                    cfg.aliases.insert(alias, target);
                }
            }
        }
    }

    Ok(cfg)
}

fn replace_in_book(book: &mut Value, store: &IconStore, options: TransformOptions) -> Result<()> {
    let items = if let Some(items) = book.get_mut("sections").and_then(Value::as_array_mut) {
        items
    } else if let Some(items) = book.get_mut("items").and_then(Value::as_array_mut) {
        items
    } else {
        return Err(anyhow!("mdBook input missing top-level book items array"));
    };

    for item in items {
        replace_in_book_item(item, store, options)?;
    }

    Ok(())
}

fn replace_in_book_item(
    item: &mut Value,
    store: &IconStore,
    options: TransformOptions,
) -> Result<()> {
    let Some(obj) = item.as_object_mut() else {
        return Ok(());
    };

    if let Some(chapter) = obj.get_mut("Chapter") {
        let chapter = chapter
            .as_object_mut()
            .ok_or_else(|| anyhow!("chapter item is not an object"))?;

        let chapter_label = chapter
            .get("path")
            .and_then(Value::as_str)
            .or_else(|| chapter.get("name").and_then(Value::as_str))
            .unwrap_or("<unnamed chapter>")
            .to_string();

        if let Some(content) = chapter.get_mut("content") {
            let content_str = content
                .as_str()
                .ok_or_else(|| anyhow!("chapter content is not a string"))?;
            let transformed = transform_markdown_with_options(content_str, store, options)
                .with_context(|| format!("failed to process chapter '{chapter_label}'"))?;
            *content = Value::String(transformed);
        }

        if let Some(sub_items) = chapter.get_mut("sub_items").and_then(Value::as_array_mut) {
            for sub_item in sub_items {
                replace_in_book_item(sub_item, store, options)?;
            }
        }
    }

    Ok(())
}

fn markdown_options() -> Options {
    let mut options = Options::empty();
    options.insert(Options::ENABLE_TABLES);
    options.insert(Options::ENABLE_FOOTNOTES);
    options.insert(Options::ENABLE_STRIKETHROUGH);
    options.insert(Options::ENABLE_TASKLISTS);
    options.insert(Options::ENABLE_HEADING_ATTRIBUTES);
    options.insert(Options::ENABLE_GFM);
    options.insert(Options::ENABLE_YAML_STYLE_METADATA_BLOCKS);
    options.insert(Options::ENABLE_PLUSES_DELIMITED_METADATA_BLOCKS);
    options
}

fn should_skip_tag(tag: &Tag<'_>) -> bool {
    matches!(
        tag,
        Tag::CodeBlock(_) | Tag::Image { .. } | Tag::MetadataBlock(_)
    )
}

fn should_skip_tag_end(tag_end: &TagEnd) -> bool {
    matches!(
        tag_end,
        TagEnd::CodeBlock | TagEnd::Image | TagEnd::MetadataBlock(_)
    )
}

fn is_escaped_at(input: &str, pos: usize) -> bool {
    let backslashes = input.as_bytes()[..pos]
        .iter()
        .rev()
        .take_while(|&&b| b == b'\\')
        .count();
    backslashes % 2 == 1
}

fn is_adjacent_to_colon(input: &str, start: usize, end: usize) -> bool {
    let bytes = input.as_bytes();
    (start > 0 && bytes[start - 1] == b':') || bytes.get(end) == Some(&b':')
}

fn resolve_shortcode_for_transform(
    shortcode: &str,
    store: &IconStore,
    strict: bool,
) -> Result<Option<String>> {
    if !store.has_alias(shortcode) && shortcode.bytes().all(|b| b.is_ascii_digit()) {
        return Ok(None);
    }

    if strict {
        match store.render_shortcode_result(shortcode) {
            Ok(Some(svg)) => Ok(Some(svg)),
            Ok(None) => Err(anyhow!("unknown icon shortcode ':{shortcode}:'")),
            Err(err) => Err(err.context(format!("failed to render ':{shortcode}:'"))),
        }
    } else {
        Ok(store.render_shortcode(shortcode))
    }
}

fn collect_inline_replacements(
    input: &str,
    range: Range<usize>,
    store: &IconStore,
    strict: bool,
    replacements: &mut Vec<(Range<usize>, String)>,
) -> Result<()> {
    let slice = &input[range.start..range.end];
    for caps in SHORTCODE_RE.captures_iter(slice) {
        let full = caps.get(0).unwrap();
        let abs_start = range.start + full.start();
        let abs_end = range.start + full.end();
        if is_escaped_at(input, abs_start) || is_adjacent_to_colon(input, abs_start, abs_end) {
            continue;
        }
        let shortcode = caps.get(1).unwrap().as_str();
        if let Some(svg) = resolve_shortcode_for_transform(shortcode, store, strict)? {
            replacements.push((abs_start..abs_end, svg));
        }
    }
    Ok(())
}

#[cfg(test)]
fn transform_markdown(input: &str, store: &IconStore) -> String {
    transform_markdown_with_options(input, store, TransformOptions::default())
        .expect("non-strict markdown transform should not fail")
}

fn transform_markdown_with_options(
    input: &str,
    store: &IconStore,
    options: TransformOptions,
) -> Result<String> {
    let parser = MarkdownParser::new_ext(input, markdown_options()).into_offset_iter();
    let mut replacements: Vec<(Range<usize>, String)> = Vec::new();

    let mut skip_depth: usize = 0;
    let mut in_table_cell = false;
    let mut cell_event_count: usize = 0;
    let mut cell_text_ranges: Vec<Range<usize>> = Vec::new();

    for (event, range) in parser {
        match event {
            Event::Start(Tag::TableCell) => {
                in_table_cell = true;
                cell_event_count = 0;
                cell_text_ranges.clear();
            }
            Event::End(TagEnd::TableCell) => {
                if options.center_table_icons
                    && cell_event_count == 1
                    && cell_text_ranges.len() == 1
                {
                    let r = &cell_text_ranges[0];
                    let slice = &input[r.start..r.end];
                    let trimmed = slice.trim();
                    let trim_offset = trimmed.as_ptr() as usize - slice.as_ptr() as usize;
                    let abs_start = r.start + trim_offset;
                    let abs_end = abs_start + trimmed.len();
                    if !is_escaped_at(input, abs_start)
                        && !is_adjacent_to_colon(input, abs_start, abs_end)
                        && let Some(shortcode) = TABLE_SHORTCODE_ONLY_RE
                            .captures(trimmed)
                            .and_then(|caps| caps.get(1).map(|m| m.as_str()))
                        && let Some(svg) =
                            resolve_shortcode_for_transform(shortcode, store, options.strict)?
                    {
                        replacements.push((
                            abs_start..abs_end,
                            format!(
                                "<span class=\"mdbook-icon-cell\" style=\"display:inline-flex; width: 100%; justify-content: center;\">{svg}</span>"
                            ),
                        ));
                        in_table_cell = false;
                        continue;
                    }
                }

                for text_range in cell_text_ranges.drain(..) {
                    collect_inline_replacements(
                        input,
                        text_range,
                        store,
                        options.strict,
                        &mut replacements,
                    )?;
                }
                in_table_cell = false;
            }
            Event::Start(ref tag) => {
                if in_table_cell {
                    cell_event_count += 1;
                }
                if should_skip_tag(tag) {
                    skip_depth += 1;
                }
            }
            Event::End(ref tag_end) => {
                if in_table_cell {
                    cell_event_count += 1;
                }
                if should_skip_tag_end(tag_end) {
                    skip_depth = skip_depth.saturating_sub(1);
                }
            }
            Event::Text(_) => {
                if in_table_cell {
                    cell_event_count += 1;
                    if skip_depth == 0 {
                        cell_text_ranges.push(range);
                    }
                } else if skip_depth == 0 {
                    collect_inline_replacements(
                        input,
                        range,
                        store,
                        options.strict,
                        &mut replacements,
                    )?;
                }
            }
            _ => {
                if in_table_cell {
                    cell_event_count += 1;
                }
            }
        }
    }

    if replacements.is_empty() {
        return Ok(input.to_string());
    }

    let mut out = String::with_capacity(input.len() + replacements.len() * 160);
    let mut last_end = 0;
    for (range, replacement) in replacements {
        debug_assert!(range.start >= last_end);
        out.push_str(&input[last_end..range.start]);
        out.push_str(&replacement);
        last_end = range.end;
    }
    out.push_str(&input[last_end..]);
    Ok(out)
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
                Some(version) => iconify::validate_version(&version)?.to_string(),
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
    if stdin.is_terminal() {
        let mut cmd = Cli::command();
        cmd.print_help()?;
        eprintln!(
            "\nNote: `mdbook-icons` runs as an mdBook preprocessor (reading JSON from stdin).\n\
             Add `[preprocessor.icons]` to your `book.toml` and run `mdbook build`."
        );
        return Ok(());
    }

    let mut input: Value =
        serde_json::from_reader(stdin).context("failed to parse mdbook preprocessing input")?;

    let context = input
        .as_array()
        .and_then(|items| items.first())
        .cloned()
        .ok_or_else(|| anyhow!("mdBook input must be a tuple of (context, book)"))?;

    let book = input
        .as_array_mut()
        .and_then(|items| items.get_mut(1))
        .ok_or_else(|| anyhow!("mdBook input must be a tuple of (context, book)"))?;

    let cfg = load_config(&context)?;
    let store = IconStore::from_config(cfg.root.as_deref(), cfg.version.as_deref(), cfg.aliases)?;
    replace_in_book(book, &store, cfg.options).context("failed to process book contents")?;

    serde_json::to_writer(io::stdout(), book).context("failed to write processed book")?;
    Ok(())
}

#[derive(Debug, Deserialize)]
struct NpmLatestMeta {
    version: String,
}

fn fetch_latest_iconify_version() -> Result<String> {
    let url = "https://registry.npmjs.org/@iconify/json/latest";
    let mut response = iconify::http_agent()
        .get(url)
        .call()
        .with_context(|| format!("downloading Iconify package metadata from {url}"))?;
    let meta: NpmLatestMeta = response
        .body_mut()
        .read_json()
        .context("parsing latest Iconify package metadata")?;
    Ok(iconify::validate_version(&meta.version)?.to_string())
}

fn write_iconify_version_file(version: &str) -> Result<PathBuf> {
    let path = find_iconify_version_file();
    fs::write(&path, format!("{version}\n"))
        .with_context(|| format!("writing {}", path.display()))?;
    Ok(path)
}

fn read_iconify_version_file(path: &Path) -> Result<String> {
    let version =
        fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
    Ok(version.trim().to_string())
}

fn find_iconify_version_file() -> PathBuf {
    iconify::find_version_file().unwrap_or_else(|| PathBuf::from(iconify::VERSION_FILE_NAME))
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
            output.contains(
                "class=\"mdbook-icon-cell\" style=\"display:inline-flex; width: 100%; justify-content: center;\""
            ),
            "icon-only table cells should be centered"
        );
    }

    #[test]
    fn handles_multi_backtick_code_spans_and_nested_fences() {
        let store = IconStore::new();
        let input = concat!(
            "Double backticks: `` `:mdi-home:` and :mdi-home: ``\n\n",
            "````md\n",
            "```md\n",
            ":mdi-home:\n",
            "```\n",
            ":mdi-home:\n",
            "````\n",
            "Outside: :mdi-home:\n",
        );
        let output = transform_markdown(input, &store);

        assert!(
            output.contains("`` `:mdi-home:` and :mdi-home: ``"),
            "double-backtick inline code should not be modified"
        );
        assert!(
            output.contains("````md\n```md\n:mdi-home:\n```\n:mdi-home:\n````"),
            "4-backtick outer fence should not close on inner 3-backtick fence"
        );
        assert!(
            output.contains("Outside: <svg"),
            "shortcode after nested fence should still render"
        );
    }

    #[test]
    fn skips_urls_images_html_attributes_and_escaped_shortcodes() {
        let store = IconStore::new();
        let input = concat!(
            "[Go :mdi-home:](https://example.com/:mdi-home:/page \":mdi-home:\")\n\n",
            "![:mdi-home:](https://example.com/logo.png)\n\n",
            "<span data-icon=\":mdi-home:\">Visible :mdi-home:</span>\n\n",
            "<script>const icon = \":mdi-home:\";</script>\n\n",
            "Escaped \\:mdi-home:\n",
        );
        let output = transform_markdown(input, &store);

        assert!(
            output.contains("](https://example.com/:mdi-home:/page \":mdi-home:\")"),
            "link destination and title should be untouched"
        );
        assert!(
            output.contains("[Go <svg"),
            "visible link label shortcode should render"
        );
        assert!(
            output.contains("![:mdi-home:](https://example.com/logo.png)"),
            "image alt text should not receive raw svg"
        );
        assert!(
            output.contains("<span data-icon=\":mdi-home:\">Visible <svg"),
            "inline html attribute should be untouched while inner text renders"
        );
        assert!(
            output.contains("<script>const icon = \":mdi-home:\";</script>"),
            "script html blocks should be untouched"
        );
        assert!(
            output.contains("Escaped \\:mdi-home:"),
            "backslash-escaped shortcode should be untouched"
        );
    }

    #[test]
    fn handles_pipeless_tables_escaped_pipes_and_code_in_cells() {
        let store = IconStore::new();
        let input = concat!(
            "Label | Icon | Code\n",
            "--- | --- | ---\n",
            "A \\| B | :twemoji-check-mark-button: | `:twemoji-check-mark-button:`\n",
        );
        let output = transform_markdown(input, &store);

        assert!(
            output.contains("A \\| B |"),
            "escaped pipe in table cell should remain intact"
        );
        assert!(
            output.contains("display:inline-flex; width: 100%; justify-content: center;"),
            "icon-only cell in pipeless GFM table should be centered"
        );
        assert!(
            output.contains("`:twemoji-check-mark-button:`"),
            "inline code in table cell should not be replaced"
        );
    }

    #[test]
    fn loads_book_toml_preprocessor_options() {
        let context = serde_json::json!({
            "root": "/tmp/my-book",
            "config": {
                "preprocessor": {
                    "icons": {
                        "version": "2.2.556",
                        "center-table-icons": false,
                        "strict": true,
                        "aliases": {
                            "home": ":mdi-home:"
                        }
                    }
                }
            }
        });

        let cfg = load_config(&context).expect("valid config");
        assert_eq!(cfg.root, Some(PathBuf::from("/tmp/my-book")));
        assert_eq!(cfg.version.as_deref(), Some("2.2.556"));
        assert!(!cfg.options.center_table_icons);
        assert!(cfg.options.strict);
        assert_eq!(
            cfg.aliases.get("home").map(String::as_str),
            Some("mdi-home")
        );
    }

    #[test]
    fn supports_disabling_table_icon_centering_and_strict_mode() {
        let store = IconStore::new();
        let table = "| Status |\n| --- |\n| :twemoji-check-mark-button: |\n";
        let uncentered = transform_markdown_with_options(
            table,
            &store,
            TransformOptions {
                center_table_icons: false,
                strict: false,
            },
        )
        .expect("transform succeeds");
        assert!(
            uncentered.contains("<svg"),
            "icon should still render when centering is disabled"
        );
        assert!(
            !uncentered.contains("mdbook-icon-cell"),
            "centering wrapper should be omitted when center-table-icons = false"
        );

        let strict_opts = TransformOptions {
            center_table_icons: true,
            strict: true,
        };

        let ok_prose =
            "Valid :mdi-home:, Rust path std::io::Read, time 10:30:00, escaped \\:mdii-home:";
        let rendered = transform_markdown_with_options(ok_prose, &store, strict_opts)
            .expect("rust paths, timestamps, and escaped shortcodes should pass strict mode");
        assert!(rendered.contains("std::io::Read"));
        assert!(rendered.contains("10:30:00"));

        let unknown_prefix_err =
            transform_markdown_with_options("Typo :mdii-home:", &store, strict_opts)
                .expect_err("unknown prefix should fail in strict mode");
        assert!(unknown_prefix_err.to_string().contains(":mdii-home:"));

        let missing_icon_err =
            transform_markdown_with_options("Missing :mdi-nope:", &store, strict_opts)
                .expect_err("missing icon in known collection should fail in strict mode");
        assert!(missing_icon_err.to_string().contains(":mdi-nope:"));
    }
}
