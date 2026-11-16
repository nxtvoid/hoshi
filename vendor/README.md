# Vendored crates

## gpui-base 0.9.1

From crates.io (Apache-2.0, https://github.com/longbridge/gpui-kit), patched in through
`[patch.crates-io]` in the root `Cargo.toml`. One change: a textarea (`TextareaState`) takes text
decorations, as the code editor already does, and its tokens can be swapped in place, so the
composer can show bold, italic, inline code and strikethrough without their markers.

- `src/input/base/kind.rs`: `TextareaMode`'s extras are `TextareaExtras`, which hold the
  decorations and hand them to the renderer through `InputExtras::decoration_layers`. The
  renderer already applies those layers in every mode.
- `src/input/editor/decorations.rs`: `InputBaseState<TextareaMode>::set_decorations` replaces them
  whole and `decorations` reads them back. Their ranges follow edits like the editor's
  collections (`adjust_annotations` in `kind.rs`), so `set_value("")` clears them.
- `display_map/text_wrapper.rs`: soft wrap measures the font weight and style decorations paint (`set_font_overrides`, fed each prepaint in `element.rs`), so bold text wraps instead of overflowing.
- `src/input/base/inline_tokens.rs`: `set_tokens` swaps the tokens keeping text, selections and
  undo history; it rejects a token through any caret with `InlineTokenError::SplitsSelection` and
  bumps the document revision so the token layout is measured again. Undo of an edit inside a
  token set later drops that token instead of panicking, and `restore_selections` in `state.rs`
  keeps restored carets out of tokens.

Upstream PRs: longbridge/gpui-kit#3970 (decorations) and #3971 (`set_tokens`).

Drop this copy and the patch once an upstream release offers the same.
