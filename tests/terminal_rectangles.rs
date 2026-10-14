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

#[test]
fn background_page_access_after_shrink_discards_clipped_history() {
    // Match Windows Terminal's PageManager: an inactive page is resized
    // traditionally when accessed, not reflowed with the visible buffer.
    let mut parser = vt100::Parser::new(8, 80, 0);
    parser
        .process(b"\x1b[38;2;123;45;67mKEPT\x1b[1;65HRIGHT-HISTORY\x1b[7;1HBOTTOM-HISTORY\x1b[0m");
    let before = parser.screen().clone();
    parser.process(b"\x1b[1;1;8;80;1;1;1;6$v");
    parser.screen_mut().set_size(3, 40);
    // Access during shrink is the destructive operation. Growing again
    // cannot recover cells already discarded from the background page.
    parser.process(b"\x1b[1;1;3;40;6;1;1;1$v");
    assert_eq!(parser.screen().cell(0, 0), before.cell(0, 0));
    parser.screen_mut().set_size(8, 80);
    parser.process(b"\x1b[1;1;8;80;6;1;1;1$v");
    assert!(!parser.screen().contents().contains("RIGHT-HISTORY"));
    assert!(!parser.screen().contents().contains("BOTTOM-HISTORY"));
}
#[test]
fn ansi_saved_cursor_restores_prompt_after_menu() {
    let mut parser = vt100::Parser::new(12, 40, 0);
    parser.process(b"PS> g\x1b[s\x1b[3;1HMENU\x1b[uit switch topic");
    assert_eq!(
        parser.screen().cursor_position(),
        (0, "PS> git switch topic".len() as u16)
    );
    let rows: Vec<_> = parser.screen().rows(0, 40).collect();
    assert_eq!(rows[0].trim_end(), "PS> git switch topic");
    assert_eq!(rows[2].trim_end(), "MENU");
}

#[test]
fn fragmented_ansi_saved_cursor_preserves_attributes_and_pending_wrap() {
    let mut parser = vt100::Parser::new(12, 5, 0);
    for byte in b"\x1b[38;2;17;123;239mABCDE\x1b[s\x1b[4;1H\x1b[31mMENU\x1b[uZ" {
        parser.process(&[*byte]);
    }
    assert_eq!(parser.screen().cursor_position(), (1, 1));
    let cell = parser.screen().cell(1, 0).unwrap();
    assert_eq!(cell.contents(), "Z");
    assert_eq!(cell.fgcolor(), vt100::Color::Rgb(17, 123, 239));
}
