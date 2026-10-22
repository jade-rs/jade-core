//! Word's 表示 tab and Word のオプション as both shells lay them out: the
//! page boxes Web レイアウト, 閲覧モード and アウトライン lay the document
//! out in, where the pages sit for 横, 複数ページ and one under another,
//! アウトライン's levels and indents, and the quick access toolbar's lists.

use std::future::Future;

use jade_docx_editor::word::customize::{
    format_commands, format_keyed, parse_commands, parse_keyed, QuickCommand, DEFAULT_TOOLBAR,
};
use jade_docx_editor::word::outline::{
    hidden_by_level, indents_of, levels, promoted_body_level, symbol, BODY_LEVEL, LEVEL_STEP,
    SYMBOL_ROOM,
};
use jade_docx_editor::word::view::{
    geometry_close, view_geometry, PageGrid, ViewKind, READ_MAX_COLUMN, WEB_STRIP_HEIGHT,
};
use jade_docx_editor::word_document::WordDocument;
use jade_docx_editor::Options;

fn block_on<F: Future>(future: F) -> F::Output {
    let mut future = std::pin::pin!(future);
    let mut context = std::task::Context::from_waker(std::task::Waker::noop());
    loop {
        if let std::task::Poll::Ready(value) = future.as_mut().poll(&mut context) {
            return value;
        }
    }
}

#[test]
fn web_layout_and_read_mode_follow_the_window() {
    let web = view_geometry(ViewKind::Web, (1200.0, 800.0), 1.0).unwrap();
    assert_eq!(web.page_height, WEB_STRIP_HEIGHT);
    assert_eq!(web.page_width, 1200.0 - 64.0);
    let zoomed = view_geometry(ViewKind::Web, (1200.0, 800.0), 2.0).unwrap();
    assert!(
        zoomed.page_width < web.page_width / 1.9,
        "zoomed in, fewer px"
    );
    let read = view_geometry(ViewKind::Read, (1600.0, 900.0), 1.0).unwrap();
    assert!(read.page_width <= READ_MAX_COLUMN);
    assert!(read.page_height < 900.0);
    assert!(view_geometry(ViewKind::Print, (1200.0, 800.0), 1.0).is_none());
    assert!(view_geometry(ViewKind::Draft, (1200.0, 800.0), 1.0).is_none());
    // A pixel wider is not worth laying the document out again for.
    let wider = view_geometry(ViewKind::Web, (1202.0, 800.0), 1.0);
    assert!(geometry_close(Some(web), wider));
    let much_wider = view_geometry(ViewKind::Web, (1400.0, 800.0), 1.0);
    assert!(!geometry_close(Some(web), much_wider));
    assert!(!geometry_close(Some(web), None));
}

#[test]
fn pages_are_found_where_they_are_set_out() {
    // One under another: page 3 starts three pitches down.
    let column = PageGrid {
        page: (600.0, 800.0),
        gap: 24.0,
        padding: 32.0,
        per_row: 1,
        horizontal: false,
    };
    assert_eq!(column.origin(3), (32.0, 32.0 + 3.0 * 824.0));
    assert_eq!(column.visible((0.0, 0.0), (900.0, 700.0), 10), 0..1);
    assert_eq!(column.visible((0.0, 800.0), (900.0, 700.0), 10), 0..2);
    assert_eq!(column.current((0.0, 1700.0), (900.0, 600.0), 10), 2);
    // 複数ページ: two across, rows of two.
    let rows = PageGrid {
        per_row: 2,
        ..column
    };
    assert_eq!(rows.origin(3), (32.0 + 624.0, 32.0 + 824.0));
    assert_eq!(rows.visible((0.0, 0.0), (1400.0, 900.0), 5), 0..4);
    assert_eq!(rows.current((0.0, 900.0), (1400.0, 600.0), 5), 2);
    // 横: one row, scrolled sideways.
    let side = PageGrid {
        horizontal: true,
        ..column
    };
    assert_eq!(side.origin(2), (32.0 + 2.0 * 624.0, 32.0));
    assert_eq!(side.visible((700.0, 0.0), (800.0, 900.0), 6), 1..3);
    assert_eq!(side.current((1300.0, 0.0), (900.0, 900.0), 6), 2);
    // Past the end, the last page.
    assert_eq!(side.current((99999.0, 0.0), (900.0, 900.0), 6), 5);
}

#[test]
fn the_outline_sets_headings_in_by_level() {
    let levels = [(0, 1), (1, BODY_LEVEL), (2, 2), (3, BODY_LEVEL), (4, 1)];
    let indents = indents_of(&levels);
    assert_eq!(indents[&0], SYMBOL_ROOM);
    assert_eq!(indents[&1], SYMBOL_ROOM + LEVEL_STEP, "under 見出し 1");
    assert_eq!(indents[&2], SYMBOL_ROOM + LEVEL_STEP);
    assert_eq!(
        indents[&3],
        SYMBOL_ROOM + 2.0 * LEVEL_STEP,
        "under 見出し 2"
    );
    assert_eq!(indents[&4], SYMBOL_ROOM);
    assert_eq!(promoted_body_level(&levels, 3), 2);
    assert_eq!(promoted_body_level(&levels, 0), 1);
    assert_eq!(symbol(1, Some(BODY_LEVEL), false), "⊕");
    assert_eq!(symbol(2, Some(1), false), "⊖");
    assert_eq!(symbol(2, None, true), "⊕");
    assert_eq!(symbol(BODY_LEVEL, Some(1), false), "○");
}

#[test]
fn the_outline_reads_a_documents_levels() {
    let bytes = std::fs::read(concat!(env!("CARGO_MANIFEST_DIR"), "/../../contract.docx")).unwrap();
    let document = block_on(WordDocument::load(&bytes, &Options::default())).unwrap();
    let all = levels(&document);
    assert!(!all.is_empty());
    assert!(all
        .iter()
        .all(|(_, level)| (1..=BODY_LEVEL).contains(level)));
    // Showing every level hides nothing; showing only 見出し 1 hides the
    // body text.
    assert!(hidden_by_level(&document, BODY_LEVEL).is_empty());
    assert!(hidden_by_level(&document, 0).is_empty());
    let body = all.iter().filter(|(_, level)| *level > 1).count();
    assert_eq!(!hidden_by_level(&document, 1).is_empty(), body > 0);
}

#[test]
fn the_toolbar_lists_round_trip() {
    let saved = format_commands(&[QuickCommand::Print, QuickCommand::Bold]);
    assert_eq!(saved, "print,bold");
    assert_eq!(
        parse_commands(&saved),
        [QuickCommand::Print, QuickCommand::Bold]
    );
    assert_eq!(
        parse_commands("save, nonsense ,undo"),
        [QuickCommand::Save, QuickCommand::Undo]
    );
    assert_eq!(DEFAULT_TOOLBAR[0], QuickCommand::Save);
    for command in QuickCommand::ALL {
        assert_eq!(QuickCommand::from_id(command.id()), Some(command));
    }
    let mut values = parse_keyed("qat=save\ntab.Design=hidden\n");
    assert_eq!(values["tab.Design"], "hidden");
    values.insert("custom_group".into(), "bold".into());
    assert_eq!(
        format_keyed(&values),
        "custom_group=bold\nqat=save\ntab.Design=hidden\n"
    );
}
