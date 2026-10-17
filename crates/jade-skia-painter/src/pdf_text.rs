use lopdf::{dictionary, Document, Object, ObjectId};
use std::collections::BTreeMap;

/// Searchable counterparts of text whose visual effects require glyph paths.
pub(crate) struct RecordedText {
    pub text: String,
    pub font_size: f32,
    pub widths: Vec<f32>,
    pub ascent: f32,
    pub descent: f32,
    pub transform: [f32; 6],
}

fn unicode(text: &str, bom: bool) -> Vec<u8> {
    let mut bytes = if bom { vec![0xfe, 0xff] } else { Vec::new() };
    for unit in text.encode_utf16() {
        bytes.extend(unit.to_be_bytes());
    }
    bytes
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02X}")).collect()
}

pub(crate) fn append(
    pdf: &mut Document,
    page_id: ObjectId,
    height: f64,
    runs: &[RecordedText],
) -> Result<(), String> {
    if runs.is_empty() {
        return Ok(());
    }
    let key = |ch: char, width: f32, run: &RecordedText| {
        (
            ch,
            (width / run.font_size * 1000.0).to_bits(),
            (run.ascent / run.font_size * 1000.0).to_bits(),
            (run.descent / run.font_size * 1000.0).to_bits(),
        )
    };
    let mut keys = BTreeMap::new();
    for run in runs {
        for (ch, width) in run.text.chars().zip(&run.widths) {
            let key = key(ch, *width, run);
            keys.insert(
                key,
                (
                    f32::from_bits(key.1),
                    f32::from_bits(key.2),
                    f32::from_bits(key.3),
                ),
            );
        }
    }
    let mut metrics = BTreeMap::<_, Vec<_>>::new();
    for (key, value) in keys {
        metrics
            .entry((key.2, key.3))
            .or_default()
            .push((key, value));
    }
    let mut codes = BTreeMap::new();
    let mut fonts = lopdf::Dictionary::new();
    for group in metrics.values().flat_map(|entries| entries.chunks(255)) {
        let index = fonts.len();
        let name = format!("JadeEffectText{index}");
        let mut procs = lopdf::Dictionary::new();
        // An unused conventional 'm' supplies the em scale to Type3 readers
        // which otherwise estimate it from the first arbitrary glyph width.
        let em_proc = pdf.add_object(lopdf::Stream::new(
            lopdf::Dictionary::new(),
            b"600 0 0 0 600 0 d1\n".to_vec(),
        ));
        procs.set("m", em_proc);
        let mut differences = vec![Object::Integer(0), Object::Name(b"m".to_vec())];
        let mut widths = vec![Object::Integer(600)];
        let (mut max_width, mut max_ascent, mut max_descent) = (0.0_f32, 0.0_f32, 0.0_f32);
        let mut mappings = vec!["<00> <006D>".to_string()];
        for (code, (key, (width, ascent, descent))) in group.iter().enumerate() {
            let code = code + 1;
            let glyph_name = format!("g{code}");
            let procedure = format!("{width} 0 0 {} {width} {ascent} d1\n", -descent);
            let procedure_id = pdf.add_object(lopdf::Stream::new(
                lopdf::Dictionary::new(),
                procedure.into_bytes(),
            ));
            procs.set(glyph_name.clone(), procedure_id);
            differences.push(Object::Name(glyph_name.into_bytes()));
            widths.push(Object::Real(*width));
            max_width = max_width.max(*width);
            max_ascent = max_ascent.max(*ascent);
            max_descent = max_descent.max(*descent);
            mappings.push(format!(
                "<{code:02X}> <{}>",
                hex(&unicode(&key.0.to_string(), false))
            ));
            codes.insert(*key, (name.clone(), code));
        }
        let mut cmap = String::from("/CIDInit /ProcSet findresource begin\n12 dict begin\nbegincmap\n/CIDSystemInfo << /Registry (Adobe) /Ordering (UCS) /Supplement 0 >> def\n/CMapName /JadeUnicode def\n/CMapType 2 def\n1 begincodespacerange\n<00> <FF>\nendcodespacerange\n");
        for chunk in mappings.chunks(100) {
            cmap.push_str(&format!(
                "{} beginbfchar\n{}\nendbfchar\n",
                chunk.len(),
                chunk.join("\n")
            ));
        }
        cmap.push_str("endcmap\nCMapName currentdict /CMap defineresource pop\nend\nend\n");
        let cmap = pdf.add_object(lopdf::Stream::new(
            lopdf::Dictionary::new(),
            cmap.into_bytes(),
        ));
        let bounds = vec![
            Object::Real(0.0),
            Object::Real(-max_descent),
            Object::Real(max_width),
            Object::Real(max_ascent),
        ];
        let descriptor = pdf.add_object(dictionary! {
            "Type" => "FontDescriptor", "FontName" => Object::Name(name.as_bytes().to_vec()),
            "Flags" => 4, "FontBBox" => bounds.clone(), "ItalicAngle" => 0,
            "Ascent" => max_ascent, "Descent" => -max_descent,
            "CapHeight" => max_ascent, "StemV" => 0,
        });
        let font = pdf.add_object(dictionary! {
            "Type" => "Font", "Subtype" => "Type3",
            "FontBBox" => bounds, "FontDescriptor" => descriptor,
            "FontMatrix" => vec![Object::Real(0.001), 0.into(), 0.into(), Object::Real(0.001), 0.into(), 0.into()],
            "CharProcs" => procs, "Encoding" => dictionary! { "Type" => "Encoding", "Differences" => differences },
            "FirstChar" => 0, "LastChar" => group.len() as i64, "Widths" => widths,
            "Resources" => lopdf::Dictionary::new(), "ToUnicode" => cmap,
        });
        fonts.set(name, font);
    }
    let page = pdf
        .get_object(page_id)
        .and_then(Object::as_dict)
        .map_err(|error| error.to_string())?;
    let mut resources = page
        .get(b"Resources")
        .ok()
        .and_then(|value| pdf.dereference(value).ok())
        .and_then(|(_, value)| value.as_dict().ok())
        .cloned()
        .unwrap_or_default();
    let mut existing_fonts = resources
        .get(b"Font")
        .ok()
        .and_then(|value| pdf.dereference(value).ok())
        .and_then(|(_, value)| value.as_dict().ok())
        .cloned()
        .unwrap_or_default();
    for (name, font) in fonts.iter() {
        existing_fonts.set(name.clone(), font.clone());
    }
    resources.set("Font", existing_fonts);
    let mut commands = String::from("q\nBT\n3 Tr\n");
    for run in runs {
        commands.push_str(&format!(
            "/Span << /ActualText <{}> >> BDC\n",
            hex(&unicode(&run.text, true))
        ));
        let [a, b, c, d, e, f] = run.transform.map(f64::from);
        let mut advance = 0.0_f64;
        for (ch, width) in run.text.chars().zip(&run.widths) {
            let (name, code) = &codes[&key(ch, *width, run)];
            commands.push_str(&format!(
                "/{name} {} Tf\n{} {} {} {} {} {} Tm\n<{code:02X}> Tj\n",
                run.font_size,
                a * 0.75,
                -b * 0.75,
                -c * 0.75,
                d * 0.75,
                (e + a * advance) * 0.75,
                height - (f + b * advance) * 0.75
            ));
            advance += f64::from(*width);
        }
        commands.push_str("EMC\n");
    }
    commands.push_str("ET\nQ\n");
    let mut content = pdf
        .get_page_content(page_id)
        .map_err(|error| error.to_string())?;
    content.extend(commands.into_bytes());
    let stream = pdf.add_object(lopdf::Stream::new(lopdf::Dictionary::new(), content));
    let page = pdf
        .get_object_mut(page_id)
        .and_then(Object::as_dict_mut)
        .map_err(|error| error.to_string())?;
    page.set("Contents", stream);
    page.set("Resources", resources);
    Ok(())
}
