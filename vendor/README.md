# Vendored crates

## gpui-base 0.9.1

From crates.io (Apache-2.0, https://github.com/longbridge/gpui-kit), patched in through
`[patch.crates-io]` in the root `Cargo.toml`. Four changes.

### Textarea decorations

A textarea (`TextareaState`) takes text
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

### Highlighted selectable text

- `src/selectable_text.rs`: `SelectableText::highlights` styles ranges of its text, so the
  changes panel's diff lines show their syntax colors and stay selectable.

### Markdown it can't parse

- `src/text/format/markdown.rs`: when `markdown::to_mdast` fails, `parse` shows the source as
  plain text, a paragraph per line (`plain_lines`), instead of an error in place of the text.

### Long documents in a scrolling parent

A long answer in the transcript laid out and painted every block on every frame while it showed,
so scrolling past it cost several times a frame's budget.

- `src/text/markdown_ext.rs`: `MarkdownExtensions::lazy_blocks` turns it on, for a view that is
  as wide as its container. A stand-in has no width of its own, so a view that sizes its container
  to its text, as a message bubble, must leave it off.
- `src/text/lazy_block.rs`: `LazyBlock` draws a top-level block whole while its room is unknown
  and records it in `BlockHeights`: from the bottom of the block above to its own bottom, so a
  margin that collapses above it (the transcript's code blocks have one) counts. Once every block
  is measured, a block outside the content mask only takes up that room; one that comes into view
  is built in prepaint, as `gpui::list` builds its items. The first and last blocks are always
  built, since their margins can collapse into the document's edges. A new document, style, rem
  or font size, a new width, a reflow (`invalidate_measured_heights`, as when an inline resource
  loads) or a frame drawn whole forgets the heights, and a block standing in while they are
  forgotten never records the room it stood in at.
- `src/text/document.rs`: `render_root` lays the blocks of a non-scrollable view out as
  `LazyBlock`s when given heights. `src/text/state.rs` gives them unless the view clamps to
  `max_lines`, reveals a line or fades streamed text in.
- `src/text/state.rs`: `block_ix_at` finds a block from the heights for such a view too, so a
  selection whose blocks were never painted still copies them whole, as in a scrollable view.

Drop this copy and the patch once an upstream release offers the same.

## markdown 1.0.0

From crates.io (MIT, https://github.com/wooorm/markdown-rs), patched in through
`[patch.crates-io]` in the root `Cargo.toml`. One change.

### Errors instead of panics

For some inputs, such as ``"- ```\n1)"`` or ``"+ $$\n1)"``, the tokenizer makes events that
don't nest, and `to_mdast` panicked on them; release builds abort on a panic, so a message
holding such text closed the app.

- `src/to_mdast.rs`: a mismatched exit returns an error (`malformed`) instead of
  `unreachable!`, and an enter the tree can't take (a child pushed into a node without
  children) sets `CompileContext::broken`, which stops compiling with that error. `tail_pop`
  returns it too when its stacks run out.
- `src/lib.rs`: `to_mdast`'s docs say it can return that error with MDX off.

Drop this copy and the patch once an upstream release no longer panics on those inputs.
