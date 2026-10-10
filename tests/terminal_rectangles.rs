//! Standard DECCRA observation, independent of the product's menu renderer.

#[test]
fn rectangle_pages_preserve_rich_cells_and_leave_cursor_alone() {
    let mut parser = vt100::Parser::new(6, 30, 0);
    parser.process("\x1b[38;2;123;45;67m\x1b[48;5;123m中🚀e\u{301}\x1b[0m\x1b[4;9H".as_bytes());
    let before = parser.screen().clone();
    parser.process(b"\x1b[1;1;1;30;1;1;1;6$v\x1b[1;1HDESTROY\x1b[4;9H\x1b[1;1;1;30;6;1;1;1$v");
    assert_eq!(parser.screen().cursor_position(), before.cursor_position());
    for col in 0..30 {
        assert_eq!(parser.screen().cell(0, col), before.cell(0, col));
    }
}

#[test]
fn overlapping_rectangles_copy_original_cells_and_clip_destination() {
    let mut parser = vt100::Parser::new(3, 8, 0);
    parser.process(b"ABCDEFGH\x1b[2;1Habcdefgh\x1b[1;1;2;6;1;2;3;1$v");
    let rows: Vec<_> = parser.screen().rows(0, 8).collect();
    assert_eq!(rows, ["ABCDEFGH", "abABCDEF", "  abcdef"]);
    let before = parser.screen().clone();
    parser.process(b"\x1b[6;1;7;8;1;1;1;1$v\x1b[1;1;1;8;1;99;1;1$v");
    assert_eq!(parser.screen().contents(), before.contents());
}

#[test]
fn alternate_screen_does_not_overwrite_main_rectangle_pages() {
    let mut parser = vt100::Parser::new(3, 8, 0);
    parser.process(b"MAIN\x1b[1;1;1;4;1;1;1;6$v\x1b[?1049hALT!\x1b[1;1;1;4;1;2;1;6$v");
    assert!(parser.screen().contents().contains("ALT!\nALT!"));
    parser.process(b"\x1b[?1049l\x1b[1;1Hxxxx\x1b[1;1;1;4;6;1;1;1$v");
    assert_eq!(parser.screen().contents(), "MAIN");
}

#[test]
fn clipped_wide_rectangles_remain_safe_to_edit_and_resize() {
    for col in 1..=8 {
        let mut parser = vt100::Parser::new(3, 8, 0);
        parser.process("中🚀中文".as_bytes());
        parser.process(format!("\x1b[1;1;1;8;1;2;{col};1$v").as_bytes());
        parser.process(format!("\x1b[2;{col}Hx").as_bytes());
        parser.screen_mut().set_size(3, 1);
        parser.process(b"\x1b[1;1Hx");
        assert_eq!(parser.screen().cell(0, 0).unwrap().contents(), "x");
    }
}

#[test]
fn same_coordinate_shadow_copy_is_harmless_when_alternate_pages_alias() {
    let mut parser = vt100::Parser::new(6, 30, 0);
    parser.process(b"\x1b[?1049hHISTORY-ONE\x1b[4;1HHISTORY-FOUR\x1b[6;1HPROMPT> ");
    let before = parser.screen().clone();
    parser.process(b"\x1b[3;1;5;30;1;3;1;6$v\x1b[3;1;5;30;6;3;1;1$v");
    assert_eq!(parser.screen().cursor_position(), before.cursor_position());
    for row in 0..6 {
        for col in 0..30 {
            assert_eq!(parser.screen().cell(row, col), before.cell(row, col));
        }
    }
}
