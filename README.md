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

[preprocessor.icons.aliases]
star = "twemoji-glowing-star"
```

You can also check out a working example on my GitHub Pages site: [mdbook-icons example](https://ld3z.github.io/mdbook-icons/).

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
