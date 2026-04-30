use mdbook::book::{Book, BookItem};
use mdbook::errors::Error;
use mdbook::preprocess::{CmdPreprocessor, Preprocessor, PreprocessorContext};
use pulldown_cmark::{Event, Options, Parser, Tag, TagEnd};
use regex::Regex;
use std::collections::HashMap;
use std::fs;
use std::io::{self, Write};
use std::ops::Range;
use std::path::{Path, PathBuf};

const STYLE: &str = r#"
<style>
.mdbook-icon {
  display: inline-block;
  width: 1em;
  height: 1em;
  vertical-align: -0.125em;
  fill: currentColor;
}
.mdbook-icon svg {
  width: 1em;
  height: 1em;
  display: block;
}
</style>
"#;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum IconMode {
    Online,
    CacheOnly,
    CacheThenBundled,
    BundledOnly,
}

impl IconMode {
    fn from_env() -> Self {
        match std::env::var("MDBOOK_ICONS_MODE")
            .unwrap_or_default()
            .to_ascii_lowercase()
            .as_str()
        {
            "cache-only" | "offline" => Self::CacheOnly,
            "bundled-only" => Self::BundledOnly,
            "bundled" => Self::CacheThenBundled,
            _ => Self::Online,
        }
    }

    fn allows_network(self) -> bool {
        matches!(self, Self::Online)
    }

    fn allows_bundled(self) -> bool {
        matches!(self, Self::CacheThenBundled | Self::BundledOnly)
    }
}

struct IconsPreprocessor {
    shortcode: Regex,
    mode: IconMode,
}

impl Default for IconsPreprocessor {
    fn default() -> Self {
        Self {
            shortcode: Regex::new(r":([a-z0-9]+(?:-[a-z0-9]+)+):").unwrap(),
            mode: IconMode::from_env(),
        }
    }
}

impl Preprocessor for IconsPreprocessor {
    fn name(&self) -> &str {
        "mdbook-icons"
    }

    fn supports_renderer(&self, renderer: &str) -> bool {
        renderer == "html"
    }

    fn run(&self, ctx: &PreprocessorContext, mut book: Book) -> Result<Book, Error> {
        let cache_dir = cache_dir(ctx);
        let bundle_dir = bundle_dir(ctx);
        fs::create_dir_all(&cache_dir)?;

        book.for_each_mut(|item| {
            if let BookItem::Chapter(chapter) = item {
                chapter.content = inject_style(&transform_markdown(
                    &chapter.content,
                    &self.shortcode,
                    &cache_dir,
                    &bundle_dir,
                    self.mode,
                ));
            }
        });

        Ok(book)
    }
}

fn main() {
    let mut args = std::env::args().skip(1);
    match args.next().as_deref() {
        Some("supports") => {
            let renderer = args.next().unwrap_or_default();
            std::process::exit((renderer == "html") as i32);
        }
        _ => {
            if let Err(err) = process() {
                eprintln!("mdbook-icons: {err}");
                std::process::exit(1);
            }
        }
    }
}

fn process() -> Result<(), Box<dyn std::error::Error>> {
    let (ctx, book) = CmdPreprocessor::parse_input(io::stdin().lock())?;
    let processor = IconsPreprocessor::default();
    let book = processor.run(&ctx, book)?;
    serde_json::to_writer(io::stdout().lock(), &book)?;
    io::stdout().lock().flush()?;
    Ok(())
}

fn inject_style(content: &str) -> String {
    if content.contains(".mdbook-icon") {
        content.to_string()
    } else {
        let mut out = String::with_capacity(STYLE.len() + content.len());
        out.push_str(STYLE);
        out.push_str(content);
        out
    }
}

fn transform_markdown(
    content: &str,
    shortcode: &Regex,
    cache_dir: &Path,
    bundle_dir: &Path,
    mode: IconMode,
) -> String {
    let mut edits: Vec<(Range<usize>, String)> = Vec::new();
    let mut code_block_depth = 0usize;

    for (event, range) in Parser::new_ext(content, Options::all()).into_offset_iter() {
        match event {
            Event::Start(Tag::CodeBlock(_)) => code_block_depth += 1,
            Event::End(TagEnd::CodeBlock) => {
                code_block_depth = code_block_depth.saturating_sub(1);
            }
            Event::Text(text) if code_block_depth == 0 => {
                let replaced =
                    replace_shortcodes(text.as_ref(), shortcode, cache_dir, bundle_dir, mode);
                if replaced != text.as_ref() {
                    edits.push((range, replaced));
                }
            }
            _ => {}
        }
    }

    apply_edits(content, edits)
}

fn replace_shortcodes(
    content: &str,
    shortcode: &Regex,
    cache_dir: &Path,
    bundle_dir: &Path,
    mode: IconMode,
) -> String {
    let mut icons = HashMap::new();

    shortcode
        .replace_all(content, |caps: &regex::Captures| {
            let shortcode = caps.get(1).unwrap().as_str();
            match shortcode_to_svg(shortcode, cache_dir, bundle_dir, mode, &mut icons) {
                Ok(svg) => format!(r#"<span class="mdbook-icon" aria-hidden="true">{svg}</span>"#),
                Err(_) => caps.get(0).unwrap().as_str().to_string(),
            }
        })
        .into_owned()
}

fn shortcode_to_svg(
    shortcode: &str,
    cache_dir: &Path,
    bundle_dir: &Path,
    mode: IconMode,
    memo: &mut HashMap<String, String>,
) -> Result<String, Box<dyn std::error::Error>> {
    if let Some(svg) = memo.get(shortcode) {
        return Ok(svg.clone());
    }

    let (collection, icon) = shortcode
        .split_once('-')
        .ok_or_else(|| format!("invalid icon shortcode: {shortcode}"))?;

    let cache_key = format!("{}__{}.svg", collection, icon.replace('/', "__"));
    let cache_path = cache_dir.join(cache_key);

    let svg = if cache_path.exists() {
        fs::read_to_string(&cache_path)?
    } else if mode.allows_bundled() {
        if let Some(svg) = read_bundled_icon(bundle_dir, collection, icon)? {
            fs::write(&cache_path, &svg)?;
            svg
        } else if mode.allows_network() {
            fetch_icon(collection, icon, &cache_path)?
        } else {
            return Err(format!("icon not found in cache or bundled icons: {shortcode}").into());
        }
    } else if mode.allows_network() {
        fetch_icon(collection, icon, &cache_path)?
    } else {
        return Err(format!("icon not found in cache: {shortcode}").into());
    };

    memo.insert(shortcode.to_string(), svg.clone());
    Ok(svg)
}

fn fetch_icon(
    collection: &str,
    icon: &str,
    cache_path: &Path,
) -> Result<String, Box<dyn std::error::Error>> {
    let url = format!("https://api.iconify.design/{collection}/{icon}.svg");
    let response = ureq::get(&url).call()?;
    let body = response.into_string()?;
    let body = normalize_svg(&body);
    fs::write(cache_path, &body)?;
    Ok(body)
}

fn read_bundled_icon(
    bundle_dir: &Path,
    collection: &str,
    icon: &str,
) -> Result<Option<String>, Box<dyn std::error::Error>> {
    let path = bundle_dir.join(collection).join(format!("{icon}.svg"));
    if !path.exists() {
        return Ok(None);
    }
    Ok(Some(normalize_svg(&fs::read_to_string(path)?)))
}

fn normalize_svg(svg: &str) -> String {
    let mut s = svg.trim().to_string();
    if let Some(idx) = s.find("<svg") {
        s = s[idx..].to_string();
    }
    if let Some(end) = s.rfind("</svg>") {
        s.truncate(end + "</svg>".len());
    }
    s
}

fn apply_edits(source: &str, mut edits: Vec<(Range<usize>, String)>) -> String {
    if edits.is_empty() {
        return source.to_string();
    }

    edits.sort_by_key(|(range, _)| range.start);

    let mut out = String::with_capacity(source.len());
    let mut last = 0usize;

    for (range, replacement) in edits {
        if range.start < last {
            continue;
        }
        out.push_str(&source[last..range.start]);
        out.push_str(&replacement);
        last = range.end;
    }

    out.push_str(&source[last..]);
    out
}

fn cache_dir(ctx: &PreprocessorContext) -> PathBuf {
    ctx.root.join(".mdbook-icons-cache")
}

fn bundle_dir(ctx: &PreprocessorContext) -> PathBuf {
    std::env::var_os("MDBOOK_ICONS_BUNDLE_DIR")
        .map(PathBuf::from)
        .map(|p| if p.is_absolute() { p } else { ctx.root.join(p) })
        .unwrap_or_else(|| ctx.root.join("icons"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use mdbook::book::Chapter;
    use mdbook::config::Config;
    use std::cell::RefCell;
    use std::collections::HashMap;

    fn ctx() -> PreprocessorContext {
        PreprocessorContext {
            root: PathBuf::from("/tmp/book"),
            config: Config::default(),
            renderer: "html".to_string(),
            mdbook_version: "0.4".to_string(),
            chapter_titles: RefCell::new(HashMap::new()),
            __non_exhaustive: (),
        }
    }

    #[test]
    fn skips_code_blocks() {
        let content = "hello :twemoji-glowing-star:\n\n```md\n:twemoji-heart-suit:\n```\n";
        let transformed = transform_markdown(
            content,
            &Regex::new(r":([a-z0-9]+(?:-[a-z0-9]+)+):").unwrap(),
            Path::new("/tmp/cache"),
            Path::new("/tmp/icons"),
            IconMode::CacheOnly,
        );

        assert!(transformed.contains(":twemoji-glowing-star:"));
        assert!(transformed.contains(":twemoji-heart-suit:"));
    }

    #[test]
    fn injects_style_once() {
        let content = inject_style("abc");
        assert!(content.starts_with(STYLE));
        assert_eq!(inject_style(&content), content);
    }

    #[test]
    fn cache_dir_uses_book_root() {
        assert_eq!(
            cache_dir(&ctx()),
            PathBuf::from("/tmp/book/.mdbook-icons-cache")
        );
    }

    #[test]
    fn replaces_using_cache() {
        let base = std::env::temp_dir().join(format!("mdbook-icons-{}", std::process::id()));
        let cache_dir = base.join("cache");
        let bundle_dir = base.join("icons");
        fs::create_dir_all(&cache_dir).unwrap();
        fs::create_dir_all(&bundle_dir).unwrap();
        fs::write(
            cache_dir.join("twemoji__glowing-star.svg"),
            r#"<svg viewBox='0 0 16 16'><path d='M0 0h16v16H0z'/></svg>"#,
        )
        .unwrap();

        let transformed = transform_markdown(
            "hello :twemoji-glowing-star:",
            &Regex::new(r":([a-z0-9]+(?:-[a-z0-9]+)+):").unwrap(),
            &cache_dir,
            &bundle_dir,
            IconMode::CacheOnly,
        );

        assert!(transformed.contains(r#"<span class="mdbook-icon" aria-hidden="true">"#));
        assert!(transformed.contains("<svg"));
    }
}
