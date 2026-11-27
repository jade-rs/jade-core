//! macOS provides a colour glyph for joined emoji as one OpenType cluster.
use super::{with_font_mgr, Font, JoinedGlyph, Typeface};
use rustybuzz::ttf_parser::{Face, RawFaceTables};
use skia_safe::{Data, FontStyle};

struct EmojiFont {
    typeface: Typeface,
    head: Data,
    hhea: Data,
    maxp: Data,
    cmap: Data,
    hmtx: Data,
    gsub: Option<Data>,
    gdef: Option<Data>,
    gpos: Option<Data>,
    morx: Option<Data>,
    feat: Option<Data>,
    trak: Option<Data>,
}

impl EmojiFont {
    fn load() -> Option<Self> {
        let typeface = with_font_mgr(|manager| {
            manager.match_family_style("Apple Color Emoji", FontStyle::normal())
        })?;
        let table = |tag| typeface.copy_table_data(u32::from_be_bytes(tag));
        // Use only the shaping tables here. CoreText reads the bitmap for
        // painting; copying the full font would retain every strike in memory.
        Some(Self {
            head: table(*b"head")?,
            hhea: table(*b"hhea")?,
            maxp: table(*b"maxp")?,
            cmap: table(*b"cmap")?,
            hmtx: table(*b"hmtx")?,
            gsub: table(*b"GSUB"),
            gdef: table(*b"GDEF"),
            gpos: table(*b"GPOS"),
            morx: table(*b"morx"),
            feat: table(*b"feat"),
            trak: table(*b"trak"),
            typeface,
        })
    }

    fn shape(&self, text: &str, size: f32) -> Option<(Font, JoinedGlyph)> {
        let face = Face::from_raw_tables(RawFaceTables {
            head: self.head.as_bytes(),
            hhea: self.hhea.as_bytes(),
            maxp: self.maxp.as_bytes(),
            cmap: Some(self.cmap.as_bytes()),
            hmtx: Some(self.hmtx.as_bytes()),
            gsub: self.gsub.as_ref().map(Data::as_bytes),
            gdef: self.gdef.as_ref().map(Data::as_bytes),
            gpos: self.gpos.as_ref().map(Data::as_bytes),
            morx: self.morx.as_ref().map(Data::as_bytes),
            feat: self.feat.as_ref().map(Data::as_bytes),
            trak: self.trak.as_ref().map(Data::as_bytes),
            ..Default::default()
        })
        .ok()?;
        let face = rustybuzz::Face::from_face(face);
        let mut input = rustybuzz::UnicodeBuffer::new();
        input.push_str(text);
        input.guess_segment_properties();
        let shaped = rustybuzz::shape(&face, &[], input);
        let [glyph] = shaped.glyph_infos() else {
            return None;
        };
        let [position] = shaped.glyph_positions() else {
            return None;
        };
        let id = u16::try_from(glyph.glyph_id).ok().filter(|&id| id != 0)?;
        let advance = size * position.x_advance as f32 / face.units_per_em() as f32;
        let mut font = Font::from_typeface(self.typeface.clone(), size);
        let native_width = font.measure_text(&[id][..], None).0;
        if !(advance.is_finite() && advance > 0.0 && native_width.is_finite() && native_width > 0.0)
        {
            return None;
        }
        // CoreText draws the bitmap larger than its OpenType advance. Fit the
        // image to the layout cell and align its bottom with the text baseline.
        font.set_size(size * advance / native_width);
        let ink = font.measure_text(&[id][..], None).1;
        Some((
            font,
            JoinedGlyph {
                id,
                advance,
                baseline_shift: ink.bottom.max(0.0),
            },
        ))
    }
}

pub(super) fn shape(text: &str, size: f32) -> Option<(Font, JoinedGlyph)> {
    if !(matches!(
        text.chars().next().map(u32::from),
        Some(0x1f000..=0x1faff | 0x2600..=0x27bf)
    ) && text.contains('\u{200d}'))
    {
        return None;
    }
    thread_local! {
        static EMOJI: Option<EmojiFont> = EmojiFont::load();
    }
    EMOJI.with(|font| font.as_ref()?.shape(text, size))
}
