//! PDF export must consume the editor renderer, including the live edits.
mod common;

use lopdf::Document;

fn assert_embedded_editor_pixels(
    pdf: &Document,
    page: usize,
    bgra: &[u8],
    width: i32,
    height: i32,
) {
    let page_id = pdf.get_pages()[&(page as u32 + 1)];
    let (resources, _) = pdf.get_page_resources(page_id).unwrap();
    let xobjects = resources.unwrap().get(b"XObject").unwrap();
    let (_, xobjects) = pdf.dereference(xobjects).unwrap();
    let image = xobjects
        .as_dict()
        .unwrap()
        .iter()
        .find_map(|(_, object)| {
            let (_, object) = pdf.dereference(object).ok()?;
            let stream = object.as_stream().ok()?;
            let dict = &stream.dict;
            (dict.get(b"Subtype").ok()?.as_name().ok()? == b"Image"
                && dict.get(b"Width").ok()?.as_i64().ok()? == i64::from(width)
                && dict.get(b"Height").ok()?.as_i64().ok()? == i64::from(height))
            .then_some(stream)
        })
        .expect("page must contain its full editor raster");
    assert_eq!(
        image
            .dict
            .get(b"BitsPerComponent")
            .unwrap()
            .as_i64()
            .unwrap(),
        8
    );
    let data = image.decompressed_content().unwrap();
    let expected = bgra
        .chunks_exact(4)
        .flat_map(|pixel| [pixel[2], pixel[1], pixel[0]])
        .collect::<Vec<_>>();
    if data != expected {
        let test_name = std::thread::current().name().unwrap_or("pdf").to_string();
        let output = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("target/pdf-parity/failures")
            .join(test_name);
        std::fs::create_dir_all(&output).unwrap();
        pdf.clone().save(output.join("document.pdf")).unwrap();
        common::write_bgra_png(
            output.join(format!("editor-{page}.png")).to_str().unwrap(),
            bgra,
            width,
            height,
        );
        image::RgbImage::from_raw(width as u32, height as u32, data.clone())
            .unwrap()
            .save(output.join(format!("pdf-{page}.png")))
            .unwrap();
    }
    assert!(
        data == expected,
        "PDF page {} changed {} of {} live raster channels",
        page + 1,
        data.iter().zip(&expected).filter(|(a, b)| a != b).count(),
        expected.len()
    );
}

fn rewrite_document_xml(xml: impl FnOnce(String) -> String) -> Vec<u8> {
    use std::io::{Cursor, Read, Write};
    let bytes = std::fs::read(common::repo_path("public/fixtures/hello.docx")).unwrap();
    let mut input = zip::ZipArchive::new(Cursor::new(bytes)).unwrap();
    let mut output = zip::ZipWriter::new(Cursor::new(Vec::new()));
    let mut transform = Some(xml);
    for index in 0..input.len() {
        let mut part = input.by_index(index).unwrap();
        let name = part.name().unwrap().into_owned();
        let mut data = Vec::new();
        part.read_to_end(&mut data).unwrap();
        if name == "word/document.xml" {
            data = transform.take().unwrap()(String::from_utf8(data).unwrap()).into_bytes();
        }
        output
            .start_file(name, zip::write::SimpleFileOptions::default())
            .unwrap();
        output.write_all(&data).unwrap();
    }
    output.finish().unwrap().into_inner()
}

#[test]
fn malformed_embedded_image_returns_an_error_instead_of_panicking() {
    use std::io::{Cursor, Read, Write};
    let bytes = std::fs::read(common::repo_path("public/fixtures/image-inline.docx")).unwrap();
    let mut input = zip::ZipArchive::new(Cursor::new(bytes)).unwrap();
    let mut output = zip::ZipWriter::new(Cursor::new(Vec::new()));
    let mut replaced = false;
    for index in 0..input.len() {
        let mut part = input.by_index(index).unwrap();
        let name = part.name().unwrap().into_owned();
        let mut data = Vec::new();
        part.read_to_end(&mut data).unwrap();
        if name.starts_with("word/media/") {
            data = b"not an image".to_vec();
            replaced = true;
        }
        output
            .start_file(name, zip::write::SimpleFileOptions::default())
            .unwrap();
        output.write_all(&data).unwrap();
    }
    assert!(replaced);
    let bytes = output.finish().unwrap().into_inner();
    let error = jade_docx_native_render::NativeDocument::open(&bytes, 816.0, 1056.0, &[])
        .err()
        .expect("a broken picture must not be silently omitted");
    assert!(error.contains("image"), "{error}");
}

#[test]
fn pdf_exports_live_edits_as_searchable_text_without_changing_authoring_state() {
    let mut editor = common::open_fixture("public/fixtures/hello.docx");
    editor.insert_text(0, 0, "PDF日本語テスト", 1).unwrap();
    editor.set_formatting_marks(true).unwrap();
    editor.place_caret(0, 3);
    let before_docx = editor.export_docx().unwrap();
    let before_pixels = editor.page_pixels(0).unwrap();
    let before_history = (editor.can_undo(), editor.can_redo());

    let bytes = editor
        .export_pdf_with_options(jade_docx_native_render::PdfExportOptions {
            rendering: jade_docx_native_render::PdfRenderingMode::Vector,
            ..Default::default()
        })
        .expect("shared renderer vector PDF export");
    let pdf = Document::load_mem(&bytes).expect("valid PDF");
    assert_eq!(pdf.get_pages().len(), editor.page_count());
    let text = pdf
        .extract_text(&[1])
        .expect("extract the embedded font text")
        // lopdf inserts a newline per drawing operation, including individual
        // glyphs; this is not a line boundary in the shared layout.
        .replace('\n', "");
    assert!(
        text.contains("PDF") && text.contains("本語テスト"),
        "missing live edit: {text:?}"
    );
    // Noto maps 日 and the Kangxi radical ⽇ to the same glyph. PDF must
    // retain the original Unicode via ActualText when ToUnicode is ambiguous.
    // lopdf's extractor ignores ActualText; Poppler/Acrobat honor it.
    let page_id = *pdf.get_pages().get(&1).unwrap();
    let content = lopdf::content::Content::decode(&pdf.get_page_content(page_id).unwrap()).unwrap();
    assert!(
        content.operations.iter().any(|op| op.operator == "BDC"
            && op
                .operands
                .get(1)
                .and_then(|obj| obj.as_dict().ok())
                .and_then(|dict| dict.get(b"ActualText").ok())
                .and_then(|obj| obj.as_str().ok())
                == Some(&[0xfe, 0xff, 0x65, 0xe5][..])),
        "ambiguous glyph must carry the original Unicode"
    );
    assert!(
        !text.contains('¶'),
        "nonprinting marks must not be exported"
    );
    assert!(
        pdf.objects.values().any(|object| object
            .as_dict()
            .is_ok_and(|dict| dict.has(b"FontFile2") || dict.has(b"FontFile3"))),
        "PDF must embed its fonts"
    );
    assert!(
        !pdf.objects
            .values()
            .any(|object| object.as_stream().is_ok_and(|stream| stream
                .dict
                .get(b"Subtype")
                .is_ok_and(|value| value.as_name().is_ok_and(|name| name == b"Image")))),
        "text-only PDF must not be a full-page bitmap"
    );

    assert_eq!(editor.export_docx().unwrap(), before_docx);
    assert_eq!(editor.page_pixels(0).unwrap(), before_pixels);
    assert_eq!((editor.can_undo(), editor.can_redo()), before_history);
    assert!(editor.formatting_marks());
}

#[test]
fn default_pdf_retains_original_unicode_and_uses_editor_page_pixels() {
    let mut editor = common::open_fixture("public/fixtures/hello.docx");
    editor.insert_text(0, 0, "PDF日本語😀𠮷", 1).unwrap();
    editor.set_formatting_marks(false).unwrap();
    editor.set_crop_marks(false).unwrap();
    let before = editor.export_docx().unwrap();
    let pixels = editor.page_pixels(0).unwrap();
    let history = (editor.can_undo(), editor.can_redo());
    let pdf = Document::load_mem(&editor.export_pdf().unwrap()).unwrap();
    let text = pdf.extract_text(&[1]).unwrap().replace('\n', "");
    assert!(text.contains("PDF日本語😀𠮷"), "{text:?}");
    let (width, height) = (pixels.1, pixels.2);
    assert_embedded_editor_pixels(&pdf, 0, &pixels.0, width, height);
    assert_eq!(before, editor.export_docx().unwrap());
    assert_eq!(pixels, editor.page_pixels(0).unwrap());
    assert_eq!(history, (editor.can_undo(), editor.can_redo()));
}

#[test]
fn generated_documents_keep_live_pixels_after_edit_undo_and_redo() {
    // Combine supported formatting independently of the fixed corpus. Vary
    // page geometry, run boundaries, scripts, tabs, rules and table content.
    for seed in 0..16 {
        let bytes = rewrite_document_xml(|xml| {
            let start = xml.find("<w:body>").unwrap() + "<w:body>".len();
            let end = xml.rfind("</w:body>").unwrap();
            let mut body = String::new();
            for paragraph in 0..12 {
                let size = 16 + (seed + paragraph) % 24;
                let color = ["203060", "A03050", "206040", "101010"][(seed + paragraph) % 4];
                let decoration = ["<w:b/>", "<w:i/>", "<w:u w:val=\"single\"/>", "<w:strike/>"]
                    [(seed + paragraph) % 4];
                let alignment = ["left", "center", "right", "both"][(seed + paragraph) % 4];
                body.push_str(&format!(
                    "<w:p><w:pPr><w:jc w:val=\"{alignment}\"/><w:spacing w:before=\"{}\" w:after=\"{}\"/><w:tabs><w:tab w:val=\"right\" w:leader=\"dot\" w:pos=\"3600\"/></w:tabs></w:pPr><w:r><w:rPr><w:rFonts w:ascii=\"Noto Sans JP\" w:eastAsia=\"Noto Sans JP\"/><w:sz w:val=\"{size}\"/><w:color w:val=\"{color}\"/>{decoration}</w:rPr><w:t>日本語 Latin é 😀𠮷 {seed}:{paragraph}</w:t></w:r><w:r><w:tab/><w:t>123.45</w:t></w:r></w:p>",
                    seed * 7, paragraph * 11,
                ));
            }
            body.push_str("<w:tbl><w:tblPr><w:tblW w:w=\"3600\" w:type=\"dxa\"/><w:tblBorders><w:top w:val=\"single\" w:sz=\"4\"/><w:bottom w:val=\"single\" w:sz=\"4\"/></w:tblBorders></w:tblPr><w:tblGrid><w:gridCol w:w=\"1800\"/><w:gridCol w:w=\"1800\"/></w:tblGrid><w:tr><w:tc><w:p><w:r><w:t>表 Table</w:t></w:r></w:p></w:tc><w:tc><w:p><w:r><w:t>مرحبا עברית</w:t></w:r></w:p></w:tc></w:tr></w:tbl>");
            body.push_str(&format!(
                "<w:sectPr><w:pgSz w:w=\"{}\" w:h=\"{}\"/><w:pgMar w:top=\"720\" w:right=\"720\" w:bottom=\"720\" w:left=\"720\"/></w:sectPr>",
                8000 + seed * 137, 10000 + seed * 113,
            ));
            format!("{}{}{}", &xml[..start], body, &xml[end..])
        });
        let mut editor = common::open_bytes(&bytes);
        editor.set_formatting_marks(false).unwrap();
        editor.set_crop_marks(false).unwrap();
        for state in 0..4 {
            eprintln!("Generated PDF parity: seed={seed}, state={state}");
            match state {
                1 => {
                    editor.insert_text(0, 0, "編集😀 ", 1).unwrap();
                }
                2 => {
                    assert!(editor.undo().unwrap().is_some());
                }
                3 => {
                    assert!(editor.redo().unwrap().is_some());
                }
                _ => {}
            }
            let before = editor.export_docx().unwrap();
            let history = (editor.can_undo(), editor.can_redo());
            let pdf = Document::load_mem(&editor.export_pdf().unwrap()).unwrap();
            assert_eq!(pdf.get_pages().len(), editor.page_count());
            for page in 0..editor.page_count() {
                let (pixels, width, height) = editor.page_pixels(page).unwrap();
                assert_embedded_editor_pixels(&pdf, page, &pixels, width, height);
            }
            assert_eq!(before, editor.export_docx().unwrap());
            assert_eq!(history, (editor.can_undo(), editor.can_redo()));
        }
    }
}

#[test]
fn pdf_export_resolution_changes_pixels_without_changing_page_geometry() {
    let editor = common::open_fixture("public/fixtures/hello.docx");
    for scale in [0.5, 1.0, 1.25, 1.5, 2.0, 2.5, 4.0] {
        let bytes = editor
            .export_pdf_with_options(jade_docx_native_render::PdfExportOptions {
                scale,
                ..Default::default()
            })
            .unwrap();
        let pdf = Document::load_mem(&bytes).unwrap();
        let (width, height) = editor.layout_page_size();
        for page_id in pdf.get_pages().values() {
            let bounds = pdf
                .get_object(*page_id)
                .unwrap()
                .as_dict()
                .unwrap()
                .get(b"MediaBox")
                .unwrap()
                .as_array()
                .unwrap();
            assert!((f64::from(bounds[2].as_float().unwrap()) - width * 0.75).abs() < 0.01);
            assert!((f64::from(bounds[3].as_float().unwrap()) - height * 0.75).abs() < 0.01);
        }
        assert!(
            pdf.objects
                .values()
                .any(|object| object.as_stream().is_ok_and(|stream| {
                    let dict = &stream.dict;
                    dict.get(b"Subtype")
                        .is_ok_and(|value| value.as_name().is_ok_and(|name| name == b"Image"))
                        && dict.get(b"Width").is_ok_and(|value| {
                            value
                                .as_i64()
                                .is_ok_and(|value| value == (width * scale).ceil() as i64)
                        })
                        && dict.get(b"Height").is_ok_and(|value| {
                            value
                                .as_i64()
                                .is_ok_and(|value| value == (height * scale).ceil() as i64)
                        })
                })),
            "image resolution did not honor scale {scale}"
        );
    }
    for scale in [f64::NAN, f64::INFINITY, -1.0, 0.0, 0.49, 4.01] {
        assert!(
            editor
                .export_pdf_with_options(jade_docx_native_render::PdfExportOptions {
                    scale,
                    ..Default::default()
                })
                .is_err(),
            "invalid scale {scale} accepted"
        );
    }
}

#[test]
#[ignore = "full corpus edit/undo/redo pixel comparison; run --ignored"]
fn edited_corpus_documents_keep_live_pixels_after_undo_and_redo() {
    let mut paths: Vec<_> = std::fs::read_dir(common::repo_path("public/fixtures"))
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| {
            path.extension()
                .is_some_and(|extension| extension == "docx")
        })
        .collect();
    paths.sort();
    if let Ok(filter) = std::env::var("JADE_PDF_CORPUS_FILTER") {
        paths.retain(|path| path.file_stem().is_some_and(|name| name == filter.as_str()));
        assert!(!paths.is_empty(), "unknown fixture {filter}");
    }
    assert!(!paths.is_empty(), "the corpus must contain DOCX fixtures");
    for path in paths {
        let name = path.file_name().unwrap().to_string_lossy();
        let mut editor = common::open_bytes(&std::fs::read(&path).unwrap());
        editor.set_formatting_marks(false).unwrap();
        editor.set_crop_marks(false).unwrap();
        for state in ["edited", "undo", "redo"] {
            eprintln!("Edited corpus PDF parity: {name}, state={state}");
            match state {
                "edited" => {
                    // Treat every page as visible for this corpus oracle;
                    // a bounded one-page edit leaves later native bitmaps
                    // intentionally untouched until they are requested.
                    editor
                        .insert_text(0, 0, "[PDF 日本語😀 edit] ", usize::MAX)
                        .unwrap();
                }
                "undo" => {
                    assert!(editor.undo().unwrap().is_some());
                }
                "redo" => {
                    assert!(editor.redo().unwrap().is_some());
                }
                _ => unreachable!(),
            }
            let before = editor.export_docx().unwrap();
            let history = (editor.can_undo(), editor.can_redo());
            let pdf = Document::load_mem(&editor.export_pdf().unwrap()).unwrap();
            assert_eq!(
                pdf.get_pages().len(),
                editor.page_count(),
                "{name}, {state}"
            );
            for page in 0..editor.page_count() {
                let (pixels, width, height) = editor.page_pixels(page).unwrap();
                assert_embedded_editor_pixels(&pdf, page, &pixels, width, height);
            }
            assert_eq!(before, editor.export_docx().unwrap(), "{name}, {state}");
            assert_eq!(
                history,
                (editor.can_undo(), editor.can_redo()),
                "{name}, {state}"
            );
        }
    }
}

#[test]
fn pdf_keeps_every_page_and_document_page_geometry() {
    let editor = common::open_fixture("public/fixtures/contract.docx");
    let pdf = Document::load_mem(&editor.export_pdf().unwrap()).unwrap();
    let (width, height) = editor.layout_page_size();
    assert_eq!(pdf.get_pages().len(), editor.page_count());
    for id in pdf.get_pages().values() {
        let page = pdf.get_object(*id).unwrap().as_dict().unwrap();
        let bounds = page.get(b"MediaBox").unwrap().as_array().unwrap();
        assert!(
            (f64::from(bounds[2].as_float().unwrap()) - width * 0.75).abs() < 0.01,
            "{bounds:?}, expected {}",
            width * 0.75
        );
        assert!((f64::from(bounds[3].as_float().unwrap()) - height * 0.75).abs() < 0.01);
    }
}

#[test]
fn comment_rendering_is_stable_before_pdf_and_after_repaint() {
    let mut editor = common::open_fixture("public/fixtures/comment.docx");
    editor.set_formatting_marks(false).unwrap();
    let before = editor.page_pixels(0).unwrap();
    editor.repaint_all().unwrap();
    assert!(
        editor.page_pixels(0).unwrap() == before,
        "comment pixels changed on cache replay"
    );
}

#[test]
fn pdf_keeps_first_page_and_section_specific_header_footer_stories() {
    let editor = common::open_fixture("public/fixtures/header-footer-sections.docx");
    let pdf = Document::load_mem(&editor.export_pdf().unwrap()).unwrap();
    let pages = pdf.get_pages();
    let texts = (1..=pages.len() as u32)
        .map(|page| pdf.extract_text(&[page]).unwrap().replace('\n', ""))
        .collect::<Vec<_>>();
    assert_eq!(texts.len(), 2, "section pages changed: {texts:?}");
    assert!(
        texts[0].replace(' ', "").contains("TITLEPAGEHEADER"),
        "first-page header missing: {texts:?}"
    );
    assert!(
        texts[1].replace(' ', "").contains("SECTIONTWOHEADER"),
        "section header missing: {texts:?}"
    );
    // The unchanged Word capture checked by mac_missing_first_footer_stays_blank
    // has no first footer: titlePg does not fall back to the default footer.
    // The second section still inherits that default footer.
    assert_eq!(
        texts
            .iter()
            .map(|text| text.replace(' ', "").matches("SHAREDFOOTER").count())
            .collect::<Vec<_>>(),
        vec![0, 1],
        "first/default footer selection changed: {texts:?}"
    );
}

#[test]
fn pdf_retains_the_live_total_page_field_on_early_pages() {
    let mut editor = common::open_fixture("public/fixtures/header-footer-edge-cases.docx");
    editor.set_formatting_marks(false).unwrap();
    editor.set_crop_marks(false).unwrap();
    let (bgra, width, height) = editor.page_pixels(0).unwrap();
    let pdf = Document::load_mem(&editor.export_pdf().unwrap()).unwrap();
    let text = pdf.extract_text(&[1]).unwrap().replace(['\n', ' '], "");
    assert!(
        text.contains("Page1of2"),
        "total page field changed: {text:?}"
    );
    assert_embedded_editor_pixels(&pdf, 0, &bgra, width, height);
}

#[test]
fn tab_leader_pixels_are_stable_across_repaint_and_pdf_export() {
    let mut editor = common::open_fixture("public/fixtures/tabs.docx");
    editor.set_formatting_marks(false).unwrap();
    editor.set_crop_marks(false).unwrap();
    let before = editor.page_pixels(0).unwrap();
    editor.repaint_all().unwrap();
    assert!(
        before == editor.page_pixels(0).unwrap(),
        "tab leaders changed after cache replay"
    );
    let pdf = Document::load_mem(&editor.export_pdf().unwrap()).unwrap();
    assert_embedded_editor_pixels(&pdf, 0, &before.0, before.1, before.2);
}

#[test]
fn line_number_pixels_remain_correct_when_only_one_paragraph_changes() {
    let mut editor = common::open_fixture("public/fixtures/line-numbering.docx");
    editor.set_formatting_marks(false).unwrap();
    editor.set_crop_marks(false).unwrap();
    editor.insert_text(2, 0, "edit ", 1).unwrap();
    let pdf = Document::load_mem(&editor.export_pdf().unwrap()).unwrap();
    for page in 0..editor.page_count() {
        let (pixels, width, height) = editor.page_pixels(page).unwrap();
        assert_embedded_editor_pixels(&pdf, page, &pixels, width, height);
    }
}

#[test]
fn pdf_keeps_external_link_actions() {
    let editor = common::open_fixture("public/fixtures/hyperlink.docx");
    let pdf = Document::load_mem(&editor.export_pdf().unwrap()).unwrap();
    assert!(
        pdf.objects
            .values()
            .any(|object| object.as_dict().is_ok_and(|dict| dict
                .get(b"A")
                .and_then(lopdf::Object::as_dict)
                .and_then(|action| action.get(b"URI"))
                .is_ok_and(|uri| uri.as_str().is_ok_and(|uri| uri == b"https://example.com/")))),
        "PDF lost the document hyperlink"
    );
}

#[test]
fn pdf_resolves_forward_bookmarks_to_the_actual_page_and_position() {
    let bytes = rewrite_document_xml(|xml| {
        xml.replace(
            r#"<w:r><w:rPr><w:rtl w:val="0"/></w:rPr><w:t>Hello</w:t><w:br w:type="page"/></w:r>"#,
            r#"<w:hyperlink w:anchor="pdf_target"><w:r><w:t>Jump</w:t></w:r></w:hyperlink><w:r><w:br w:type="page"/></w:r>"#,
        )
        .replace(
            r#"<w:r><w:rPr><w:rtl w:val="0"/></w:rPr><w:t>World</w:t></w:r>"#,
            r#"<w:bookmarkStart w:id="901" w:name="pdf_target"/><w:r><w:t>World</w:t></w:r><w:bookmarkEnd w:id="901"/>"#,
        )
    });
    let mut editor = common::open_bytes(&bytes);
    let caret = editor.place_caret(1, 0).unwrap();
    let (page, y) = editor.page_and_offset(caret.document_y);
    assert_eq!(page, 1);
    let pdf = Document::load_mem(&editor.export_pdf().unwrap()).unwrap();
    let destination = pdf
        .objects
        .values()
        .filter_map(|value| value.as_dict().ok())
        .find_map(|dict| {
            dict.get(b"Dest")
                .ok()
                .and_then(|value| value.as_array().ok())
        })
        .expect("bookmark hyperlink must become an internal PDF destination");
    assert_eq!(destination[0].as_reference().unwrap(), pdf.get_pages()[&2]);
    assert_eq!(destination[1].as_name().unwrap(), b"XYZ");
    assert!((f64::from(destination[2].as_float().unwrap()) - caret.x * 0.75).abs() < 0.01);
    let height = editor.layout_page_size().1;
    assert!((f64::from(destination[3].as_float().unwrap()) - (height - y) * 0.75).abs() < 0.01);
}

#[test]
fn pdf_exports_the_live_header_after_edit_undo_and_redo() {
    let mut editor = common::open_fixture("public/fixtures/header.docx");
    let before = editor.export_docx().unwrap();
    editor
        .set_header_footer_text(false, 0, "PDF live header")
        .unwrap();
    let edited = editor.export_docx().unwrap();
    let text = Document::load_mem(&editor.export_pdf().unwrap())
        .unwrap()
        .extract_text(&[1])
        .unwrap()
        .replace(['\n', ' '], "");
    assert!(text.contains("PDFliveheader"), "{text:?}");
    editor.undo().unwrap();
    assert_eq!(editor.export_docx().unwrap(), before);
    assert!(
        editor.can_redo(),
        "header edit must remain redoable after PDF export"
    );
    editor.redo().unwrap();
    assert_eq!(editor.export_docx().unwrap(), edited);
}

#[test]
fn pdf_keeps_text_searchable_when_the_glyphs_have_visual_effects() {
    let editor = common::open_fixture("public/fixtures/char-effects.docx");
    let pdf = Document::load_mem(&editor.export_pdf().unwrap()).unwrap();
    let text = pdf.extract_text(&[1]).unwrap().replace(['\n', ' '], "");
    for expected in [
        "Shadowedtext",
        "Outlinedtext",
        "Embossedtext",
        "Imprintedtext",
        "Visibletail",
    ] {
        assert!(text.contains(expected), "missing {expected}: {text:?}");
    }
}

#[test]
#[ignore = "requires Poppler: set JADE_PDFTOTEXT and run --ignored"]
fn poppler_recovers_original_unicode_and_effect_text() {
    let poppler = std::env::var_os("JADE_PDFTOTEXT").expect("set JADE_PDFTOTEXT");
    let directory = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("target/pdf-text-parity");
    std::fs::create_dir_all(&directory).unwrap();
    for (fixture, expected) in [
        ("hello", vec!["PDF日本語😀"]),
        (
            "char-effects",
            vec![
                "Shadowedtext",
                "Outlinedtext",
                "Embossedtext",
                "Imprintedtext",
                "Visibletail",
            ],
        ),
    ] {
        let mut editor = common::open_fixture(&format!("public/fixtures/{fixture}.docx"));
        if fixture == "hello" {
            editor.insert_text(0, 0, "PDF日本語😀", 1).unwrap();
        }
        let path = directory.join(format!("{fixture}.pdf"));
        std::fs::write(&path, editor.export_pdf().unwrap()).unwrap();
        let result = std::process::Command::new(&poppler)
            .args(["-enc", "UTF-8"])
            .arg(path)
            .arg("-")
            .output()
            .unwrap();
        assert!(result.status.success());
        let text = String::from_utf8(result.stdout).unwrap();
        let text = text
            .chars()
            .filter(|ch| !ch.is_whitespace())
            .collect::<String>();
        for expected in expected {
            assert!(
                text.contains(expected),
                "{fixture} lost {expected}: {text:?}"
            );
        }
    }
}

#[test]
#[ignore = "requires Poppler: set JADE_PDFTOPPM and run --ignored"]
fn pdf_raster_matches_editor_on_representative_documents() {
    let poppler = std::env::var_os("JADE_PDFTOPPM").expect("set JADE_PDFTOPPM");
    let output = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("target/pdf-parity");
    std::fs::create_dir_all(&output).unwrap();
    let mut names = [
        "hello",
        "contract",
        "word-compat-comprehensive",
        "footnote-long-continuation",
        "section-break-types",
        "image-crop-edge-cases",
        "header-footer-edge-cases",
        "math",
        "table-object-edge-cases",
    ]
    .into_iter()
    .map(str::to_string)
    .collect::<Vec<_>>();
    if std::env::var_os("JADE_PDF_CORPUS_ALL").is_some() {
        names = std::fs::read_dir(common::repo_path("public/fixtures"))
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .filter(|path| path.extension().is_some_and(|ext| ext == "docx"))
            .map(|path| path.file_stem().unwrap().to_string_lossy().into_owned())
            .collect();
        names.sort();
    }
    names.push("_live-missing-glyph".to_string());
    if let Ok(filter) = std::env::var("JADE_PDF_CORPUS_FILTER") {
        names.retain(|name| name == &filter);
        assert!(!names.is_empty(), "unknown fixture {filter}");
    }
    let mut differences = Vec::new();
    for name in names {
        eprintln!("PDF parity: {name}");
        let mut editor = if name == "_live-missing-glyph" {
            let mut editor = common::open_fixture("public/fixtures/hello.docx");
            editor.insert_text(0, 0, "PDF日本語😀👨‍👩‍👧‍👦𠮷𩸽", 1).unwrap();
            editor
        } else {
            common::open_fixture(&format!("public/fixtures/{name}.docx"))
        };
        editor.set_formatting_marks(false).unwrap();
        editor.set_crop_marks(false).unwrap();
        let pdf_path = output.join(format!("{name}.pdf"));
        let bytes = editor.export_pdf().unwrap();
        let pdf = Document::load_mem(&bytes).unwrap();
        std::fs::write(&pdf_path, bytes).unwrap();
        let prefix = output.join(&name);
        let status = std::process::Command::new(&poppler)
            .args(["-png", "-r", "96"])
            .arg(&pdf_path)
            .arg(&prefix)
            .status()
            .unwrap();
        assert!(status.success(), "Poppler failed for {name}");
        let digits = editor.page_count().to_string().len();
        for page in 0..editor.page_count() {
            let rendered = image::open(output.join(format!("{name}-{:0digits$}.png", page + 1)))
                .unwrap()
                .to_rgb8();
            let (bgra, width, height) = editor.page_pixels(page).unwrap();
            assert_eq!(
                rendered.dimensions(),
                (width as u32, height as u32),
                "{name} page {page}"
            );
            common::write_bgra_png(
                output
                    .join(format!("{name}-editor-{}.png", page + 1))
                    .to_str()
                    .unwrap(),
                &bgra,
                width,
                height,
            );
            assert_embedded_editor_pixels(&pdf, page, &bgra, width, height);
            let mut misplaced_ink = 0;
            let mut ink = 0;
            // Different PDF/raster antialiasers can differ on a glyph edge.
            // Require all substantial ink to remain within one pixel on both
            // sides; a shifted, missing, or clipped object exceeds this gate.
            for y in 0..height {
                for x in 0..width {
                    let idx = ((y * width + x) * 4) as usize;
                    let reference = bgra[idx..idx + 3].iter().any(|v| *v < 192);
                    let actual = rendered
                        .get_pixel(x as u32, y as u32)
                        .0
                        .iter()
                        .any(|v| *v < 192);
                    if reference || actual {
                        ink += 1;
                    }
                    for (present, other_pdf) in [(reference, true), (actual, false)] {
                        if !present {
                            continue;
                        }
                        let mut found = false;
                        for yy in (y - 1).max(0)..=(y + 1).min(height - 1) {
                            for xx in (x - 1).max(0)..=(x + 1).min(width - 1) {
                                found |= if other_pdf {
                                    rendered
                                        .get_pixel(xx as u32, yy as u32)
                                        .0
                                        .iter()
                                        .any(|v| *v < 224)
                                } else {
                                    let offset = ((yy * width + xx) * 4) as usize;
                                    bgra[offset..offset + 3].iter().any(|v| *v < 224)
                                };
                            }
                        }
                        if !found {
                            misplaced_ink += 1;
                        }
                    }
                }
            }
            if misplaced_ink > ink / 500 + 2 {
                differences.push(format!(
                    "{name} page {}: {misplaced_ink}/{ink} ink pixels differ",
                    page + 1
                ));
            }
        }
    }
    assert!(
        differences.is_empty(),
        "{}; see {}",
        differences.join("\n"),
        output.display()
    );
}

#[test]
#[ignore = "requires Poppler: set JADE_PDFTOPPM and run --ignored"]
fn default_pdf_preserves_live_text_density() {
    let mut editor = common::open_fixture("public/fixtures/word-compat-comprehensive.docx");
    editor.set_formatting_marks(false).unwrap();
    editor.set_crop_marks(false).unwrap();
    let directory = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("target/pdf-density");
    std::fs::create_dir_all(&directory).unwrap();
    let path = directory.join("text.pdf");
    std::fs::write(&path, editor.export_pdf().unwrap()).unwrap();
    let status = std::process::Command::new(std::env::var_os("JADE_PDFTOPPM").unwrap())
        .args(["-png", "-r", "96", "-f", "1", "-l", "1", "-singlefile"])
        .arg(&path)
        .arg(directory.join("text"))
        .status()
        .unwrap();
    assert!(status.success());
    let rendered = image::open(directory.join("text.png")).unwrap().to_rgb8();
    let (bgra, _, _) = editor.page_pixels(0).unwrap();
    let expected = bgra
        .chunks_exact(4)
        .map(|pixel| {
            pixel[..3]
                .iter()
                .map(|channel| u64::from(255 - channel))
                .sum::<u64>()
        })
        .sum::<u64>();
    let actual = rendered
        .pixels()
        .map(|pixel| {
            pixel
                .0
                .iter()
                .map(|channel| u64::from(255 - channel))
                .sum::<u64>()
        })
        .sum::<u64>();
    assert!(
        expected.abs_diff(actual) * 100 <= expected * 5,
        "live={expected}, PDF={actual}; see {}",
        directory.display()
    );
}
