# mdBook Icons

A small mdBook extension for writing icons directly in Markdown.

Use an icon shortcode by wrapping the collection and icon name in colons:

```md
:twemoji-glowing-star:
```

When the book is built, the shortcode is replaced with the matching icon.

## What it looks like

### Inline icons

You can drop icons right into a sentence:

The project is on :github: GitHub and does not use :docker: Docker. But is written entirely in [Rust!:rust:](https://rust-lang.org/)

### Icons in headings

Icons also work in headings, which makes section titles feel a little more expressive:

## Hello there :twemoji-waving-hand:

### Lists and callouts

Icons can be mixed into any normal Markdown content:

- :done: Supported in regular text
- :done: Supported in headings
- :done: Supported in tables
- :done: Supported wherever Markdown is rendered

### Tables

| Feature | Example |
| --- | --- |
| Inline icon | Hello :mdi-github: world! |
| Header icon | Hello :twemoji-waving-hand: |
| Code example | :star: |

> [!IMPORTANT]
> Icons get centered in tables with no text from version 0.2.3^! Set `center-table-icons = false` under `[preprocessor.icons]` to opt out.

## Finding more icons

Browse the icon catalog at [Icones](https://icones.js.org/).

## Roadmap

| Feature | Status |
| --- | --- |
| Custom shortcode mappings | :done: |
| Updating command | :done: |
| Custom icon sets | :not-done: |

## Configuration & aliases

Configure `mdbook-icons` in `book.toml` under `[preprocessor.icons]`:

```toml
[preprocessor.icons]
command = "mdbook-icons"
# Optional:
# version = "2.2.556"
# center-table-icons = true
# strict = false

[preprocessor.icons.aliases]
star = ":twemoji-glowing-star:"
```

Values in `aliases` can be written with or without surrounding colons, then used as `:star:` in Markdown.

Rendered icons include `.mdbook-icon` on each `<svg>` and `.mdbook-icon-cell` on centered table wrappers for custom CSS styling.

## Faster Installing

As of mdbook-icons v0.3.2, we supply prebuilt binaries from GitHub for use with `cargo-binstall`!

## Working Versions

This crate should work with every mdBook version since it does not use any specific mdBook features.
