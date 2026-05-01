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

The project is on :mdi-github: GitHub and does not use :skill-icons-docker: Docker. But is written entirely in [Rust!:mdi-language-rust:](https://rust-lang.org/)

### Icons in headings

Icons also work in headings, which makes section titles feel a little more expressive:

## Hello there :twemoji-waving-hand:

### Lists and callouts

Icons can be mixed into any normal Markdown content:

- :twemoji-check-mark-button: Supported in regular text
- :twemoji-check-mark-button: Supported in headings
- :twemoji-check-mark-button: Supported in tables
- :twemoji-check-mark-button: Supported wherever Markdown is rendered

### Tables

| Feature | Example |
| --- | --- |
| Inline icon | Hello :mdi-github: world! |
| Header icon | Hello :twemoji-waving-hand: |
| Code example | :twemoji-glowing-star: |

> [!IMPORTANT]
> Icons get centered in tables with no text!

## Finding more icons

Browse the icon catalog at [icones.js.org](https://icones.js.org/).

## Roadmap

| Feature | Status |
| --- | --- |
| Custom shortcode mappings | :twemoji-hourglass-not-done: |
| Custom icon sets | :twemoji-hourglass-not-done: |

### Tested versions

| Version | Status |
| --- | --- |
| mdBook 0.5 | :twemoji-check-mark-button: |
| mdBook 0.4 | :twemoji-check-mark-button: |
