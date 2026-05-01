# mdbook-icons

A small mdbook preprocessor that replaces icon shortcodes like `:mdi-home:` or `:twemoji-glowing-star:` with inline SVGs from Iconify.

The icon collections are embedded at build time, so users do not need `npm` installed at runtime.

## Setup

1. Build the preprocessor:

```bash
cargo build --release
```

2. Optional: if you want to avoid downloading the Iconify package from the network during build, point the build to a local `@iconify/json` checkout:

```bash
export ICONIFY_JSON_DIR=./node_modules/@iconify/json/json
```

By default the build script downloads the Iconify JSON package from the npm registry and embeds it into the binary.
It also caches the packed Iconify data under `target/iconify-cache` to speed up rebuilds.

3. Add it to your `book.toml`:

```toml
[preprocessor.icons]
command = "mdbook-icons"
```

## Usage

In Markdown:

```md
:mdi-home: Home
:twemoji-glowing-star: Star
```

The preprocessor will replace the shortcode with inline SVG from Iconify.
