//! The menu a right-click (or ⌃-click) opens, as Word for Mac lays it out
//! for what is under the pointer: text, a table, a picture or a shape, a
//! header or footer, a link, a field. Both shells draw it from this list
//! and run each item through their own commands.

/// What a right-click landed on; several can hold at once (a link in a
/// table cell).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ContextTarget {
    pub selection: bool,
    pub table: bool,
    pub picture: bool,
    pub shape: bool,
    pub header_footer: bool,
    pub link: bool,
    pub field: bool,
    pub comment: bool,
}

/// What a context menu item does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ContextCommand {
    Cut,
    Copy,
    Paste,
    /// 貼り付けのオプション: 元の書式を保持 / 書式を結合 / テキストのみ保持.
    PasteKeepSource,
    PasteMerge,
    PasteTextOnly,
    /// 形式を選択して貼り付け….
    PasteSpecial,
    Font,
    Paragraph,
    /// 箇条書きと段落番号.
    Bullets,
    Numbering,
    /// スマート検索: the selection in the find bar.
    SmartLookup,
    NewComment,
    Link,
    EditLink,
    OpenLink,
    RemoveLink,
    UpdateField,
    EditField,
    ToggleFieldCodes,
    InsertRowAbove,
    InsertRowBelow,
    InsertColumnLeft,
    InsertColumnRight,
    InsertCells,
    DeleteCells,
    DeleteRow,
    DeleteColumn,
    DeleteTable,
    MergeCells,
    SplitCells,
    DistributeRows,
    DistributeColumns,
    TableBorders,
    TextDirection,
    AutoFitContents,
    AutoFitWindow,
    TableProperties,
    /// 文字列の折り返し: 行内 / 四角形 / 狭く / 内部 / 上下 / 背面 / 前面.
    WrapInline,
    WrapSquare,
    WrapTight,
    WrapThrough,
    WrapTopBottom,
    WrapBehind,
    WrapInFront,
    EditHeader,
    EditFooter,
    CloseHeaderFooter,
}

/// One row of the menu: a caption over a group, or an item.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ContextEntry {
    /// A group's heading (挿入, 削除, 文字列の折り返し).
    Caption(&'static str),
    Item(&'static str, ContextCommand),
    Separator,
}

impl ContextEntry {
    pub fn command(self) -> Option<ContextCommand> {
        match self {
            ContextEntry::Item(_, command) => Some(command),
            _ => None,
        }
    }
}

use ContextCommand as C;
use ContextEntry::{Caption, Item, Separator};

/// The menu for `target`, in Word's order. Labels are i18n keys.
pub fn menu(target: ContextTarget) -> Vec<ContextEntry> {
    let mut entries = Vec::new();
    if target.picture || target.shape {
        entries.extend([
            Item("context.cut", C::Cut),
            Item("context.copy", C::Copy),
            Item("context.paste", C::Paste),
            Separator,
            Caption("context.wrap"),
            Item("picture.wrap.inline", C::WrapInline),
            Item("picture.wrap.square", C::WrapSquare),
            Item("picture.wrap.tight", C::WrapTight),
            Item("picture.wrap.through", C::WrapThrough),
            Item("picture.wrap.top_bottom", C::WrapTopBottom),
            Item("picture.wrap.behind", C::WrapBehind),
            Item("picture.wrap.front", C::WrapInFront),
            Separator,
            Item("context.link", C::Link),
            Item("context.new_comment", C::NewComment),
        ]);
        return entries;
    }
    if target.header_footer {
        entries.extend([
            Item("context.cut", C::Cut),
            Item("context.copy", C::Copy),
            Item("context.paste", C::Paste),
            Separator,
            Item("context.font", C::Font),
            Item("context.paragraph", C::Paragraph),
            Separator,
            Item("context.edit_header", C::EditHeader),
            Item("context.edit_footer", C::EditFooter),
            Item("context.close_header_footer", C::CloseHeaderFooter),
        ]);
        return entries;
    }
    entries.extend([
        Item("context.cut", C::Cut),
        Item("context.copy", C::Copy),
        Item("context.paste", C::Paste),
        Caption("context.paste_options"),
        Item("context.paste_keep", C::PasteKeepSource),
        Item("context.paste_merge", C::PasteMerge),
        Item("context.paste_text", C::PasteTextOnly),
        Item("context.paste_special", C::PasteSpecial),
        Separator,
    ]);
    if target.field {
        entries.extend([
            Item("context.update_field", C::UpdateField),
            Item("context.edit_field", C::EditField),
            Item("context.toggle_field_codes", C::ToggleFieldCodes),
            Separator,
        ]);
    }
    entries.extend([
        Item("context.font", C::Font),
        Item("context.paragraph", C::Paragraph),
        Item("context.bullets", C::Bullets),
        Item("context.numbering", C::Numbering),
    ]);
    if target.table {
        entries.extend([
            Separator,
            Caption("context.insert"),
            Item("context.row_above", C::InsertRowAbove),
            Item("context.row_below", C::InsertRowBelow),
            Item("context.column_left", C::InsertColumnLeft),
            Item("context.column_right", C::InsertColumnRight),
            Item("context.insert_cells", C::InsertCells),
            Caption("context.delete"),
            Item("context.delete_cells", C::DeleteCells),
            Item("context.delete_row", C::DeleteRow),
            Item("context.delete_column", C::DeleteColumn),
            Item("context.delete_table", C::DeleteTable),
            Separator,
            Item("context.merge_cells", C::MergeCells),
            Item("context.split_cells", C::SplitCells),
            Item("context.distribute_rows", C::DistributeRows),
            Item("context.distribute_columns", C::DistributeColumns),
            Item("context.table_borders", C::TableBorders),
            Item("context.text_direction", C::TextDirection),
            Caption("context.autofit"),
            Item("context.autofit_contents", C::AutoFitContents),
            Item("context.autofit_window", C::AutoFitWindow),
            Item("context.table_properties", C::TableProperties),
        ]);
    }
    entries.push(Separator);
    if target.link {
        entries.extend([
            Item("context.edit_link", C::EditLink),
            Item("context.open_link", C::OpenLink),
            Item("context.remove_link", C::RemoveLink),
        ]);
    } else {
        entries.push(Item("context.link", C::Link));
    }
    if target.selection {
        entries.push(Item("context.smart_lookup", C::SmartLookup));
    }
    entries.push(Item("context.new_comment", C::NewComment));
    entries
}

#[cfg(test)]
mod tests {
    use super::*;

    fn commands(target: ContextTarget) -> Vec<ContextCommand> {
        menu(target)
            .into_iter()
            .filter_map(ContextEntry::command)
            .collect()
    }

    #[test]
    fn each_target_has_word_s_items() {
        let text = commands(ContextTarget::default());
        assert!(text.contains(&C::Cut) && text.contains(&C::Font) && text.contains(&C::Link));
        assert!(!text.contains(&C::InsertRowAbove));
        let table = commands(ContextTarget {
            table: true,
            ..ContextTarget::default()
        });
        assert!(table.contains(&C::InsertRowAbove) && table.contains(&C::TableProperties));
        let picture = commands(ContextTarget {
            picture: true,
            ..ContextTarget::default()
        });
        assert!(picture.contains(&C::WrapSquare) && !picture.contains(&C::Font));
        let link = commands(ContextTarget {
            link: true,
            ..ContextTarget::default()
        });
        assert!(link.contains(&C::OpenLink) && !link.contains(&C::Link));
        let header = commands(ContextTarget {
            header_footer: true,
            ..ContextTarget::default()
        });
        assert!(header.contains(&C::CloseHeaderFooter));
        let field = commands(ContextTarget {
            field: true,
            ..ContextTarget::default()
        });
        assert!(field.contains(&C::UpdateField));
    }
}
