use crate::FontData;
use lopdf::{dictionary, Document, Object, ObjectId, Stream};
use skia_safe::Rect;
use std::collections::BTreeMap;

/// Real font/glyph commands culled by Skia's rectangular clip. They remain
/// clipped in the PDF too, but retain Word's searchable, unclipped geometry.
pub(crate) struct Run {
    pub font: FontData,
    pub font_id: u32,
    pub postscript_name: String,
    pub text: String,
    pub glyphs: Vec<u16>,
    pub widths: Vec<f32>,
    pub font_size: f32,
    pub transform: [f32; 6],
    pub clip: Rect,
}

pub(crate) fn has_truetype_outlines(font: &FontData) -> bool {
    let bytes = &font.0;
    let offset = if bytes.get(..4) == Some(b"ttcf") {
        let at = 12 + font.1 as usize * 4;
        let Some(value) = bytes
            .get(at..at + 4)
            .and_then(|v| <[u8; 4]>::try_from(v).ok())
        else {
            return false;
        };
        u32::from_be_bytes(value) as usize
    } else {
        if font.1 != 0 {
            return false;
        }
        0
    };
    matches!(bytes.get(offset..offset + 4), Some(b"\0\x01\0\0" | b"true"))
}

fn hex_unicode(ch: char) -> String {
    ch.encode_utf16(&mut [0; 2])
        .iter()
        .map(|u| format!("{u:04X}"))
        .collect()
}

pub(crate) fn append(
    pdf: &mut Document,
    page_id: ObjectId,
    height: f64,
    runs: &[Run],
) -> Result<(), String> {
    if runs.is_empty() {
        return Ok(());
    }
    let mut groups = BTreeMap::<u32, Vec<&Run>>::new();
    for run in runs {
        groups.entry(run.font_id).or_default().push(run);
    }
    let mut resources_fonts = lopdf::Dictionary::new();
    let mut fonts = BTreeMap::new();
    for (index, (font_id, group)) in groups.iter().enumerate() {
        let first = group[0];
        let program = crate::sfnt_from_collection(&first.font.0, first.font.1 as usize)
            .ok_or("could not retain clipped TrueType font")?;
        let face =
            rustybuzz::Face::from_slice(&program, 0).ok_or("invalid clipped TrueType font")?;
        let scale = 1000.0 / f64::from(face.units_per_em());
        let bounds = face.global_bounding_box();
        let bbox = vec![
            Object::Real((f64::from(bounds.x_min) * scale) as f32),
            Object::Real((f64::from(bounds.y_min) * scale) as f32),
            Object::Real((f64::from(bounds.x_max) * scale) as f32),
            Object::Real((f64::from(bounds.y_max) * scale) as f32),
        ];
        let ascent = (f64::from(face.ascender()) * scale) as f32;
        let descent = (f64::from(face.descender()) * scale) as f32;
        let mut codes = BTreeMap::<(u16, char, u32), u16>::new();
        for run in group {
            for ((glyph, ch), width) in run.glyphs.iter().zip(run.text.chars()).zip(&run.widths) {
                let key = (*glyph, ch, (*width / run.font_size * 1000.0).to_bits());
                let next_code = codes.len() + 1;
                if let std::collections::btree_map::Entry::Vacant(entry) = codes.entry(key) {
                    let code = u16::try_from(next_code)
                        .map_err(|_| "clipped font CID capacity exceeded")?;
                    entry.insert(code);
                }
            }
        }
        let mut gids = vec![0; (codes.len() + 1) * 2];
        let mut widths = Vec::new();
        let mut mappings = Vec::new();
        let mut ordered = codes.iter().collect::<Vec<_>>();
        ordered.sort_by_key(|(_, code)| **code);
        for ((glyph, ch, width), code) in ordered {
            gids[*code as usize * 2..*code as usize * 2 + 2].copy_from_slice(&glyph.to_be_bytes());
            widths.push(Object::Integer(i64::from(*code)));
            widths.push(Object::Array(vec![Object::Real(f32::from_bits(*width))]));
            mappings.push(format!("<{code:04X}> <{}>", hex_unicode(*ch)));
        }
        let mut cmap = String::from("/CIDInit /ProcSet findresource begin\n12 dict begin\nbegincmap\n/CIDSystemInfo << /Registry (Adobe) /Ordering (UCS) /Supplement 0 >> def\n/CMapName /JadeClippedUnicode def\n/CMapType 2 def\n1 begincodespacerange\n<0000> <FFFF>\nendcodespacerange\n");
        for chunk in mappings.chunks(100) {
            cmap.push_str(&format!(
                "{} beginbfchar\n{}\nendbfchar\n",
                chunk.len(),
                chunk.join("\n")
            ));
        }
        cmap.push_str("endcmap\nCMapName currentdict /CMap defineresource pop\nend\nend\n");
        let file = pdf.add_object(Stream::new(
            dictionary! { "Length1" => program.len() as i64 },
            program,
        ));
        let descriptor = pdf.add_object(dictionary! {
            "Type" => "FontDescriptor", "FontName" => Object::Name(first.postscript_name.as_bytes().to_vec()),
            "Flags" => 32, "FontBBox" => bbox, "ItalicAngle" => 0,
            "Ascent" => ascent, "Descent" => descent, "CapHeight" => ascent,
            "StemV" => 80, "FontFile2" => file,
        });
        let gid_map = pdf.add_object(Stream::new(lopdf::Dictionary::new(), gids));
        let cid_font = pdf.add_object(dictionary! {
            "Type" => "Font", "Subtype" => "CIDFontType2",
            "BaseFont" => Object::Name(first.postscript_name.as_bytes().to_vec()),
            "CIDSystemInfo" => dictionary! { "Registry" => Object::string_literal("Adobe"), "Ordering" => Object::string_literal("Identity"), "Supplement" => 0 },
            "FontDescriptor" => descriptor, "CIDToGIDMap" => gid_map,
            "DW" => 1000, "W" => widths,
        });
        let unicode = pdf.add_object(Stream::new(lopdf::Dictionary::new(), cmap.into_bytes()));
        let font = pdf.add_object(dictionary! {
            "Type" => "Font", "Subtype" => "Type0",
            "BaseFont" => Object::Name(first.postscript_name.as_bytes().to_vec()),
            "Encoding" => "Identity-H", "DescendantFonts" => vec![Object::Reference(cid_font)], "ToUnicode" => unicode,
        });
        let name = format!("JadeClipped{index}");
        resources_fonts.set(name.as_bytes(), font);
        fonts.insert(*font_id, (name, codes));
    }
    let page = pdf
        .get_object(page_id)
        .and_then(Object::as_dict)
        .map_err(|e| e.to_string())?;
    let mut resources = page
        .get(b"Resources")
        .ok()
        .and_then(|v| pdf.dereference(v).ok())
        .and_then(|(_, v)| v.as_dict().ok())
        .cloned()
        .unwrap_or_default();
    let mut existing = resources
        .get(b"Font")
        .ok()
        .and_then(|v| pdf.dereference(v).ok())
        .and_then(|(_, v)| v.as_dict().ok())
        .cloned()
        .unwrap_or_default();
    for (name, font) in resources_fonts.iter() {
        existing.set(name.clone(), font.clone());
    }
    resources.set("Font", existing);
    let mut commands = String::new();
    for run in runs {
        let (name, codes) = &fonts[&run.font_id];
        let clip = run.clip;
        commands.push_str(&format!(
            "q\n{} {} {} {} re W n\nBT\n3 Tr\n/{name} {} Tf\n",
            f64::from(clip.left()) * 0.75,
            height - f64::from(clip.bottom()) * 0.75,
            f64::from(clip.width()) * 0.75,
            f64::from(clip.height()) * 0.75,
            run.font_size
        ));
        let [a, b, c, d, e, f] = run.transform.map(f64::from);
        let mut advance = 0.0;
        for ((glyph, ch), width) in run.glyphs.iter().zip(run.text.chars()).zip(&run.widths) {
            let code = codes[&(*glyph, ch, (*width / run.font_size * 1000.0).to_bits())];
            commands.push_str(&format!(
                "{} {} {} {} {} {} Tm\n<{code:04X}> Tj\n",
                a * 0.75,
                -b * 0.75,
                -c * 0.75,
                d * 0.75,
                (e + a * advance) * 0.75,
                height - (f + b * advance) * 0.75
            ));
            advance += f64::from(*width);
        }
        commands.push_str("ET\nQ\n");
    }
    let mut content = pdf.get_page_content(page_id).map_err(|e| e.to_string())?;
    content.extend(commands.into_bytes());
    let stream = pdf.add_object(Stream::new(lopdf::Dictionary::new(), content));
    let page = pdf
        .get_object_mut(page_id)
        .and_then(Object::as_dict_mut)
        .map_err(|e| e.to_string())?;
    page.set("Contents", stream);
    page.set("Resources", resources);
    Ok(())
}
