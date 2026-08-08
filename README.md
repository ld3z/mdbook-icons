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
```

## Usage

Find collection prefixes and icon names at https://icones.js.org/ and use shortcodes in Markdown:

```md
:mdi-home: Home
:twemoji-glowing-star: Star
```

On `mdbook build` the shortcodes are replaced with inline SVG.

## Cache & offline

Icon collections are downloaded on demand and cached per-user (e.g. `~/.cache/mdbook-icons/` on Linux). Only collections your book uses are fetched, and subsequent builds read from the cache so they're fast.

To use a local copy of the Iconify JSON data (fully offline), set either:

- `ICONIFY_JSON_DIR` to point at a local `@iconify/json` package (the package root or its `json/` dir), or
- add an `iconify-json.version` file in your repo (or a parent directory) to pin a version.

## Updating the icon data

The Iconify data version is resolved in this order:

1. `ICONIFY_JSON_VERSION` environment variable
2. `iconify-json.version` file in the current directory or any parent
3. The version bundled with this binary

Update the pinned version (or create a version file):

```bash
mdbook-icons update
# or pin a specific release
mdbook-icons update --version 2.2.481
# check current pin without changing
mdbook-icons update --check
```

After updating the version file, the next `mdbook build` will fetch the new icon data.

## Custom aliases

You can define short aliases in `book.toml` under `[preprocessor.icons.aliases]`. Values may include surrounding colons or not.

```toml
[preprocessor.icons.aliases]
star = ":twemoji-glowing-star:"
```

Then use `:star:` in your Markdown.

## Examples & links

- Example site: https://ld3z.github.io/mdbook-icons/
- Icon search: https://icones.js.org/
- Iconify JSON package: https://www.npmjs.com/package/@iconify/json

License: MIT
