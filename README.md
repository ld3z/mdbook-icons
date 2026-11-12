# mdbook-icons

[![crates.io downloads](https://img.shields.io/crates/d/mdbook-icons?style=for-the-badge&logo=rust)](https://crates.io/crates/mdbook-icons)

Turn Iconify shortcodes (e.g. `:mdi-home:`, `:twemoji-glowing-star:`) into inline SVGs in mdBook.

A small mdBook preprocessor that replaces icon shortcodes with SVG from Iconify.

## Quick install

Prebuilt binaries are attached to each release: https://github.com/ld3z/mdbook-icons/releases

Install with cargo-binstall (recommended):

```bash
cargo binstall mdbook-icons
```

Or build from source:

```bash
cargo install mdbook-icons
```

## Configure

Add the preprocessor to your `book.toml`:

```toml
[preprocessor.icons]
command = "mdbook-icons"
# Optional settings:
# version = "2.2.556"       # Pin an @iconify/json release directly in book.toml
# center-table-icons = true # Center single-icon table cells (default: true)
# strict = false            # Fail the build on unknown or broken shortcodes (default: false)
```

## Usage

Find collection prefixes and icon names at https://icones.js.org/ and use shortcodes in Markdown:

```md
:mdi-home: Home
:twemoji-glowing-star: Star
```

On `mdbook build` the shortcodes are replaced with inline SVG. Shortcodes inside inline code spans, fenced code blocks, link URLs, image `alt` text, and HTML attributes are automatically ignored. To write a literal shortcode in prose, escape the leading colon (`\:mdi-home:`).

## Cache & offline

Icon collections are downloaded on demand and cached per-user (e.g. `~/.cache/mdbook-icons/` on Linux; override with `MDBOOK_ICONS_CACHE_DIR`). Only collections your book uses are fetched, and subsequent builds read from the cache so they're fast.

To use a local copy of the Iconify JSON data (fully offline), set `ICONIFY_JSON_DIR` to point at a local `@iconify/json` package (the package root or its `json/` dir).

## Updating the icon data

The Iconify data version is resolved in this order:

1. `ICONIFY_JSON_VERSION` environment variable
2. `version` under `[preprocessor.icons]` in `book.toml`
3. `iconify-json.version` file in the book's root directory (or any parent directory)
4. The version bundled with this binary

Update the pinned version file (or create one):

```bash
mdbook-icons update
# or pin a specific release
mdbook-icons update --version 2.2.556
# check current pin without changing
mdbook-icons update --check
```

After updating the version, the next `mdbook build` will fetch the new icon data.

## Custom aliases

You can define short aliases in `book.toml` under `[preprocessor.icons.aliases]`. Values may include surrounding colons or not.

```toml
[preprocessor.icons.aliases]
star = ":twemoji-glowing-star:"
```

Then use `:star:` in your Markdown.

## Styling

Rendered icons include CSS hooks you can target in mdBook's `additional-css`:

- `.mdbook-icon` — applied to every inline `<svg>` element
- `.mdbook-icon-cell` — applied to the centering `<span>` wrapper in icon-only table cells

## Examples & links

- Example site: https://ld3z.github.io/mdbook-icons/
- Icon search: https://icones.js.org/
- Iconify JSON package: https://www.npmjs.com/package/@iconify/json

License: MIT
