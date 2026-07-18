# mdbook-icons

Replace Iconify shortcodes like `:mdi-home:` and `:twemoji-glowing-star:` with inline SVG in mdBook.

An mdbook preprocessor that turns icon shortcodes into SVGs from Iconify.

## Install

```bash
cargo install mdbook-icons
```

## Configure `book.toml`

Add this to your book's `book.toml`:

```toml
[preprocessor.icons]
command = "mdbook-icons"
```

You can also check out a working example on my GitHub Pages site: [mdbook-icons example](https://ld3z.github.io/mdbook-icons/).

## How icon data is fetched

Icon collections are downloaded on demand the first time a book uses them, and
cached on disk (`~/.cache/mdbook-icons/` on Linux, the equivalent user cache
directory on macOS and Windows). Only the collections your book actually
references are fetched — typically a few hundred kilobytes — and subsequent
builds are served entirely from the cache, so no network access is needed.

For fully offline environments, point `ICONIFY_JSON_DIR` at a local copy of the
[`@iconify/json`](https://www.npmjs.com/package/@iconify/json) package (either
the package root or its `json/` directory) and no downloads will ever happen.

## Updating Icons

The Iconify data version is pinned. It resolves in this order:

1. The `ICONIFY_JSON_VERSION` environment variable.
2. An `iconify-json.version` file found in the current directory or any parent
   (so a book repository can pin its own version).
3. The version this binary was released with.

To update a pinned `iconify-json.version` file to the latest release, run:

```bash
mdbook-icons update
```

You can also pin a specific version:

```bash
mdbook-icons update --version 2.2.481
```

To check whether your pinned version is already current without changing anything:

```bash
mdbook-icons update --check
```

After updating the version file, the next `mdbook build` will fetch icon data for the new version automatically.

## Usage

Find icon names and prefixes at [icones.js.org](https://icones.js.org/).

In Markdown:

```md
:mdi-home: Home
:twemoji-glowing-star: Star
```

The preprocessor will replace the shortcode with inline SVG from Iconify.

## Custom aliases

Define your own shortcodes by adding an `aliases` table under `[preprocessor.icons]`. Values can be written with or without surrounding colons.

```toml
[preprocessor.icons.aliases]
star = ":twemoji-glowing-star:"
```

Then use `:star:` in Markdown.
