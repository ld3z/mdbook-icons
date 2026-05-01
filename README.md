# mdbook-icons

Replace Iconify shortcodes like `:mdi-home:` and `:twemoji-glowing-star:` with inline SVG in mdBook.

An mdbook preprocessor that turns icon shortcodes into SVGs from Iconify.

## Install

```bash
cargo install mdbook-icons
```

## Configure `book.toml`

Copy `book.toml.example` into your book and keep the preprocessor entry:

```toml
[preprocessor.icons]
command = "mdbook-icons"
```

## Usage

Find icon names and prefixes at [icones.js.org](https://icones.js.org/).

In Markdown:

```md
:mdi-home: Home
:twemoji-glowing-star: Star
```

The preprocessor will replace the shortcode with inline SVG from Iconify.
