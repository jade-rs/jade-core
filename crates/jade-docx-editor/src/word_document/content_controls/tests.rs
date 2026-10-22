use super::*;
use crate::commands::{CommandContext, CommandExecutor};
use crate::headless_api::apply_headless_commands_with_executor_to_document;
use jade_docx_headless_api::{HeadlessCommand, HeadlessDeleteDirection, HeadlessEditRequest};
use std::io::Write;
use zip::write::SimpleFileOptions;

const NAMESPACES: &str = r#"xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main" xmlns:w14="http://schemas.microsoft.com/office/word/2010/wordml" xmlns:w15="http://schemas.microsoft.com/office/word/2012/wordml" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships""#;

pub(crate) fn package(body: &str, settings: &str) -> Vec<u8> {
    let content_types = r#"<?xml version="1.0" encoding="UTF-8"?>
<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">
  <Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/>
  <Default Extension="xml" ContentType="application/xml"/>
  <Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/>
  <Override PartName="/word/styles.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.styles+xml"/>
  <Override PartName="/word/settings.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.settings+xml"/>
</Types>"#;
    let root_rels = r#"<?xml version="1.0" encoding="UTF-8"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
  <Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/>
</Relationships>"#;
    let document_rels = r#"<?xml version="1.0" encoding="UTF-8"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
  <Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/styles" Target="styles.xml"/>
  <Relationship Id="rId2" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/settings" Target="settings.xml"/>
</Relationships>"#;
    let styles = r#"<?xml version="1.0" encoding="UTF-8"?>
<w:styles xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:style w:type="paragraph" w:default="1" w:styleId="Normal"><w:name w:val="Normal"/></w:style><w:style w:type="character" w:default="1" w:styleId="DefaultParagraphFont"><w:name w:val="Default Paragraph Font"/></w:style></w:styles>"#;
    let settings = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<w:settings xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">{settings}<w:defaultTabStop w:val="840"/></w:settings>"#
    );
    let document = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<w:document {NAMESPACES}><w:body>{body}<w:sectPr><w:pgSz w:w="11906" w:h="16838"/><w:pgMar w:top="1985" w:right="1701" w:bottom="1701" w:left="1701" w:header="851" w:footer="992" w:gutter="0"/></w:sectPr></w:body></w:document>"#
    );
    let mut cursor = std::io::Cursor::new(Vec::new());
    {
        let mut zip = zip::ZipWriter::new(&mut cursor);
        let options = SimpleFileOptions::default();
        for (path, content) in [
            ("[Content_Types].xml", content_types),
            ("_rels/.rels", root_rels),
            ("word/_rels/document.xml.rels", document_rels),
            ("word/styles.xml", styles),
            ("word/settings.xml", settings.as_str()),
            ("word/document.xml", document.as_str()),
        ] {
            zip.start_file(path, options).expect("entry");
            zip.write_all(content.as_bytes()).expect("write");
        }
        zip.finish().expect("finish");
    }
    cursor.into_inner()
}

pub(crate) fn open(body: &str, settings: &str) -> WordDocument {
    super::super::rewrite::run_ready(WordDocument::load(
        &package(body, settings),
        &Options::default(),
    ))
    .expect("load")
}

fn text_of(document: &WordDocument, paragraph: usize) -> String {
    document
        .paragraph(paragraph)
        .map(|paragraph| paragraph.runs.iter().map(|run| run.text.as_str()).collect())
        .unwrap_or_default()
}

fn document_xml(document: &WordDocument) -> String {
    document
        .raw_document
        .as_ref()
        .expect("xml")
        .to_string()
        .expect("serialize")
}

fn run(
    document: &mut WordDocument,
    executor: &mut CommandExecutor,
    action: ContentControlAction,
    range: SelectionRange,
) -> Result<Option<SelectionRange>, String> {
    let mut selection = Some(range);
    let mut ctx = CommandContext::new(document, &mut selection);
    executor
        .execute(Box::new(action.command(range)), &mut ctx)
        .map_err(|error| error.to_string())?;
    Ok(selection)
}

fn at(paragraph: usize, offset: usize) -> SelectionRange {
    SelectionRange::collapsed(TextPosition::new(paragraph, offset))
}

fn type_text(
    document: &mut WordDocument,
    paragraph: usize,
    offset: usize,
    text: &str,
) -> Result<(), String> {
    let mut executor = CommandExecutor::new();
    apply_headless_commands_with_executor_to_document(
        document,
        &mut executor,
        None,
        &HeadlessEditRequest {
            commands: vec![HeadlessCommand::InsertText {
                paragraph,
                offset,
                text: text.to_string(),
            }],
        },
    )
    .map(|_| ())
    .map_err(|error| error.to_string())
}

const CONTROLS: &str = r#"<w:p><w:r><w:t xml:space="preserve">Name: </w:t></w:r><w:sdt><w:sdtPr><w:alias w:val="Full name"/><w:tag w:val="name"/><w:id w:val="11"/><w:lock w:val="sdtLocked"/><w:text/></w:sdtPr><w:sdtContent><w:r><w:t>Taro</w:t></w:r></w:sdtContent></w:sdt><w:r><w:t xml:space="preserve"> ok</w:t></w:r></w:p>
<w:p><w:sdt><w:sdtPr><w:id w:val="12"/><w14:checkbox><w14:checked w14:val="1"/><w14:checkedState w14:val="2612" w14:font="MS Gothic"/><w14:uncheckedState w14:val="2610" w14:font="MS Gothic"/></w14:checkbox></w:sdtPr><w:sdtContent><w:r><w:t>☒</w:t></w:r></w:sdtContent></w:sdt><w:r><w:t xml:space="preserve"> agree</w:t></w:r></w:p>
<w:p><w:sdt><w:sdtPr><w:id w:val="13"/><w:lock w:val="contentLocked"/><w:dropDownList><w:listItem w:displayText="Red" w:value="r"/><w:listItem w:displayText="Blue" w:value="b"/></w:dropDownList></w:sdtPr><w:sdtContent><w:r><w:t>Red</w:t></w:r></w:sdtContent></w:sdt></w:p>
<w:sdt><w:sdtPr><w:id w:val="14"/><w15:repeatingSection/></w:sdtPr><w:sdtContent><w:sdt><w:sdtPr><w:id w:val="15"/><w15:repeatingSectionItem/></w:sdtPr><w:sdtContent><w:p><w:r><w:t>Item</w:t></w:r></w:p></w:sdtContent></w:sdt></w:sdtContent></w:sdt>
<w:p><w:r><w:t>End</w:t></w:r></w:p>"#;

#[test]
fn reads_controls_where_their_runs_and_paragraphs_are() {
    let document = open(CONTROLS, "");
    let controls = document.content_controls();
    assert_eq!(controls.len(), 5, "{controls:#?}");
    let name = &controls[0];
    assert_eq!(name.kind, ContentControlKind::PlainText);
    assert_eq!(name.title.as_deref(), Some("Full name"));
    assert_eq!(name.tag.as_deref(), Some("name"));
    assert!(name.cannot_delete && !name.cannot_edit);
    assert_eq!(
        (name.start, name.end),
        (TextPosition::new(0, 6), TextPosition::new(0, 10))
    );
    assert_eq!(name.text, "Taro");
    assert!(!name.block);

    let check = &controls[1];
    assert_eq!(check.kind, ContentControlKind::CheckBox);
    assert!(check.checked);
    assert_eq!(
        check.checked_symbol.as_ref().map(|symbol| symbol.symbol),
        Some('☒')
    );

    let list = &controls[2];
    assert_eq!(list.kind, ContentControlKind::DropDownList);
    assert_eq!(list.entries.len(), 2);
    assert_eq!(list.entries[1].display, "Blue");
    assert!(list.cannot_edit);

    let section = &controls[3];
    assert_eq!(section.kind, ContentControlKind::RepeatingSection);
    assert!(section.block);
    assert_eq!(section.start.paragraph, 3);
    let item = &controls[4];
    assert_eq!(item.kind, ContentControlKind::RepeatingSectionItem);
    assert_eq!(item.parent, Some(3));
    assert_eq!(item.depth, 1);
    assert_eq!(item.text, "Item");

    let at_name = document
        .content_control_at(TextPosition::new(0, 8))
        .expect("in the name");
    assert_eq!(at_name.index, 0);
    let in_item = document
        .content_control_at(TextPosition::new(3, 2))
        .expect("in the item");
    assert_eq!(in_item.index, 4);
}

#[test]
fn locks_refuse_typing_and_deleting() {
    let mut document = open(CONTROLS, "");
    // Contents cannot be edited: the drop-down's text.
    assert!(type_text(&mut document, 2, 1, "x").is_err());
    // The name can be typed in...
    type_text(&mut document, 0, 8, "-").expect("the name takes typing");
    assert_eq!(text_of(&document, 0), "Name: Ta-ro ok");
    // ...but not deleted with the text around it.
    let mut executor = CommandExecutor::new();
    let refused = apply_headless_commands_with_executor_to_document(
        &mut document,
        &mut executor,
        None,
        &HeadlessEditRequest {
            commands: vec![HeadlessCommand::ReplaceRange {
                start_paragraph: 0,
                start_offset: 4,
                end_paragraph: 0,
                end_offset: 13,
                text: String::new(),
            }],
        },
    );
    assert!(refused.is_err());
    // A check box takes no typing; Backspace over it is refused too.
    assert!(type_text(&mut document, 1, 1, "x").is_err());
    let backspace = apply_headless_commands_with_executor_to_document(
        &mut document,
        &mut executor,
        None,
        &HeadlessEditRequest {
            commands: vec![HeadlessCommand::DeleteText {
                paragraph: 2,
                offset: 2,
                direction: HeadlessDeleteDirection::Backward,
                count: 1,
            }],
        },
    );
    assert!(backspace.is_err());
    // Plain text: Return is refused.
    let enter = apply_headless_commands_with_executor_to_document(
        &mut document,
        &mut executor,
        None,
        &HeadlessEditRequest {
            commands: vec![HeadlessCommand::InsertParagraphBreak {
                paragraph: 0,
                offset: 8,
                count: 1,
            }],
        },
    );
    assert!(enter.is_err());
}

#[test]
fn inserts_a_rich_text_control_with_its_placeholder_selected() {
    let mut document = open(r#"<w:p><w:r><w:t>Hello world</w:t></w:r></w:p>"#, "");
    let mut executor = CommandExecutor::new();
    let selection = run(
        &mut document,
        &mut executor,
        ContentControlAction::Insert {
            kind: ContentControlKind::RichText,
            japanese: true,
        },
        at(0, 5),
    )
    .expect("insert");
    let controls = document.content_controls();
    assert_eq!(controls.len(), 1);
    let control = &controls[0];
    assert!(control.showing_placeholder);
    assert_eq!(
        control.text,
        "ここをクリックまたはタップしてテキストを入力してください。"
    );
    assert_eq!(
        selection,
        Some(SelectionRange::new(control.start, control.end))
    );
    assert_eq!(
        text_of(&document, 0).len(),
        "Hello world".len() + control.text.len()
    );
    let xml = document_xml(&document);
    assert!(xml.contains("w:showingPlcHdr"), "{xml}");
    assert!(xml.contains("PlaceholderText"), "{xml}");
    let styles = document.read_xml_part_content("word/styles.xml").unwrap();
    assert!(styles.contains("Placeholder Text"));

    // Typing over the placeholder: its text, out of the placeholder style.
    let selection = run(
        &mut document,
        &mut executor,
        ContentControlAction::SetText {
            control: 0,
            text: "X".to_string(),
        },
        selection.unwrap(),
    )
    .expect("type over");
    assert_eq!(text_of(&document, 0), "HelloX world");
    let control = &document.content_controls()[0];
    assert!(!control.showing_placeholder);
    assert_eq!(
        selection,
        Some(SelectionRange::collapsed(TextPosition::new(0, 6)))
    );
    assert!(!document_xml(&document)
        .contains("PlaceholderText\"/></w:rPr><w:t xml:space=\"preserve\">X"));

    // Undo takes both back.
    let mut selection = None;
    let mut ctx = CommandContext::new(&mut document, &mut selection);
    executor.undo(&mut ctx).expect("undo");
    executor.undo(&mut ctx).expect("undo");
    assert_eq!(text_of(&document, 0), "Hello world");
    assert!(document.content_controls().is_empty());
}

#[test]
fn wraps_the_selection_in_a_control() {
    let mut document = open(
        r#"<w:p><w:r><w:rPr><w:b/></w:rPr><w:t>Hello world</w:t></w:r></w:p>"#,
        "",
    );
    let mut executor = CommandExecutor::new();
    run(
        &mut document,
        &mut executor,
        ContentControlAction::Insert {
            kind: ContentControlKind::PlainText,
            japanese: false,
        },
        SelectionRange::new(TextPosition::new(0, 6), TextPosition::new(0, 11)),
    )
    .expect("wrap");
    assert_eq!(text_of(&document, 0), "Hello world");
    let control = &document.content_controls()[0];
    assert_eq!(control.kind, ContentControlKind::PlainText);
    assert_eq!(control.text, "world");
    assert!(!control.showing_placeholder);
    // Both halves keep the bold.
    assert_eq!(document_xml(&document).matches("<w:b/>").count(), 2);
}

#[test]
fn ticks_a_check_box_and_picks_entries_and_dates() {
    let mut document = open(r#"<w:p><w:r><w:t>A</w:t></w:r></w:p>"#, "");
    let mut executor = CommandExecutor::new();
    run(
        &mut document,
        &mut executor,
        ContentControlAction::Insert {
            kind: ContentControlKind::CheckBox,
            japanese: true,
        },
        at(0, 1),
    )
    .expect("check box");
    assert_eq!(text_of(&document, 0), "A☐");
    run(
        &mut document,
        &mut executor,
        ContentControlAction::SetChecked {
            control: 0,
            checked: true,
        },
        at(0, 0),
    )
    .expect("tick");
    assert_eq!(text_of(&document, 0), "A☒");
    assert!(document.content_controls()[0].checked);

    run(
        &mut document,
        &mut executor,
        ContentControlAction::Insert {
            kind: ContentControlKind::DropDownList,
            japanese: true,
        },
        at(0, 0),
    )
    .expect("list");
    let list = document.content_controls()[0].clone();
    assert_eq!(list.kind, ContentControlKind::DropDownList);
    let mut properties = ContentControlProperties::of(&list);
    properties.title = "Colour".to_string();
    properties.entries.push(ListEntry {
        display: "Green".to_string(),
        value: "g".to_string(),
    });
    properties.cannot_delete = true;
    run(
        &mut document,
        &mut executor,
        ContentControlAction::SetProperties {
            control: 0,
            properties,
        },
        at(0, 0),
    )
    .expect("properties");
    let list = document.content_controls()[0].clone();
    assert_eq!(list.title.as_deref(), Some("Colour"));
    assert!(list.cannot_delete);
    assert_eq!(list.entries.len(), 2);
    assert!(list.showing_placeholder);
    run(
        &mut document,
        &mut executor,
        ContentControlAction::ChooseEntry {
            control: 0,
            entry: 1,
        },
        at(0, 0),
    )
    .expect("choose");
    assert!(text_of(&document, 0).starts_with("Green"));
    assert!(!document.content_controls()[0].showing_placeholder);

    run(
        &mut document,
        &mut executor,
        ContentControlAction::Insert {
            kind: ContentControlKind::Date,
            japanese: true,
        },
        at(0, 0),
    )
    .expect("date");
    run(
        &mut document,
        &mut executor,
        ContentControlAction::SetDate {
            control: 0,
            date: "2026-10-21".to_string(),
        },
        at(0, 0),
    )
    .expect("pick a date");
    assert!(
        text_of(&document, 0).starts_with("2026/10/21"),
        "{}",
        text_of(&document, 0)
    );
    assert_eq!(
        document.content_controls()[0].full_date.as_deref(),
        Some("2026-10-21T00:00:00Z"),
        "{}",
        document_xml(&document)
    );
    assert_eq!(
        document.content_controls()[0].full_date.as_deref(),
        Some("2026-10-21T00:00:00Z")
    );
}

#[test]
fn a_picture_control_shows_a_placeholder_picture_until_one_is_chosen() {
    let mut document = open(r#"<w:p><w:r><w:t>A</w:t></w:r></w:p>"#, "");
    let mut executor = CommandExecutor::new();
    run(
        &mut document,
        &mut executor,
        ContentControlAction::Insert {
            kind: ContentControlKind::Picture,
            japanese: true,
        },
        at(0, 1),
    )
    .expect("picture");
    let control = document.content_controls()[0].clone();
    assert_eq!(control.kind, ContentControlKind::Picture);
    let has_image = |document: &WordDocument| {
        document
            .paragraph(0)
            .unwrap()
            .runs
            .iter()
            .any(|run| run.image.is_some())
    };
    assert!(has_image(&document));
    let picture = edit::placeholder_picture_png();
    run(
        &mut document,
        &mut executor,
        ContentControlAction::SetPicture {
            control: 0,
            bytes: picture,
            mime: "image/png".to_string(),
        },
        at(0, 0),
    )
    .expect("choose a picture");
    assert!(has_image(&document));
    let exported = document.export_docx().expect("export");
    let reopened =
        super::super::rewrite::run_ready(WordDocument::load(&exported, &Options::default()))
            .unwrap();
    assert_eq!(
        reopened.content_controls()[0].kind,
        ContentControlKind::Picture
    );
}

#[test]
fn repeating_sections_add_and_remove_items() {
    let mut document = open(CONTROLS, "");
    let mut executor = CommandExecutor::new();
    run(
        &mut document,
        &mut executor,
        ContentControlAction::AddRepeatingItem { control: 4 },
        at(3, 0),
    )
    .expect("add");
    let controls = document.content_controls();
    let items: Vec<_> = controls
        .iter()
        .filter(|control| control.kind == ContentControlKind::RepeatingSectionItem)
        .collect();
    assert_eq!(items.len(), 2);
    assert_ne!(items[0].id, items[1].id);
    assert_eq!(text_of(&document, 4), "Item");
    run(
        &mut document,
        &mut executor,
        ContentControlAction::RemoveRepeatingItem { control: 5 },
        at(3, 0),
    )
    .expect("remove");
    assert!(run(
        &mut document,
        &mut executor,
        ContentControlAction::RemoveRepeatingItem { control: 4 },
        at(3, 0),
    )
    .is_err());
}

#[test]
fn inserts_a_repeating_section_and_a_group_around_paragraphs() {
    let mut document = open(
        r#"<w:p><w:r><w:t>One</w:t></w:r></w:p><w:p><w:r><w:t>Two</w:t></w:r></w:p>"#,
        "",
    );
    let mut executor = CommandExecutor::new();
    run(
        &mut document,
        &mut executor,
        ContentControlAction::Insert {
            kind: ContentControlKind::Group,
            japanese: true,
        },
        SelectionRange::new(TextPosition::new(0, 0), TextPosition::new(1, 3)),
    )
    .expect("group");
    let group = document.content_controls()[0].clone();
    assert_eq!(group.kind, ContentControlKind::Group);
    assert!(group.block);
    // A group's text is fixed.
    assert!(type_text(&mut document, 0, 1, "x").is_err());
    run(
        &mut document,
        &mut executor,
        ContentControlAction::Remove { control: 0 },
        at(0, 0),
    )
    .expect("ungroup");
    type_text(&mut document, 0, 1, "x").expect("typing again");
    run(
        &mut document,
        &mut executor,
        ContentControlAction::Insert {
            kind: ContentControlKind::RepeatingSection,
            japanese: true,
        },
        SelectionRange::new(TextPosition::new(1, 0), TextPosition::new(1, 3)),
    )
    .expect("repeating section");
    let kinds: Vec<_> = document
        .content_controls()
        .into_iter()
        .map(|control| control.kind)
        .collect();
    assert_eq!(
        kinds,
        vec![
            ContentControlKind::RepeatingSection,
            ContentControlKind::RepeatingSectionItem
        ]
    );
}

const FORM: &str = r#"<w:p><w:r><w:t xml:space="preserve">Name </w:t></w:r><w:r><w:fldChar w:fldCharType="begin"><w:ffData><w:name w:val="Text1"/><w:enabled/><w:calcOnExit w:val="0"/><w:textInput/></w:ffData></w:fldChar></w:r><w:r><w:instrText xml:space="preserve"> FORMTEXT </w:instrText></w:r><w:r><w:fldChar w:fldCharType="separate"/></w:r><w:r><w:t>Taro</w:t></w:r><w:r><w:fldChar w:fldCharType="end"/></w:r></w:p>
<w:p><w:r><w:fldChar w:fldCharType="begin"><w:ffData><w:name w:val="Check1"/><w:enabled/><w:calcOnExit w:val="0"/><w:checkBox><w:sizeAuto/><w:default w:val="0"/></w:checkBox></w:ffData></w:fldChar></w:r><w:r><w:instrText xml:space="preserve"> FORMCHECKBOX </w:instrText></w:r><w:r><w:fldChar w:fldCharType="end"/></w:r><w:r><w:t xml:space="preserve"> yes</w:t></w:r></w:p>
<w:p><w:r><w:fldChar w:fldCharType="begin"><w:ffData><w:name w:val="Dropdown1"/><w:enabled/><w:calcOnExit w:val="0"/><w:ddList><w:listEntry w:val="Tokyo"/><w:listEntry w:val="Osaka"/></w:ddList></w:ffData></w:fldChar></w:r><w:r><w:instrText xml:space="preserve"> FORMDROPDOWN </w:instrText></w:r><w:r><w:fldChar w:fldCharType="separate"/></w:r><w:r><w:t>Tokyo</w:t></w:r><w:r><w:fldChar w:fldCharType="end"/></w:r></w:p>"#;

#[test]
fn reads_and_fills_legacy_form_fields() {
    let mut document = open(FORM, "");
    let fields = document.legacy_form_fields();
    assert_eq!(fields.len(), 3, "{fields:#?}");
    assert_eq!(fields[0].kind, FormFieldKind::Text);
    assert_eq!(fields[0].name, "Text1");
    assert_eq!(fields[0].result, "Taro");
    assert_eq!(fields[1].kind, FormFieldKind::CheckBox);
    assert!(!fields[1].checked);
    assert_eq!(fields[2].entries, vec!["Tokyo", "Osaka"]);
    // The check box draws its box.
    let check_text: Vec<String> = document
        .paragraph(1)
        .unwrap()
        .runs
        .iter()
        .filter_map(|run| run.field.as_ref()?.cached_text.clone())
        .collect();
    assert_eq!(check_text, vec!["☐".to_string()]);
    // The text box's result is text that can be typed.
    assert_eq!(text_of(&document, 0), "Name Taro");
    type_text(&mut document, 0, 9, "u").expect("fill in");
    assert_eq!(document.legacy_form_fields()[0].result, "Tarou");

    let mut executor = CommandExecutor::new();
    run(
        &mut document,
        &mut executor,
        ContentControlAction::SetFormCheckBox {
            field: 1,
            checked: true,
        },
        at(0, 0),
    )
    .expect("tick");
    assert!(document.legacy_form_fields()[1].checked);
    let ticked: Vec<String> = document
        .paragraph(1)
        .unwrap()
        .runs
        .iter()
        .filter_map(|run| run.field.as_ref()?.cached_text.clone())
        .collect();
    assert_eq!(ticked, vec!["☒".to_string()]);
    run(
        &mut document,
        &mut executor,
        ContentControlAction::ChooseFormEntry { field: 2, entry: 1 },
        at(0, 0),
    )
    .expect("choose");
    let fields = document.legacy_form_fields();
    assert_eq!(fields[2].selected, 1);
    assert_eq!(fields[2].result, "Osaka");
}

#[test]
fn protection_for_forms_allows_only_the_fields() {
    let mut document = open(FORM, "");
    let mut executor = CommandExecutor::new();
    run(
        &mut document,
        &mut executor,
        ContentControlAction::ProtectForms(true),
        at(0, 0),
    )
    .expect("protect");
    assert!(document.protected_for_forms());
    assert!(type_text(&mut document, 0, 1, "x").is_err());
    type_text(&mut document, 0, 7, "x").expect("in the field");
    run(
        &mut document,
        &mut executor,
        ContentControlAction::ProtectForms(false),
        at(0, 0),
    )
    .expect("unprotect");
    type_text(&mut document, 0, 1, "x").expect("anywhere again");
}

#[test]
fn inserts_legacy_form_fields_at_the_caret() {
    let mut document = open(r#"<w:p><w:r><w:t>Ab</w:t></w:r></w:p>"#, "");
    let mut executor = CommandExecutor::new();
    for kind in [
        FormFieldKind::Text,
        FormFieldKind::CheckBox,
        FormFieldKind::DropDown,
    ] {
        run(
            &mut document,
            &mut executor,
            ContentControlAction::InsertFormField { kind },
            at(0, 1),
        )
        .expect("insert a field");
    }
    let fields = document.legacy_form_fields();
    let kinds: Vec<_> = fields.iter().map(|field| field.kind).collect();
    assert_eq!(
        kinds,
        vec![
            FormFieldKind::DropDown,
            FormFieldKind::CheckBox,
            FormFieldKind::Text
        ]
    );
    assert_eq!(fields[2].name, "Text1");
    let mut options = edit::FormFieldOptions::of(&fields[0]);
    options.entries = vec!["One".to_string(), "Two".to_string()];
    run(
        &mut document,
        &mut executor,
        ContentControlAction::SetFormFieldOptions { field: 0, options },
        at(0, 0),
    )
    .expect("options");
    let fields = document.legacy_form_fields();
    assert_eq!(fields[0].entries, vec!["One", "Two"]);
    assert_eq!(fields[0].result, "One");
}

#[test]
fn design_mode_is_a_setting() {
    let mut document = open(r#"<w:p><w:r><w:t>A</w:t></w:r></w:p>"#, "");
    let mut executor = CommandExecutor::new();
    assert!(!document.forms_design_mode());
    run(
        &mut document,
        &mut executor,
        ContentControlAction::DesignMode(true),
        at(0, 0),
    )
    .unwrap();
    assert!(document.forms_design_mode());
    run(
        &mut document,
        &mut executor,
        ContentControlAction::DesignMode(false),
        at(0, 0),
    )
    .unwrap();
    assert!(!document.forms_design_mode());
}

#[test]
fn typing_over_an_inserted_form_text_box_fills_it() {
    let mut document = open(
        r#"<w:p><w:r><w:t xml:space="preserve">Name: </w:t></w:r></w:p>"#,
        "",
    );
    let mut executor = CommandExecutor::new();
    run(
        &mut document,
        &mut executor,
        ContentControlAction::InsertFormField {
            kind: FormFieldKind::Text,
        },
        at(0, 6),
    )
    .expect("insert");
    // A check box at the end of the text box's result goes after the field.
    let end = document.legacy_form_fields()[0].result_end;
    run(
        &mut document,
        &mut executor,
        ContentControlAction::InsertFormField {
            kind: FormFieldKind::CheckBox,
        },
        SelectionRange::collapsed(end),
    )
    .expect("insert a check box");
    let kinds: Vec<_> = document
        .legacy_form_fields()
        .iter()
        .map(|field| field.kind)
        .collect();
    assert_eq!(kinds, vec![FormFieldKind::Text, FormFieldKind::CheckBox]);
    let field = document.legacy_form_fields().remove(0);
    assert_eq!(field.result, forms::EMPTY_TEXT_FIELD, "{field:#?}");
    run(
        &mut document,
        &mut executor,
        ContentControlAction::SetFormText {
            field: 0,
            text: "Hanako".to_string(),
        },
        at(0, 6),
    )
    .expect("fill");
    assert_eq!(
        document.legacy_form_fields()[0].result,
        "Hanako",
        "{}",
        document_xml(&document)
    );
    assert_eq!(text_of(&document, 0), "Name: Hanako");
}

#[test]
fn a_headless_replacement_of_a_placeholder_types_into_the_control() {
    let mut document = open(r#"<w:p><w:r><w:t>Hello</w:t></w:r></w:p>"#, "");
    let mut executor = CommandExecutor::new();
    run(
        &mut document,
        &mut executor,
        ContentControlAction::Insert {
            kind: ContentControlKind::PlainText,
            japanese: false,
        },
        at(0, 5),
    )
    .expect("insert");
    let control = document.content_controls().remove(0);
    let selection = apply_headless_commands_with_executor_to_document(
        &mut document,
        &mut executor,
        Some(SelectionRange::new(control.start, control.end)),
        &HeadlessEditRequest {
            commands: vec![HeadlessCommand::ReplaceRange {
                start_paragraph: 0,
                start_offset: control.start.offset,
                end_paragraph: 0,
                end_offset: control.end.offset,
                text: "あ".to_string(),
            }],
        },
    )
    .expect("typed over");
    assert_eq!(
        selection,
        Some(SelectionRange::collapsed(TextPosition::new(0, 8)))
    );
    let control = document.content_controls().remove(0);
    assert!(!control.showing_placeholder);
    assert_eq!(control.text, "あ");
    assert!(!document_xml(&document)
        .contains("PlaceholderText\"/></w:rPr><w:t xml:space=\"preserve\">あ"));
    // The next keystroke goes on inside the control.
    type_text(&mut document, 0, 8, "い").unwrap();
    assert_eq!(document.content_controls()[0].text, "あい");
}

#[test]
fn a_selection_saved_to_the_gallery_goes_into_a_gallery_control() {
    let mut document = open(
        r#"<w:p><w:r><w:t>Kind regards,</w:t></w:r></w:p><w:p><w:r><w:t>Jade</w:t></w:r></w:p><w:p/>"#,
        "",
    );
    let mut executor = CommandExecutor::new();
    assert!(document.building_blocks(Some("Quick Parts")).is_empty());
    run(
        &mut document,
        &mut executor,
        ContentControlAction::SaveBuildingBlock {
            name: "Closing".to_string(),
            gallery: "Quick Parts".to_string(),
        },
        SelectionRange::new(TextPosition::new(0, 0), TextPosition::new(1, 4)),
    )
    .expect("save");
    assert_eq!(
        document.building_blocks(Some("Quick Parts")),
        vec!["Closing"]
    );
    run(
        &mut document,
        &mut executor,
        ContentControlAction::Insert {
            kind: ContentControlKind::BuildingBlockGallery,
            japanese: true,
        },
        at(2, 0),
    )
    .expect("gallery control");
    let gallery = document.content_controls().remove(0);
    assert!(gallery.showing_placeholder);
    run(
        &mut document,
        &mut executor,
        ContentControlAction::InsertBuildingBlock {
            control: 0,
            name: "Closing".to_string(),
        },
        at(2, 0),
    )
    .expect("insert the block");
    let control = document.content_controls().remove(0);
    assert_eq!(control.text, "Kind regards,\nJade");
    // Saved and opened again, the glossary is still there.
    let exported = document.export_docx().unwrap();
    let reopened =
        super::super::rewrite::run_ready(WordDocument::load(&exported, &Options::default()))
            .unwrap();
    assert_eq!(reopened.building_blocks(None), vec!["Closing"]);
}

#[test]
fn emptying_a_control_keeps_it() {
    let mut document = open(
        r#"<w:p><w:r><w:t xml:space="preserve">A </w:t></w:r><w:sdt><w:sdtPr><w:id w:val="5"/><w:lock w:val="sdtLocked"/></w:sdtPr><w:sdtContent><w:r><w:t>xy</w:t></w:r></w:sdtContent></w:sdt><w:r><w:t xml:space="preserve"> B</w:t></w:r></w:p>"#,
        "",
    );
    let mut executor = CommandExecutor::new();
    apply_headless_commands_with_executor_to_document(
        &mut document,
        &mut executor,
        None,
        &HeadlessEditRequest {
            commands: vec![HeadlessCommand::ReplaceRange {
                start_paragraph: 0,
                start_offset: 2,
                end_paragraph: 0,
                end_offset: 4,
                text: String::new(),
            }],
        },
    )
    .expect("the content can be deleted");
    // The control stays, its placeholder back.
    let xml = document_xml(&document);
    assert!(xml.contains("<w:sdt>"), "{xml}");
    let controls = document.content_controls();
    assert_eq!(controls.len(), 1);
    assert!(controls[0].showing_placeholder);
    assert_eq!(controls[0].text, "Click or tap here to enter text.");
    assert_eq!(controls[0].start, TextPosition::new(0, 2));
    // Typing over it puts text in again.
    let control = controls[0].clone();
    apply_headless_commands_with_executor_to_document(
        &mut document,
        &mut executor,
        Some(SelectionRange::new(control.start, control.end)),
        &HeadlessEditRequest {
            commands: vec![HeadlessCommand::ReplaceRange {
                start_paragraph: 0,
                start_offset: control.start.offset,
                end_paragraph: 0,
                end_offset: control.end.offset,
                text: "z".to_string(),
            }],
        },
    )
    .expect("typed over");
    assert_eq!(text_of(&document, 0), "A z B");
}

#[test]
fn wraps_text_that_shares_its_run_with_a_break() {
    // Word keeps "Hello" and the page break after it in one run; wrapping
    // the text splits the run before the break.
    let mut document = open(
        r#"<w:p><w:r><w:t>Hello</w:t><w:br w:type="page"/></w:r></w:p>"#,
        "",
    );
    let range = SelectionRange {
        anchor: TextPosition::new(0, 0),
        focus: TextPosition::new(0, 5),
    };
    run(
        &mut document,
        &mut CommandExecutor::new(),
        ContentControlAction::Insert {
            kind: ContentControlKind::RichText,
            japanese: true,
        },
        range,
    )
    .unwrap();
    let controls = document.content_controls();
    assert_eq!(controls.len(), 1);
    assert_eq!(controls[0].text, "Hello");
    let xml = document_xml(&document);
    assert!(xml.contains(r#"<w:br w:type="page"/>"#), "{xml}");
    assert!(
        xml.find("</w:sdt>") < xml.find("<w:br"),
        "the break stays outside the control: {xml}"
    );
}

#[test]
fn a_delete_from_inside_a_character_does_not_panic() {
    // A caret placed by byte offset can land inside a multi-byte character
    // (here byte 1 of 秘, and byte 4 of 密); stepping over a character from
    // there must not slice the text off a char boundary.
    let document = open("<w:p><w:r><w:t>秘密保持</w:t></w:r></w:p>", "");
    for (offset, direction) in [
        (1, HeadlessDeleteDirection::Forward),
        (4, HeadlessDeleteDirection::Backward),
        (4, HeadlessDeleteDirection::Forward),
    ] {
        let command = HeadlessCommand::DeleteText {
            paragraph: 0,
            offset,
            direction,
            count: 1,
        };
        assert_eq!(document.headless_edit_refusal(&command), None);
    }
}
