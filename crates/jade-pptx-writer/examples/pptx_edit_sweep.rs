//! Writes each editable run scenario for independent Open XML validation.
use jade_pptx_writer::{editable_runs, editable_shapes, EditSession};
use std::{error::Error, fs, path::Path};

fn write(root: &Path, scenario: &str, filename: &str, bytes: &[u8]) -> Result<(), Box<dyn Error>> {
    let dir = root.join(scenario);
    fs::create_dir_all(&dir)?;
    fs::write(dir.join(filename), bytes)?;
    Ok(())
}

fn main() -> Result<(), Box<dyn Error>> {
    let args: Vec<_> = std::env::args().collect();
    if args.len() != 3 {
        return Err("usage: pptx_edit_sweep <fixtures-dir> <output-dir>".into());
    }
    let output = Path::new(&args[2]);
    let mut fixtures = fs::read_dir(&args[1])?.collect::<Result<Vec<_>, _>>()?;
    fixtures.sort_by_key(|entry| entry.path());
    let mut count = 0;
    for fixture in fixtures {
        let path = fixture.path();
        if path.extension().and_then(|e| e.to_str()) != Some("pptx") {
            continue;
        }
        let filename = path
            .file_name()
            .and_then(|n| n.to_str())
            .ok_or("invalid fixture filename")?;
        let bytes = fs::read(&path)?;
        let deck = jade_pptx_reader::read_deck(&bytes)?;
        write(output, "noop", filename, &bytes)?;
        for index in 0..deck.slides.len() {
            let snapshot = jade_pptx_writer::notes_text(&bytes, index)?;
            if !snapshot.can_edit {
                continue;
            }
            for (mode, value, soft_break) in [
                (
                    "replace",
                    "発表者ノート & <PPTX> 🦀\n次の段落".to_string(),
                    false,
                ),
                ("append", format!("{}\n追加", snapshot.text), false),
                ("soft-break", format!("{}\n改行", snapshot.text), true),
                ("delete", String::new(), false),
            ] {
                let mut session = EditSession::open(&bytes)?;
                if !session.set_slide_notes_with_breaks(index, &value, soft_break)? {
                    continue;
                }
                let scenario = format!("slide-{index:03}-notes-{mode}");
                write(output, &scenario, filename, session.bytes())?;
                assert!(session.undo());
                assert_eq!(session.bytes(), bytes);
                assert!(session.redo());
                write(
                    output,
                    &format!("{scenario}-redo"),
                    filename,
                    session.bytes(),
                )?;
                count += 2;
            }
        }
        for index in 0..deck.slides.len() {
            for mode in ["duplicate", "delete", "move", "hidden", "insert"] {
                let mut session = EditSession::open(&bytes)?;
                let changed = match mode {
                    "duplicate" => session.duplicate_slide(index)?,
                    "delete" => session.delete_slide(index)?,
                    "move" => session.move_slide(index, (index + 1) % deck.slides.len())?,
                    "hidden" => {
                        session.set_slide_hidden(index, !deck.slides[index].content.hidden)?
                    }
                    "insert" => session.insert_slide(index)?,
                    _ => unreachable!(),
                };
                if !changed {
                    continue;
                }
                let scenario = format!("slide-{index:03}-operation-{mode}");
                write(output, &scenario, filename, session.bytes())?;
                assert!(session.undo());
                assert_eq!(session.bytes(), bytes);
                assert!(session.redo());
                write(
                    output,
                    &format!("{scenario}-redo"),
                    filename,
                    session.bytes(),
                )?;
                count += 2;
            }
        }
        let mut empty = EditSession::open(&bytes)?;
        for _ in 0..deck.slides.len() {
            empty.delete_slide(0)?;
        }
        write(output, "empty-deck", filename, empty.bytes())?;
        count += 1;
        let all: Vec<_> = (0..deck.slides.len()).collect();
        if all.len() > 1 {
            for (scenario, indices) in [
                (
                    "batch-delete-alternate",
                    all.iter()
                        .copied()
                        .filter(|i| i % 2 == 0)
                        .collect::<Vec<_>>(),
                ),
                ("batch-delete-all", all.clone()),
            ] {
                let mut session = EditSession::open(&bytes)?;
                assert!(session.delete_slides(&indices)?);
                write(output, scenario, filename, session.bytes())?;
                assert!(session.undo());
                assert_eq!(session.bytes(), bytes);
                assert!(session.redo());
                write(
                    output,
                    &format!("{scenario}-redo"),
                    filename,
                    session.bytes(),
                )?;
                count += 2;
            }
            for (name, indices) in [
                (
                    "alternate",
                    all.iter()
                        .copied()
                        .filter(|i| i % 2 == 0)
                        .collect::<Vec<_>>(),
                ),
                ("all", all.clone()),
            ] {
                let mut session = EditSession::open(&bytes)?;
                for hidden in [true, false] {
                    let before = session.bytes().to_vec();
                    if session.set_slides_hidden(&indices, hidden)? {
                        let scenario = format!(
                            "batch-visibility-{name}-{}",
                            if hidden { "hidden" } else { "visible" }
                        );
                        write(output, &scenario, filename, session.bytes())?;
                        assert!(session.undo());
                        assert_eq!(session.bytes(), before);
                        assert!(session.redo());
                        write(
                            output,
                            &format!("{scenario}-redo"),
                            filename,
                            session.bytes(),
                        )?;
                        assert!(!session.set_slides_hidden(&indices, hidden)?);
                        count += 2;
                    }
                }
            }
            for (name, indices, to) in [
                (
                    "alternate-front",
                    all.iter()
                        .copied()
                        .filter(|i| i % 2 == 1)
                        .collect::<Vec<_>>(),
                    0,
                ),
                ("ends-back", vec![0, all.len() - 1], all.len() - 2),
            ] {
                let mut session = EditSession::open(&bytes)?;
                if session.move_slides(&indices, to)? {
                    let scenario = format!("batch-move-{name}");
                    write(output, &scenario, filename, session.bytes())?;
                    assert!(session.undo());
                    assert_eq!(session.bytes(), bytes);
                    assert!(session.redo());
                    write(
                        output,
                        &format!("{scenario}-redo"),
                        filename,
                        session.bytes(),
                    )?;
                    let targets: Vec<_> = (to..to + indices.len()).collect();
                    assert!(!session.move_slides(&targets, to)?);
                    count += 2;
                }
            }
            for (name, indices) in [
                (
                    "alternate",
                    all.iter()
                        .copied()
                        .filter(|i| i % 2 == 0)
                        .collect::<Vec<_>>(),
                ),
                ("all", all.clone()),
            ] {
                let mut session = EditSession::open(&bytes)?;
                assert!(session.duplicate_slides(&indices)?);
                let scenario = format!("batch-duplicate-{name}");
                write(output, &scenario, filename, session.bytes())?;
                assert!(session.undo());
                assert_eq!(session.bytes(), bytes);
                assert!(session.redo());
                write(
                    output,
                    &format!("{scenario}-redo"),
                    filename,
                    session.bytes(),
                )?;
                count += 2;
            }
            for layout in jade_pptx_writer::slide_layouts(&bytes, 0)? {
                let mut session = EditSession::open(&bytes)?;
                if session.set_slide_layouts(&all, &layout.part_name)? {
                    let scenario = format!("batch-layout-{}", layout.part_name.replace('/', "_"));
                    write(output, &scenario, filename, session.bytes())?;
                    assert!(session.undo());
                    assert_eq!(session.bytes(), bytes);
                    assert!(session.redo());
                    write(
                        output,
                        &format!("{scenario}-redo"),
                        filename,
                        session.bytes(),
                    )?;
                    assert!(!session.set_slide_layouts(&all, &layout.part_name)?);
                    count += 2;
                }
            }
            let mut session = EditSession::open(&bytes)?;
            if session.reset_slides(&all)? {
                write(output, "batch-reset", filename, session.bytes())?;
                assert!(session.undo());
                assert_eq!(session.bytes(), bytes);
                assert!(session.redo());
                write(output, "batch-reset-redo", filename, session.bytes())?;
                assert!(!session.reset_slides(&all)?);
                count += 2;
            }
        }
        for slide in 0..deck.slides.len() {
            for (kind, format) in [
                ("png", image::ImageFormat::Png),
                ("jpeg", image::ImageFormat::Jpeg),
            ] {
                let mut encoded = std::io::Cursor::new(Vec::new());
                image::DynamicImage::ImageRgb8(image::RgbImage::from_pixel(
                    40,
                    20,
                    image::Rgb([200, 20, 30]),
                ))
                .write_to(&mut encoded, format)?;
                let image = encoded.into_inner();
                let mut session = EditSession::open(&bytes)?;
                let inserted = session.insert_picture(
                    slide,
                    jade_pptx_reader::Transform {
                        offset_emu: (952500, 952500),
                        extent_emu: (3810000, 1905000),
                        rotation_60000ths: 0,
                        flip_h: false,
                        flip_v: false,
                    },
                    &image,
                )?;
                let scenario = format!("slide-{slide:03}-picture-{kind}");
                write(output, &scenario, filename, session.bytes())?;
                assert!(session.undo());
                assert_eq!(session.bytes(), bytes);
                assert!(session.redo());
                write(
                    output,
                    &format!("{scenario}-redo"),
                    filename,
                    session.bytes(),
                )?;
                session.duplicate_shape(slide, inserted.id)?;
                let mut replacement = std::io::Cursor::new(Vec::new());
                image::DynamicImage::ImageRgb8(image::RgbImage::from_pixel(
                    20,
                    40,
                    image::Rgb([20, 200, 30]),
                ))
                .write_to(
                    &mut replacement,
                    if kind == "png" {
                        image::ImageFormat::Jpeg
                    } else {
                        image::ImageFormat::Png
                    },
                )?;
                let mut replaced = EditSession::open(session.bytes())?;
                let before_replace = replaced.bytes().to_vec();
                let fingerprint =
                    jade_pptx_writer::picture_replacement(replaced.bytes(), slide, inserted.id)?
                        .unwrap()
                        .fingerprint;
                replaced.replace_picture(
                    slide,
                    inserted.id,
                    replacement.get_ref(),
                    &fingerprint,
                )?;
                write(
                    output,
                    &format!("{scenario}-replace"),
                    filename,
                    replaced.bytes(),
                )?;
                assert!(replaced.undo());
                assert_eq!(replaced.bytes(), before_replace);
                assert!(replaced.redo());
                write(
                    output,
                    &format!("{scenario}-replace-redo"),
                    filename,
                    replaced.bytes(),
                )?;
                count += 2;
                for keep_frame in [false, true] {
                    let mut cropped = EditSession::open(session.bytes())?;
                    cropped.set_picture_crop(
                        slide,
                        inserted.id,
                        jade_pptx_reader::SourceRect {
                            left: 25000,
                            top: 10000,
                            right: 0,
                            bottom: 0,
                        },
                        keep_frame,
                    )?;
                    write(
                        output,
                        &format!("{scenario}-crop-{keep_frame}"),
                        filename,
                        cropped.bytes(),
                    )?;
                    cropped.set_picture_crop(
                        slide,
                        inserted.id,
                        jade_pptx_reader::SourceRect {
                            left: -10000,
                            top: 0,
                            right: -10000,
                            bottom: 0,
                        },
                        keep_frame,
                    )?;
                    write(
                        output,
                        &format!("{scenario}-outset-{keep_frame}"),
                        filename,
                        cropped.bytes(),
                    )?;
                    cropped.set_picture_crop(
                        slide,
                        inserted.id,
                        jade_pptx_reader::SourceRect::default(),
                        keep_frame,
                    )?;
                    write(
                        output,
                        &format!("{scenario}-reset-{keep_frame}"),
                        filename,
                        cropped.bytes(),
                    )?;
                    count += 3;
                }
                write(
                    output,
                    &format!("{scenario}-duplicate"),
                    filename,
                    session.bytes(),
                )?;
                count += 3;
            }
            for preset in [
                jade_pptx_writer::ShapePreset::Rect,
                jade_pptx_writer::ShapePreset::RoundRect,
                jade_pptx_writer::ShapePreset::Ellipse,
                jade_pptx_writer::ShapePreset::Triangle,
                jade_pptx_writer::ShapePreset::Diamond,
                jade_pptx_writer::ShapePreset::Hexagon,
            ] {
                for style in ["theme", "custom", "transparent"] {
                    let mut session = EditSession::open(&bytes)?;
                    let id = session.insert_shape(
                        slide,
                        jade_pptx_reader::Transform {
                            offset_emu: (-95250, 952500),
                            extent_emu: (3048000, 1524000),
                            rotation_60000ths: 30 * 60000,
                            flip_h: true,
                            flip_v: false,
                        },
                        &jade_pptx_writer::ShapeInsert {
                            preset,
                            text: "日本語 & <shape> 🦀\n\n".into(),
                            format: jade_pptx_writer::TextFormat {
                                bold: Some(true),
                                ..Default::default()
                            },
                            fill: match style {
                                "custom" => Some("#12AB34".into()),
                                "transparent" => Some("none".into()),
                                _ => None,
                            },
                            outline: match style {
                                "custom" => Some("#AB1234".into()),
                                "transparent" => Some("none".into()),
                                _ => None,
                            },
                            outline_width_pt: (style == "custom").then_some(1584.0),
                        },
                    )?;
                    assert!(jade_pptx_writer::shape_text(session.bytes(), slide, id)?.can_edit);
                    let scenario = format!("slide-{slide:03}-shape-{}-{style}", preset.as_str());
                    let saved = session.bytes().to_vec();
                    write(output, &scenario, filename, &saved)?;
                    assert!(session.undo());
                    assert_eq!(session.bytes(), bytes);
                    assert!(session.redo());
                    assert_eq!(session.bytes(), saved);
                    write(
                        output,
                        &format!("{scenario}-redo"),
                        filename,
                        session.bytes(),
                    )?;
                    count += 2;
                }
            }
            for (mode, text) in [("empty", ""), ("rich", "日本語 & <text> 🦀\n\n次の段落\n")]
            {
                let mut session = EditSession::open(&bytes)?;
                let id = session.insert_text_box(
                    slide,
                    jade_pptx_reader::Transform {
                        offset_emu: (952500, 952500),
                        extent_emu: (3048000, 1143000),
                        rotation_60000ths: 0,
                        flip_h: false,
                        flip_v: false,
                    },
                    text,
                    &jade_pptx_writer::TextFormat {
                        size_pt: Some(24.0),
                        bold: Some(true),
                        color: Some("#123456".into()),
                        ..Default::default()
                    },
                )?;
                assert_eq!(
                    jade_pptx_writer::shape_text(session.bytes(), slide, id)?.text,
                    text
                );
                let inserted = session.bytes().to_vec();
                let scenario = format!("slide-{slide:03}-text-box-{mode}");
                write(output, &scenario, filename, &inserted)?;
                assert!(session.undo());
                assert_eq!(session.bytes(), bytes);
                assert!(session.redo());
                assert_eq!(session.bytes(), inserted);
                write(
                    output,
                    &format!("{scenario}-redo"),
                    filename,
                    session.bytes(),
                )?;
                count += 2;
                session.set_shape_text(slide, id, "追加したテキストを編集 🦀", false)?;
                write(
                    output,
                    &format!("{scenario}-typed"),
                    filename,
                    session.bytes(),
                )?;
                count += 1;
            }
            {
                let mut session = EditSession::open(&bytes)?;
                let mut value = jade_pptx_writer::notes_text(&bytes, slide)?.text;
                if jade_pptx_writer::notes_text(&bytes, slide)?.can_edit {
                    for chunk in ["入力", "🦀", "続き"] {
                        let at = value.encode_utf16().count();
                        session.apply_notes_input(
                            slide,
                            &jade_pptx_writer::NotesInputEdit {
                                start: at,
                                end: at,
                                text: chunk.into(),
                                expected_text: value.clone(),
                                soft_break: false,
                                history: Some(jade_pptx_writer::TypingKind::Insert),
                            },
                        )?;
                        value.push_str(chunk);
                    }
                    assert_eq!(session.history_depth(), (1, 0));
                    let scenario = format!("slide-{slide:03}-notes-typing");
                    write(output, &scenario, filename, session.bytes())?;
                    assert!(session.undo());
                    assert_eq!(session.bytes(), bytes);
                    assert!(session.redo());
                    write(
                        output,
                        &format!("{scenario}-redo"),
                        filename,
                        session.bytes(),
                    )?;
                    count += 2;
                }
            }
            for choice in jade_pptx_writer::slide_layouts(&bytes, slide)? {
                let mut session = EditSession::open(&bytes)?;
                if !session.set_slide_layout(slide, &choice.part_name)? {
                    continue;
                }
                let scenario = format!(
                    "slide-{slide:03}-layout-{}",
                    choice.part_name.replace('/', "_")
                );
                write(output, &scenario, filename, session.bytes())?;
                assert!(session.undo());
                assert_eq!(session.bytes(), bytes);
                assert!(session.redo());
                write(
                    output,
                    &format!("{scenario}-redo"),
                    filename,
                    session.bytes(),
                )?;
                count += 2;
            }
            {
                let mut session = EditSession::open(&bytes)?;
                if session.reset_slide(slide)? {
                    let scenario = format!("slide-{slide:03}-reset");
                    write(output, &scenario, filename, session.bytes())?;
                    assert!(session.undo());
                    assert_eq!(session.bytes(), bytes);
                    assert!(session.redo());
                    write(
                        output,
                        &format!("{scenario}-redo"),
                        filename,
                        session.bytes(),
                    )?;
                    assert!(!session.reset_slide(slide)?);
                    count += 2;
                }
            }
            for item in &deck.slides[slide].content.shapes {
                let jade_pptx_reader::ShapeItem::Shape(shape) = item else {
                    continue;
                };
                let Some(id) = shape.id.filter(|_| shape.placeholder.is_some()) else {
                    continue;
                };
                let state = jade_pptx_writer::shape_text(&bytes, slide, id)?;
                let Some(info) = editable_shapes(&bytes, slide)?.into_iter().find(|s| {
                    s.id == id && s.can_move && s.can_resize && s.can_rotate && !s.lock_aspect_ratio
                }) else {
                    continue;
                };
                if !state.can_edit {
                    continue;
                }
                let Some(mut frame) = info.transform else {
                    continue;
                };
                let mut session = EditSession::open(&bytes)?;
                frame.offset_emu.0 += 95250;
                session.set_shape_transform(slide, id, frame)?;
                session.format_text_range(
                    slide,
                    &jade_pptx_writer::TextFormatEdit {
                        shape_id: id,
                        start: 0,
                        end: state.text.encode_utf16().count(),
                        format: jade_pptx_writer::TextFormat {
                            size_pt: Some(68.0),
                            color: Some("#FF0000".into()),
                            ..Default::default()
                        },
                    },
                )?;
                let before = session.bytes().to_vec();
                if session.reset_slide(slide)? {
                    let scenario = format!("slide-{slide:03}-shape-{id}-edited-reset");
                    write(output, &scenario, filename, session.bytes())?;
                    assert!(session.undo());
                    assert_eq!(session.bytes(), before);
                    assert!(session.redo());
                    write(
                        output,
                        &format!("{scenario}-redo"),
                        filename,
                        session.bytes(),
                    )?;
                    assert!(!session.reset_slide(slide)?);
                    count += 2;
                }
            }
            let layout = jade_pptx_layout::layout_deck_slide(
                &deck,
                slide,
                &jade_pptx_layout::HeuristicMeasure,
            )
            .ok_or("slide has no layout")?;
            let groupable: Vec<_> = editable_shapes(&bytes, slide)?
                .into_iter()
                .filter(|object| object.can_group)
                .map(|object| object.id)
                .collect();
            if groupable.len() >= 2 {
                for (mode, ids) in [("pair", &groupable[..2]), ("all", groupable.as_slice())] {
                    let mut session = EditSession::open(&bytes)?;
                    session.group_objects(slide, ids)?;
                    let scenario = format!("slide-{slide:03}-group-{mode}");
                    write(output, &scenario, filename, session.bytes())?;
                    assert!(session.undo());
                    assert_eq!(session.bytes(), bytes);
                    assert!(session.redo());
                    write(
                        output,
                        &format!("{scenario}-redo"),
                        filename,
                        session.bytes(),
                    )?;
                    count += 2;
                }
            }
            for object in editable_shapes(&bytes, slide)? {
                if jade_pptx_writer::shape_style(&bytes, slide, object.id)?
                    .is_some_and(|style| style.can_edit)
                {
                    use jade_pptx_writer::ShapeStyleEdit;
                    for (mode, style) in [
                        (
                            "fill",
                            ShapeStyleEdit {
                                fill: Some("#12AB34".into()),
                                ..Default::default()
                            },
                        ),
                        (
                            "no-fill",
                            ShapeStyleEdit {
                                fill: Some("none".into()),
                                ..Default::default()
                            },
                        ),
                        (
                            "outline",
                            ShapeStyleEdit {
                                outline: Some("#AB1234".into()),
                                ..Default::default()
                            },
                        ),
                        (
                            "no-outline",
                            ShapeStyleEdit {
                                outline: Some("none".into()),
                                ..Default::default()
                            },
                        ),
                        (
                            "zero-width",
                            ShapeStyleEdit {
                                outline_width_pt: Some(0.0),
                                ..Default::default()
                            },
                        ),
                        (
                            "max-width",
                            ShapeStyleEdit {
                                outline_width_pt: Some(1584.0),
                                ..Default::default()
                            },
                        ),
                        (
                            "all",
                            ShapeStyleEdit {
                                fill: Some("#AB1234".into()),
                                outline: Some("#1234AB".into()),
                                outline_width_pt: Some(2.75),
                            },
                        ),
                    ] {
                        let mut session = EditSession::open(&bytes)?;
                        if !session.set_shape_style(slide, object.id, &style)? {
                            continue;
                        }
                        let scenario =
                            format!("slide-{slide:03}-object-{:03}-style-{mode}", object.id);
                        write(output, &scenario, filename, session.bytes())?;
                        assert!(session.undo());
                        assert_eq!(session.bytes(), bytes);
                        assert!(session.redo());
                        write(
                            output,
                            &format!("{scenario}-redo"),
                            filename,
                            session.bytes(),
                        )?;
                        count += 2;
                    }
                }
                if object.can_duplicate {
                    let mut session = EditSession::open(&bytes)?;
                    let new_id = session.duplicate_shape(slide, object.id)?;
                    let scenario = format!("slide-{slide:03}-object-{:03}-duplicate", object.id);
                    write(output, &scenario, filename, session.bytes())?;
                    assert!(session.undo());
                    assert_eq!(session.bytes(), bytes);
                    assert!(session.redo());
                    write(
                        output,
                        &format!("{scenario}-redo"),
                        filename,
                        session.bytes(),
                    )?;
                    session.delete_shape(slide, new_id)?;
                    write(
                        output,
                        &format!("{scenario}-delete-copy"),
                        filename,
                        session.bytes(),
                    )?;
                    count += 3;
                }
                if object.can_delete {
                    let mut session = EditSession::open(&bytes)?;
                    session.delete_shape(slide, object.id)?;
                    let scenario = format!("slide-{slide:03}-object-{:03}-delete", object.id);
                    write(output, &scenario, filename, session.bytes())?;
                    assert!(session.undo());
                    assert_eq!(session.bytes(), bytes);
                    assert!(session.redo());
                    write(
                        output,
                        &format!("{scenario}-redo"),
                        filename,
                        session.bytes(),
                    )?;
                    count += 2;
                }
                if object.can_arrange {
                    for (mode, direction) in [
                        ("front", jade_pptx_writer::ObjectArrangement::Front),
                        ("back", jade_pptx_writer::ObjectArrangement::Back),
                        ("forward", jade_pptx_writer::ObjectArrangement::Forward),
                        ("backward", jade_pptx_writer::ObjectArrangement::Backward),
                    ] {
                        let mut session = EditSession::open(&bytes)?;
                        if !session.arrange_shape(slide, object.id, direction)? {
                            continue;
                        }
                        let scenario = format!("slide-{slide:03}-object-{:03}-{mode}", object.id);
                        write(output, &scenario, filename, session.bytes())?;
                        assert!(session.undo());
                        assert_eq!(session.bytes(), bytes);
                        assert!(session.redo());
                        write(
                            output,
                            &format!("{scenario}-redo"),
                            filename,
                            session.bytes(),
                        )?;
                        count += 2;
                    }
                }
                match jade_pptx_writer::shape_text(&bytes, slide, object.id) {
                    Ok(snapshot) if snapshot.can_edit => {
                        let length = snapshot.text.encode_utf16().count();
                        let first = snapshot
                            .text
                            .chars()
                            .next()
                            .map(char::len_utf16)
                            .unwrap_or(0);
                        for (mode, start, end, format) in [
                            (
                                "bold",
                                0,
                                length,
                                jade_pptx_writer::TextFormat {
                                    bold: Some(true),
                                    ..Default::default()
                                },
                            ),
                            (
                                "plain",
                                0,
                                length,
                                jade_pptx_writer::TextFormat {
                                    bold: Some(false),
                                    italic: Some(false),
                                    underline: Some(false),
                                    ..Default::default()
                                },
                            ),
                            (
                                "partial",
                                0,
                                first,
                                jade_pptx_writer::TextFormat {
                                    italic: Some(true),
                                    underline: Some(true),
                                    size_pt: Some(32.5),
                                    ..Default::default()
                                },
                            ),
                            (
                                "minimum-size",
                                0,
                                length,
                                jade_pptx_writer::TextFormat {
                                    size_pt: Some(1.0),
                                    ..Default::default()
                                },
                            ),
                            (
                                "maximum-size",
                                0,
                                length,
                                jade_pptx_writer::TextFormat {
                                    size_pt: Some(4000.0),
                                    ..Default::default()
                                },
                            ),
                            (
                                "font-color",
                                0,
                                length,
                                jade_pptx_writer::TextFormat {
                                    font: Some("Aptos".into()),
                                    east_asian_font: Some("游ゴシック".into()),
                                    color: Some("#12ABEF".into()),
                                    ..Default::default()
                                },
                            ),
                            (
                                "partial-font-color",
                                0,
                                first,
                                jade_pptx_writer::TextFormat {
                                    font: Some("A & B \"face\"".into()),
                                    east_asian_font: Some("游ゴシック".into()),
                                    color: Some("#FF8800".into()),
                                    ..Default::default()
                                },
                            ),
                        ] {
                            let mut session = EditSession::open(&bytes)?;
                            if !session.format_text_range(
                                slide,
                                &jade_pptx_writer::TextFormatEdit {
                                    shape_id: object.id,
                                    start,
                                    end,
                                    format,
                                },
                            )? {
                                continue;
                            }
                            let scenario =
                                format!("slide-{slide:03}-format-{:03}-{mode}", object.id);
                            write(output, &scenario, filename, session.bytes())?;
                            assert!(session.undo());
                            assert_eq!(session.bytes(), bytes);
                            assert!(session.redo());
                            write(
                                output,
                                &format!("{scenario}-redo"),
                                filename,
                                session.bytes(),
                            )?;
                            count += 2;
                        }
                        for (mode, format) in [
                            (
                                "center",
                                jade_pptx_writer::ParagraphFormat {
                                    alignment: Some(jade_pptx_writer::ParagraphAlignment::Center),
                                    ..Default::default()
                                },
                            ),
                            (
                                "right",
                                jade_pptx_writer::ParagraphFormat {
                                    alignment: Some(jade_pptx_writer::ParagraphAlignment::Right),
                                    ..Default::default()
                                },
                            ),
                            (
                                "justify",
                                jade_pptx_writer::ParagraphFormat {
                                    alignment: Some(jade_pptx_writer::ParagraphAlignment::Justify),
                                    ..Default::default()
                                },
                            ),
                            (
                                "distributed",
                                jade_pptx_writer::ParagraphFormat {
                                    alignment: Some(
                                        jade_pptx_writer::ParagraphAlignment::Distributed,
                                    ),
                                    ..Default::default()
                                },
                            ),
                            (
                                "right-margin-zero",
                                jade_pptx_writer::ParagraphFormat {
                                    margin_right_pt: Some(0.0),
                                    ..Default::default()
                                },
                            ),
                            (
                                "right-margin-max",
                                jade_pptx_writer::ParagraphFormat {
                                    margin_right_pt: Some(4032.0),
                                    ..Default::default()
                                },
                            ),
                            (
                                "bullet",
                                jade_pptx_writer::ParagraphFormat {
                                    bullet: Some(jade_pptx_writer::ParagraphBullet::Character {
                                        character: "•".into(),
                                        font: None,
                                    }),
                                    ..Default::default()
                                },
                            ),
                            (
                                "numbered",
                                jade_pptx_writer::ParagraphFormat {
                                    bullet: Some(jade_pptx_writer::ParagraphBullet::Numbered {
                                        scheme: "arabicPeriod".into(),
                                        start_at: Some(5),
                                    }),
                                    ..Default::default()
                                },
                            ),
                            (
                                "no-bullet",
                                jade_pptx_writer::ParagraphFormat {
                                    bullet: Some(jade_pptx_writer::ParagraphBullet::None),
                                    ..Default::default()
                                },
                            ),
                            (
                                "spacing-indent",
                                jade_pptx_writer::ParagraphFormat {
                                    level: Some(2),
                                    margin_left_pt: Some(36.0),
                                    indent_pt: Some(-18.0),
                                    line_spacing: Some(
                                        jade_pptx_writer::ParagraphSpacing::Percent(150.0),
                                    ),
                                    space_before: Some(jade_pptx_writer::ParagraphSpacing::Points(
                                        6.0,
                                    )),
                                    space_after: Some(jade_pptx_writer::ParagraphSpacing::Points(
                                        12.0,
                                    )),
                                    ..Default::default()
                                },
                            ),
                            (
                                "maximum",
                                jade_pptx_writer::ParagraphFormat {
                                    level: Some(8),
                                    margin_left_pt: Some(4032.0),
                                    indent_pt: Some(-4032.0),
                                    line_spacing: Some(jade_pptx_writer::ParagraphSpacing::Points(
                                        1584.0,
                                    )),
                                    space_before: Some(
                                        jade_pptx_writer::ParagraphSpacing::Percent(13200.0),
                                    ),
                                    space_after: Some(jade_pptx_writer::ParagraphSpacing::Points(
                                        1584.0,
                                    )),
                                    bullet: Some(jade_pptx_writer::ParagraphBullet::Numbered {
                                        scheme: "romanUcPeriod".into(),
                                        start_at: Some(32767),
                                    }),
                                    ..Default::default()
                                },
                            ),
                        ] {
                            let mut session = EditSession::open(&bytes)?;
                            if !session.format_paragraphs(
                                slide,
                                &jade_pptx_writer::ParagraphFormatEdit {
                                    shape_id: object.id,
                                    start: 0,
                                    end: length,
                                    format,
                                },
                            )? {
                                continue;
                            }
                            let scenario =
                                format!("slide-{slide:03}-paragraph-{:03}-{mode}", object.id);
                            write(output, &scenario, filename, session.bytes())?;
                            assert!(session.undo());
                            assert_eq!(session.bytes(), bytes);
                            assert!(session.redo());
                            write(
                                output,
                                &format!("{scenario}-redo"),
                                filename,
                                session.bytes(),
                            )?;
                            count += 2;
                        }
                        {
                            let mut session = EditSession::open(&bytes)?;
                            let mut expected = snapshot.text.clone();
                            let mut at = length;
                            for text in ["入力", "🦀", "続き"] {
                                let edit = jade_pptx_writer::TextInputEdit {
                                    shape_id: object.id,
                                    start: at,
                                    end: at,
                                    text: text.into(),
                                    expected_text: expected.clone(),
                                    soft_break: false,
                                    history: Some(jade_pptx_writer::TypingKind::Insert),
                                    format: jade_pptx_writer::TextFormat::default(),
                                };
                                session.apply_text_input(slide, &edit)?;
                                at += text.encode_utf16().count();
                                expected.push_str(text);
                            }
                            assert_eq!(session.history_depth(), (1, 0));
                            let scenario = format!("slide-{slide:03}-typing-{:03}", object.id);
                            write(output, &scenario, filename, session.bytes())?;
                            assert!(session.undo());
                            assert_eq!(session.bytes(), bytes);
                            assert!(session.redo());
                            write(
                                output,
                                &format!("{scenario}-redo"),
                                filename,
                                session.bytes(),
                            )?;
                            count += 2;
                        }
                        let mut session = EditSession::open(&bytes)?;
                        session.set_shape_text_formatted(
                            slide,
                            object.id,
                            &format!("{}\n書式つき入力", snapshot.text),
                            false,
                            &jade_pptx_writer::TextFormat {
                                bold: Some(true),
                                underline: Some(true),
                                size_pt: Some(22.75),
                                ..Default::default()
                            },
                        )?;
                        let scenario = format!("slide-{slide:03}-format-{:03}-typing", object.id);
                        write(output, &scenario, filename, session.bytes())?;
                        assert!(session.undo());
                        assert_eq!(session.bytes(), bytes);
                        assert!(session.redo());
                        write(
                            output,
                            &format!("{scenario}-redo"),
                            filename,
                            session.bytes(),
                        )?;
                        count += 2;
                        let mut scenarios = vec![
                            ("paragraph", length, length, "\n次の段落", false),
                            ("soft-break", length, length, "\n改行", true),
                            ("replace-all", 0, length, "置換 & <PPTX> 🦀\n次", false),
                            ("delete-all", 0, length, "", false),
                        ];
                        if let Some(byte) = snapshot.text.find('\n') {
                            let offset = snapshot.text[..byte].encode_utf16().count();
                            scenarios.push(("join", offset, offset + 1, "", false));
                        }
                        for (mode, start, end, text, soft_break) in scenarios {
                            let mut session = EditSession::open(&bytes)?;
                            if !session.replace_text_range(
                                slide,
                                &jade_pptx_writer::TextRangeEdit {
                                    shape_id: object.id,
                                    start,
                                    end,
                                    text: text.into(),
                                    soft_break,
                                },
                            )? {
                                continue;
                            }
                            let scenario = format!("slide-{slide:03}-text-{:03}-{mode}", object.id);
                            write(output, &scenario, filename, session.bytes())?;
                            assert!(session.undo());
                            assert_eq!(session.bytes(), bytes);
                            assert!(session.redo());
                            write(
                                output,
                                &format!("{scenario}-redo"),
                                filename,
                                session.bytes(),
                            )?;
                            count += 2;
                        }
                    }
                    Ok(_) | Err(jade_pptx_writer::WriteError::NoTextBody(_)) => {}
                    Err(error) => return Err(error.into()),
                }
                let before = object.transform.or_else(|| {
                    let shape = layout.shapes.iter().find(|shape| {
                        shape.origin == jade_pptx_layout::PlacedOrigin::Slide
                            && shape.source_id == Some(object.id)
                    })?;
                    Some(jade_pptx_reader::Transform {
                        offset_emu: (
                            (shape.rect.x * 9525.0).round() as i64,
                            (shape.rect.y * 9525.0).round() as i64,
                        ),
                        extent_emu: (
                            (shape.rect.width * 9525.0).round() as i64,
                            (shape.rect.height * 9525.0).round() as i64,
                        ),
                        rotation_60000ths: (shape.rotation_rad.to_degrees() * 60000.0).round()
                            as i32,
                        flip_h: shape.flip_h,
                        flip_v: shape.flip_v,
                    })
                });
                let Some(before) = before else {
                    continue;
                };
                for mode in ["move", "resize", "rotate", "flip"] {
                    if !object.can_select
                        || (mode == "move" && !object.can_move)
                        || (mode == "resize" && !object.can_resize)
                        || (matches!(mode, "rotate" | "flip") && !object.can_rotate)
                    {
                        continue;
                    }
                    let mut after = before;
                    match mode {
                        "move" => {
                            after.offset_emu.0 += 9525;
                            after.offset_emu.1 += 19050;
                        }
                        "resize" => {
                            if object.lock_aspect_ratio {
                                after.extent_emu.0 =
                                    (before.extent_emu.0 as f64 * 1.1).round() as i64;
                                after.extent_emu.1 =
                                    (before.extent_emu.1 as f64 * 1.1).round() as i64;
                            } else {
                                after.extent_emu.0 += 9525;
                                after.extent_emu.1 += 9525;
                            }
                        }
                        "rotate" => after.rotation_60000ths = 2700000,
                        "flip" => after.flip_h = !after.flip_h,
                        _ => unreachable!(),
                    }
                    let mut session = EditSession::open(&bytes)?;
                    if !session.set_shape_transform(slide, object.id, after)? {
                        continue;
                    }
                    let scenario = format!("slide-{slide:03}-shape-{:03}-{mode}", object.id);
                    write(output, &scenario, filename, session.bytes())?;
                    assert!(session.undo());
                    assert_eq!(session.bytes(), bytes);
                    assert!(session.redo());
                    write(
                        output,
                        &format!("{scenario}-redo"),
                        filename,
                        session.bytes(),
                    )?;
                    count += 2;
                }
            }
            for (index, run) in editable_runs(&bytes, slide)?.iter().enumerate() {
                let mut session = EditSession::open(&bytes)?;
                let replacement = format!("互換テスト & <PPTX> 🦀 {}", run.text);
                session.set_run_text(slide, run.address, &replacement)?;
                let scenario = format!("slide-{slide:03}-run-{index:03}");
                write(output, &scenario, filename, session.bytes())?;
                assert!(session.undo());
                assert_eq!(session.bytes(), bytes);
                assert!(session.redo());
                write(
                    output,
                    &format!("{scenario}-redo"),
                    filename,
                    session.bytes(),
                )?;
                count += 2;
            }
        }
        count += 1;
    }
    if count == 0 {
        return Err("no PPTX fixtures were checked".into());
    }
    println!("PPTX edit sweep: {count} packages");
    Ok(())
}
