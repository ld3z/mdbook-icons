# mdbook-icons

A small mdbook preprocessor that replaces icon shortcodes like `:mdi-home:` or `:twemoji-glowing-star:` with inline SVGs from Iconify.

The icon collections are embedded at build time, so users do not need `npm` installed at runtime.

## Setup

1. Build the preprocessor:

```bash
cargo build --release
```

2. The embedded Iconify version is controlled by the `iconify-json.version` file in the repo.
   To update the binary, edit that file and rebuild:

```bash
# change iconify-json.version, then
cargo build --release
```

   You can still override it temporarily with `ICONIFY_JSON_VERSION` if needed.

3. Optional: if you want to avoid downloading the Iconify package from the network during build, point the build to a local `@iconify/json` checkout:

```bash
export ICONIFY_JSON_DIR=./node_modules/@iconify/json/json
```

The build script also caches the packed Iconify data under `target/iconify-cache` to speed up rebuilds.

4. Add it to your `book.toml`:

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
