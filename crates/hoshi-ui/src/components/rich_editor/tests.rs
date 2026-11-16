use super::{BlockKind, RichEditor, RichEditorEvent};
use gpui_kit::{
    AppContext as _, Bounds, Entity, Point, TestAppContext, VisualTestContext, WindowBounds,
    WindowOptions, px, size,
};
use std::cell::RefCell;
use std::rc::Rc;

struct Harness {
    cx: VisualTestContext,
    editor: Entity<RichEditor>,
    submitted: Rc<RefCell<usize>>,
}

fn harness(cx: &mut TestAppContext) -> Harness {
    cx.update(|cx| {
        gpui_kit::init(cx);
        crate::theme::apply(gpui_kit::component::theme::ThemeMode::Dark, cx);
    });
    let submitted = Rc::new(RefCell::new(0));
    let (window, editor) = cx.update(|cx| {
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(Bounds {
                origin: Point::default(),
                size: size(px(600.), px(400.)),
            })),
            ..Default::default()
        };
        gpui_kit::open_window(options, cx, |window, cx| {
            cx.new(|cx| RichEditor::new(window, cx))
        })
        .unwrap()
    });
    let mut cx = VisualTestContext::from_window(window, cx);
    let counter = submitted.clone();
    cx.update(|_, cx| {
        cx.subscribe(&editor, move |_, event: &RichEditorEvent, _| {
            if *event == RichEditorEvent::Submit {
                *counter.borrow_mut() += 1;
            }
        })
        .detach();
    });
    let keys = editor.downgrade();
    cx.update(|_, cx| {
        cx.intercept_keystrokes(move |event, window, cx| {
            let handled = keys
                .update(cx, |editor, cx| {
                    editor.handle_key(&event.keystroke, window, cx)
                })
                .unwrap_or(false);
            if handled {
                cx.stop_propagation();
            }
        })
        .detach();
    });
    editor.update_in(&mut cx, |editor, window, cx| editor.focus(window, cx));
    cx.run_until_parked();
    Harness {
        cx,
        editor,
        submitted,
    }
}

fn markdown(h: &mut Harness) -> String {
    h.editor.read_with(&h.cx, |editor, cx| editor.markdown(cx))
}

fn kinds(h: &mut Harness) -> Vec<BlockKind> {
    h.editor.read_with(&h.cx, |editor, cx| {
        editor
            .blocks(cx)
            .into_iter()
            .map(|block| block.kind)
            .collect()
    })
}

#[gpui_kit::test]
fn markdown_typed_at_the_start_turns_into_blocks(cx: &mut TestAppContext) {
    let mut h = harness(cx);
    h.cx.simulate_input("> quoted");
    h.cx.simulate_keystrokes("shift-enter");
    h.cx.simulate_input("again");
    h.cx.simulate_keystrokes("shift-enter shift-enter");
    h.cx.simulate_input("- item");
    h.cx.simulate_keystrokes("shift-enter");
    h.cx.simulate_input("next");
    h.cx.run_until_parked();

    assert_eq!(markdown(&mut h), "> quoted\n> again\n\n- item\n- next");
    assert_eq!(
        kinds(&mut h),
        [
            BlockKind::Quote,
            BlockKind::Quote,
            BlockKind::Bullet,
            BlockKind::Bullet
        ]
    );
}

#[gpui_kit::test]
fn a_fence_opens_a_code_block_where_enter_breaks_lines(cx: &mut TestAppContext) {
    let mut h = harness(cx);
    h.cx.simulate_input("```tsx");
    h.cx.simulate_keystrokes("enter");
    h.cx.simulate_input("const a = 1");
    h.cx.simulate_keystrokes("enter");
    h.cx.simulate_input("a");
    h.cx.run_until_parked();

    assert_eq!(*h.submitted.borrow(), 0);
    assert_eq!(markdown(&mut h), "```tsx\nconst a = 1\na\n```");

    h.cx.simulate_keystrokes("ctrl-enter");
    assert_eq!(*h.submitted.borrow(), 1);
}

#[gpui_kit::test]
fn enter_submits_and_backspace_at_the_start_joins_blocks(cx: &mut TestAppContext) {
    let mut h = harness(cx);
    h.cx.simulate_input("first");
    h.cx.simulate_keystrokes("shift-enter");
    h.cx.simulate_input("second");
    assert_eq!(markdown(&mut h), "first\nsecond");

    h.cx.simulate_keystrokes("home backspace");
    h.cx.run_until_parked();
    assert_eq!(markdown(&mut h), "firstsecond");

    h.cx.simulate_keystrokes("enter");
    assert_eq!(*h.submitted.borrow(), 1);
}

#[gpui_kit::test]
fn backspace_at_the_start_of_a_list_item_makes_it_a_paragraph(cx: &mut TestAppContext) {
    let mut h = harness(cx);
    h.cx.simulate_input("1. step");
    h.cx.simulate_keystrokes("home backspace");
    h.cx.run_until_parked();
    assert_eq!(kinds(&mut h), [BlockKind::Paragraph]);
    assert_eq!(markdown(&mut h), "step");
}

#[gpui_kit::test]
fn markdown_set_from_outside_reads_back_the_same(cx: &mut TestAppContext) {
    let mut h = harness(cx);
    let text = "hello **bold** *it* `code` ~~gone~~\n\n> quote\n\n```rust\nfn main() {}\n```\n\n1. a\n2. b";
    h.editor.update_in(&mut h.cx, |editor, window, cx| {
        editor.set_markdown(text, window, cx)
    });
    assert_eq!(markdown(&mut h), text);
    let snapshot = h.editor.read_with(&h.cx, |editor, cx| editor.snapshot(cx));
    h.editor.update_in(&mut h.cx, |editor, window, cx| {
        editor.set_markdown("", window, cx);
        editor.restore(snapshot, window, cx);
    });
    assert_eq!(markdown(&mut h), text);
}

#[gpui_kit::test]
fn enter_in_the_language_search_picks_the_first_match(cx: &mut TestAppContext) {
    let mut h = harness(cx);
    h.cx.simulate_input("```tsx");
    h.cx.simulate_keystrokes("enter");
    h.cx.simulate_input("let a = 1;");
    h.editor.update_in(&mut h.cx, |editor, window, cx| {
        let id = editor.blocks[0].id;
        editor.open_language_picker(Some(id), window, cx);
        editor
            .language_search
            .update(cx, |search, cx| search.set_value("rus", window, cx));
        editor.pick_first_language(window, cx);
    });
    h.cx.run_until_parked();
    assert_eq!(markdown(&mut h), "```rust\nlet a = 1;\n```");
    assert_eq!(*h.submitted.borrow(), 0);
}

#[gpui_kit::test]
fn select_all_spans_every_block_and_clears_them_at_once(cx: &mut TestAppContext) {
    let mut h = harness(cx);
    h.cx.simulate_input("first");
    h.cx.simulate_keystrokes("shift-enter");
    h.cx.simulate_input("> quoted");
    h.cx.simulate_keystrokes("ctrl-a backspace");
    h.cx.run_until_parked();
    assert_eq!(kinds(&mut h), [BlockKind::Paragraph]);
    assert_eq!(markdown(&mut h), "");

    h.cx.simulate_input("one");
    h.cx.simulate_keystrokes("shift-enter");
    h.cx.simulate_input("two");
    h.cx.simulate_keystrokes("ctrl-a x");
    h.cx.run_until_parked();
    assert_eq!(markdown(&mut h), "x");

    h.cx.simulate_keystrokes("shift-enter");
    h.cx.simulate_input("y");
    h.cx.simulate_keystrokes("ctrl-a right z");
    h.cx.run_until_parked();
    assert_eq!(markdown(&mut h), "x\nyz");
}

#[gpui_kit::test]
fn a_snapshot_brings_back_blocks_and_chips(cx: &mut TestAppContext) {
    let mut h = harness(cx);
    let blocks = vec![
        super::BlockSnapshot {
            kind: BlockKind::Heading(2),
            text: "Plan".into(),
            chips: Vec::new(),
        },
        super::BlockSnapshot {
            kind: BlockKind::Paragraph,
            text: "see [image:3] and #12".into(),
            chips: vec![
                super::ChipData {
                    range: 4..13,
                    id: "attachment:3".into(),
                    text: "[image:3]".into(),
                    label: "Image".into(),
                },
                super::ChipData {
                    range: 18..21,
                    id: "pr:12".into(),
                    text: "#99".into(),
                    label: "#99".into(),
                },
            ],
        },
    ];
    h.editor.update_in(&mut h.cx, |editor, window, cx| {
        editor.restore(blocks.clone(), window, cx)
    });
    h.cx.run_until_parked();
    let back = h.editor.read_with(&h.cx, |editor, cx| editor.snapshot(cx));
    assert_eq!(back[0], blocks[0]);
    assert_eq!(back[1].text, "see [image:3] and #12");
    assert_eq!(
        back[1].chips,
        vec![blocks[1].chips[0].clone()],
        "a chip whose text no longer matches stays plain text"
    );
}

fn primary(key: &str) -> String {
    if cfg!(target_os = "macos") {
        format!("cmd-{key}")
    } else {
        format!("ctrl-{key}")
    }
}

#[gpui_kit::test]
fn mark_shortcuts_wrap_and_unwrap_the_selection_keeping_chips(cx: &mut TestAppContext) {
    let mut h = harness(cx);
    let chip = super::ChipData {
        range: 4..13,
        id: "attachment:3".into(),
        text: "[image:3]".into(),
        label: "Image".into(),
    };
    h.editor.update_in(&mut h.cx, |editor, window, cx| {
        editor.restore(
            vec![super::BlockSnapshot {
                kind: BlockKind::Paragraph,
                text: "fix [image:3] now".into(),
                chips: vec![chip.clone()],
            }],
            window,
            cx,
        );
        editor.focus(window, cx);
    });
    h.cx.simulate_keystrokes(&primary("a"));
    h.cx.simulate_keystrokes(&primary("b"));
    h.cx.run_until_parked();
    let wrapped = h.editor.read_with(&h.cx, |editor, cx| editor.snapshot(cx));
    assert_eq!(wrapped[0].text, "**fix [image:3] now**");
    assert_eq!(wrapped[0].chips[0].range, 6..15);

    h.cx.simulate_keystrokes(&primary("b"));
    h.cx.run_until_parked();
    let back = h.editor.read_with(&h.cx, |editor, cx| editor.snapshot(cx));
    assert_eq!(back[0].text, "fix [image:3] now");
    assert_eq!(back[0].chips, vec![chip]);

    h.cx.simulate_keystrokes("end space");
    h.cx.simulate_keystrokes(&primary("i"));
    h.cx.simulate_input("soon");
    h.cx.simulate_keystrokes(&primary("shift-s"));
    h.cx.simulate_input("x");
    h.cx.run_until_parked();
    assert_eq!(markdown(&mut h), "fix [image:3] now *soon~~x~~*");
}

#[gpui_kit::test]
fn code_blocks_are_code_editors_that_keep_their_text_across_kinds(cx: &mut TestAppContext) {
    let mut h = harness(cx);
    h.cx.simulate_input("```rust");
    h.cx.simulate_keystrokes("enter");
    h.cx.simulate_input("fn main() {}");
    h.cx.run_until_parked();
    let language = |h: &mut Harness| {
        h.editor
            .read_with(&h.cx, |editor, cx| match &editor.blocks[0].input {
                super::BlockInput::Code(state) => Some(state.read(cx).language_name().to_string()),
                super::BlockInput::Text(_) => None,
            })
    };
    assert_eq!(language(&mut h), Some("rust".into()));
    let rows = |h: &mut Harness| {
        h.editor
            .read_with(&h.cx, |editor, cx| editor.blocks[0].input.rows(cx))
    };
    assert_eq!(rows(&mut h), 1);
    h.cx.simulate_keystrokes("enter");
    h.cx.simulate_input("// done");
    h.cx.run_until_parked();
    assert_eq!(rows(&mut h), 2);
    h.cx.simulate_keystrokes(
        "backspace backspace backspace backspace backspace backspace backspace backspace",
    );
    h.cx.run_until_parked();
    assert_eq!(rows(&mut h), 1);
    assert!(
        h.editor
            .read_with(&h.cx, |editor, _| editor.active_input().is_none())
    );

    h.cx.simulate_keystrokes(&primary("b"));
    h.cx.run_until_parked();
    assert_eq!(markdown(&mut h), "```rust\nfn main() {}\n```");

    h.editor.update_in(&mut h.cx, |editor, window, cx| {
        let id = editor.blocks[0].id;
        editor.set_language(id, "ts".into(), window, cx);
    });
    h.cx.run_until_parked();
    assert_eq!(language(&mut h), Some("ts".into()));
    assert_eq!(markdown(&mut h), "```ts\nfn main() {}\n```");

    h.editor.update_in(&mut h.cx, |editor, window, cx| {
        editor.clear_all(window, cx);
    });
    h.cx.run_until_parked();
    assert_eq!(kinds(&mut h), [BlockKind::Paragraph]);
    assert!(
        h.editor
            .read_with(&h.cx, |editor, _| editor.active_input().is_some())
    );
    h.cx.simulate_input("again");
    assert_eq!(markdown(&mut h), "again");
}

#[gpui_kit::test]
fn a_one_line_code_block_is_one_line_tall(cx: &mut TestAppContext) {
    let mut h = harness(cx);
    h.editor.update_in(&mut h.cx, |editor, window, cx| {
        editor.set_markdown("```rust\nlet a = 1;\n```\n\nafter", window, cx);
        editor.focus_block(0, super::Caret::End, window, cx);
    });
    h.cx.run_until_parked();
    let code_top = h.editor.read_with(&h.cx, |editor, cx| {
        editor.blocks[0]
            .input
            .caret_geometry(cx)
            .map(|(_, _, bounds)| bounds.top())
    });
    h.editor.update_in(&mut h.cx, |editor, window, cx| {
        editor.focus_block(1, super::Caret::End, window, cx);
    });
    h.cx.run_until_parked();
    let (after_top, line_height) = h
        .editor
        .read_with(&h.cx, |editor, cx| {
            editor.blocks[1]
                .input
                .caret_geometry(cx)
                .map(|(_, line_height, bounds)| (bounds.top(), line_height))
        })
        .unwrap();
    let gap = after_top - code_top.unwrap();
    assert!(gap > line_height, "{gap:?}");
    assert!(gap < line_height * 2., "{gap:?} against {line_height:?}");
}

#[gpui_kit::test]
fn a_mark_shows_its_markers_only_while_the_caret_is_near(cx: &mut TestAppContext) {
    let mut h = harness(cx);
    let draw = |h: &mut Harness| {
        h.cx.update(|window, cx| window.draw(cx).clear(cx));
        h.cx.run_until_parked();
    };
    let state = |h: &mut Harness| {
        h.editor.read_with(&h.cx, |editor, cx| {
            let input = editor.active_input().unwrap().read(cx);
            let folded: Vec<String> = input
                .tokens()
                .iter()
                .map(|span| span.token().label().to_string())
                .collect();
            (input.decorations().len(), folded)
        })
    };
    h.cx.simulate_input("make it **bold**");
    draw(&mut h);
    assert_eq!(state(&mut h), (3, Vec::new()), "the caret touches the mark");

    h.cx.simulate_input(" now");
    draw(&mut h);
    assert_eq!(
        state(&mut h),
        (0, vec!["bold".to_owned()]),
        "the caret left it"
    );
    assert_eq!(markdown(&mut h), "make it **bold** now");
    let chips = h
        .editor
        .read_with(&h.cx, |editor, cx| editor.tokens(cx).len());
    assert_eq!(chips, 0, "a folded mark is not a chip");

    h.cx.simulate_keystrokes("left left left left");
    draw(&mut h);
    assert_eq!(state(&mut h), (3, Vec::new()), "back next to it");
}

#[gpui_kit::test]
fn undoing_a_change_inside_a_folded_mark_brings_back_the_text(cx: &mut TestAppContext) {
    let mut h = harness(cx);
    let draw = |h: &mut Harness| {
        h.cx.update(|window, cx| window.draw(cx).clear(cx));
        h.cx.run_until_parked();
    };
    let folded = |h: &mut Harness| {
        h.editor.read_with(&h.cx, |editor, cx| {
            let input = editor.active_input().unwrap().read(cx);
            input
                .tokens()
                .iter()
                .map(|span| span.token().label().to_string())
                .collect::<Vec<_>>()
        })
    };
    h.cx.simulate_input("x **a** y");
    draw(&mut h);
    assert_eq!(folded(&mut h), ["a"]);

    h.cx.simulate_keystrokes("left left");
    draw(&mut h);
    h.cx.simulate_keystrokes("left left");
    h.cx.simulate_input("b");
    h.cx.simulate_keystrokes("end");
    draw(&mut h);
    assert_eq!(
        folded(&mut h),
        ["ab"],
        "the caret left the mark it typed in"
    );

    h.cx.simulate_keystrokes(&primary("z"));
    draw(&mut h);
    assert_eq!(markdown(&mut h), "x **a** y");
    assert!(
        folded(&mut h).is_empty(),
        "the caret is back inside the mark"
    );
}

#[gpui_kit::test]
fn a_long_folded_mark_folds_word_by_word_so_it_can_wrap(cx: &mut TestAppContext) {
    let mut h = harness(cx);
    h.editor.update_in(&mut h.cx, |editor, window, cx| {
        editor.set_markdown("start ~~one two  three~~ end", window, cx);
        editor.focus_block(0, super::Caret::End, window, cx);
    });
    h.cx.update(|window, cx| window.draw(cx).clear(cx));
    h.cx.run_until_parked();
    let (pieces, gaps) = h.editor.read_with(&h.cx, |editor, cx| {
        let input = editor.active_input().unwrap().read(cx);
        let pieces: Vec<(String, String)> = input
            .tokens()
            .iter()
            .map(|span| {
                (
                    span.token().text().to_string(),
                    span.token().label().to_string(),
                )
            })
            .collect();
        let gaps: Vec<_> = input
            .decorations()
            .iter()
            .map(|decoration| decoration.range.clone())
            .collect();
        (pieces, gaps)
    });
    let piece = |text: &str, label: &str| (text.to_owned(), label.to_owned());
    assert_eq!(
        pieces,
        [
            piece("~~one", "one"),
            piece("two", "two"),
            piece("three~~", "three")
        ]
    );
    assert_eq!(
        gaps,
        [11..12, 15..17],
        "the spaces between keep the mark's style"
    );
    assert_eq!(markdown(&mut h), "start ~~one two  three~~ end");
}

/// Soft wrap measures a bold mark the caret is in, whose text is drawn bold, with bold glyphs,
/// also beside a folded mark.
#[cfg(target_os = "windows")]
#[test]
fn a_bold_mark_under_the_caret_wraps_as_drawn() {
    let platform = gpui_kit::platform::current_platform(false);
    let mut cx = TestAppContext::build_with_text_system(
        gpui_kit::TestDispatcher::new(0),
        None,
        platform.text_system(),
    );
    let mut h = harness(&mut cx);
    h.cx.update(|_, cx| {
        let fonts = crate::theme::Fonts {
            composer: Some(("Arial".into(), px(14.))),
            ..Default::default()
        };
        crate::theme::set_fonts(fonts, cx);
    });
    for (prefix, words) in ["", "*x* "]
        .into_iter()
        .flat_map(|prefix| (1..60).map(move |words| (prefix, words)))
    {
        let text = vec!["wide"; words].join(" ");
        h.editor.update_in(&mut h.cx, |editor, window, cx| {
            editor.set_markdown(&format!("{prefix}**{text}**"), window, cx);
            editor.focus_block(0, super::Caret::End, window, cx);
        });
        h.cx.update(|window, cx| window.draw(cx).clear(cx));
        h.cx.run_until_parked();
        let (caret, bounds, decorations) = h.editor.read_with(&h.cx, |editor, cx| {
            let input = editor.active_input().unwrap().read(cx);
            let (caret, _, bounds) = editor.blocks[0].input.caret_geometry(cx).unwrap();
            (caret, bounds, input.decorations().len())
        });
        assert_eq!(decorations, 3, "the caret keeps the mark open");
        assert!(
            caret.left() <= bounds.right(),
            "{prefix:?} and {words} words: the caret at {caret:?} is past {bounds:?}"
        );
    }
}
