//! Ink, video and audio edits: the 描画 tab's strokes, inserting video and
//! audio, the 再生 tab's options (start, volume, loop, rewind, hiding, full
//! screen, playing across slides), trimming, fades, bookmarks, poster
//! frames, and swapping the media behind a picture (compressing,
//! converting, embedding a linked file).
//!
//! Each is one rewrite of the package; [`EditSession`]'s methods make each
//! one step of Undo.

pub use crate::xml::media::{
    media_content_type, CaptionInsert, CaptionSource, InkEdit, MediaCuts, MediaSource,
};
use crate::{slide_xml, xml, EditSession, Result, WriteError};
use jade_pptx_reader::{MediaKind, MediaPlayback, Transform};

/// Applies ink edits to slide `slide`. The new drawings' ids come back.
pub fn edit_ink(bytes: &[u8], slide: usize, edits: &[InkEdit]) -> Result<(Vec<u8>, Vec<u32>)> {
    let (mut package, part, slide_xml) = slide_xml(bytes, slide)?;
    let (changed, created) = xml::media::edit_ink(&mut package, &part, &slide_xml, edits)?;
    package.replace(&part, changed.into_bytes());
    let result = package.write()?;
    jade_pptx_reader::read_deck(&result)?;
    Ok((result, created))
}

/// インクを図形に変換's line or arrow on slide `slide`, from `from` to `to`
/// (slide px). Its id comes back.
pub fn insert_line(
    bytes: &[u8],
    slide: usize,
    from: (f64, f64),
    to: (f64, f64),
    color: &str,
    width_pt: f64,
    arrow: bool,
) -> Result<(Vec<u8>, u32)> {
    let (mut package, part, slide_xml) = slide_xml(bytes, slide)?;
    let (changed, id) = xml::media::insert_line(&slide_xml, from, to, color, width_pt, arrow)?;
    package.replace(&part, changed.into_bytes());
    let result = package.write()?;
    jade_pptx_reader::read_deck(&result)?;
    Ok((result, id))
}

/// A video or audio to insert.
#[derive(Debug, Clone, PartialEq)]
pub struct MediaInsert<'a> {
    pub kind: MediaKind,
    pub source: MediaSource<'a>,
    /// The poster frame (video) or icon (audio): PNG or JPEG.
    pub poster: &'a [u8],
    pub transform: Transform,
    /// How long the media plays, when known: what PowerPoint times the
    /// play effect with.
    pub duration_ms: Option<f64>,
}

/// Inserts video or audio, playing as PowerPoint plays new media: in the
/// click sequence, at 80% volume. Its id comes back.
pub fn insert_media(bytes: &[u8], slide: usize, media: &MediaInsert<'_>) -> Result<(Vec<u8>, u32)> {
    crate::validate_transform(media.transform)?;
    if media.transform.extent_emu.0 <= 0 || media.transform.extent_emu.1 <= 0 {
        return Err(WriteError::InvalidTransform);
    }
    if let MediaSource::Linked { target } = media.source {
        if target.is_empty() || target.chars().any(|c| c.is_control()) {
            return Err(WriteError::InvalidPicture("invalid media link".into()));
        }
    }
    let (mut package, part, slide_xml) = slide_xml(bytes, slide)?;
    let (changed, id) = xml::media::insert_media(
        &mut package,
        &part,
        &slide_xml,
        media.kind,
        &media.source,
        media.poster,
        media.transform,
        media.duration_ms,
    )?;
    package.replace(&part, changed.into_bytes());
    let result = package.write()?;
    jade_pptx_reader::read_deck(&result)?;
    Ok((result, id))
}

/// The media shapes' playback options on slide `slide`.
pub fn media_playback(bytes: &[u8], slide: usize) -> Result<Vec<MediaPlayback>> {
    let (_, _, slide_xml) = slide_xml(bytes, slide)?;
    Ok(jade_pptx_reader::parse_media_playback(&slide_xml))
}

/// Writes how media `playback.shape_id` plays (the 再生 tab), replacing
/// its timing and leaving other animations alone.
pub fn set_media_playback(
    bytes: &[u8],
    slide: usize,
    playback: &MediaPlayback,
    duration_ms: Option<f64>,
) -> Result<Vec<u8>> {
    let (mut package, part, slide_xml) = slide_xml(bytes, slide)?;
    let changed = xml::media::set_playback(&slide_xml, playback, duration_ms)?;
    package.replace(&part, changed.into_bytes());
    let result = package.write()?;
    jade_pptx_reader::read_deck(&result)?;
    Ok(result)
}

/// Writes a media picture's trim, fades and bookmarks.
pub fn set_media_cuts(bytes: &[u8], slide: usize, id: u32, cuts: &MediaCuts) -> Result<Vec<u8>> {
    let (mut package, part, slide_xml) = slide_xml(bytes, slide)?;
    let changed = xml::media::set_media_cuts(&slide_xml, id, cuts)?;
    package.replace(&part, changed.into_bytes());
    let result = package.write()?;
    jade_pptx_reader::read_deck(&result)?;
    Ok(result)
}

/// 表紙画像: a new poster frame for a video (or icon for audio).
pub fn set_media_poster(bytes: &[u8], slide: usize, id: u32, image: &[u8]) -> Result<Vec<u8>> {
    let (mut package, part, slide_xml) = slide_xml(bytes, slide)?;
    let changed = xml::media::set_poster(&mut package, &part, &slide_xml, id, image)?;
    package.replace(&part, changed.into_bytes());
    let result = package.write()?;
    jade_pptx_reader::read_deck(&result)?;
    Ok(result)
}

/// New media behind a media picture, keeping its poster, cuts and timing.
pub fn replace_media(
    bytes: &[u8],
    slide: usize,
    id: u32,
    kind: MediaKind,
    source: &MediaSource<'_>,
) -> Result<Vec<u8>> {
    let (mut package, part, slide_xml) = slide_xml(bytes, slide)?;
    let changed = xml::media::replace_media(&mut package, &part, &slide_xml, id, kind, source)?;
    package.replace(&part, changed.into_bytes());
    let result = package.write()?;
    jade_pptx_reader::read_deck(&result)?;
    Ok(result)
}

/// 字幕の挿入: adds a WebVTT caption track to media picture `id`; the
/// track's id comes back.
pub fn add_caption_track(
    bytes: &[u8],
    slide: usize,
    id: u32,
    track: &CaptionInsert<'_>,
) -> Result<(Vec<u8>, String)> {
    let (mut package, part, slide_xml) = slide_xml(bytes, slide)?;
    let (changed, track_id) =
        xml::media::add_caption_track(&mut package, &part, &slide_xml, id, track)?;
    package.replace(&part, changed.into_bytes());
    let result = package.write()?;
    jade_pptx_reader::read_deck(&result)?;
    Ok((result, track_id))
}

/// 字幕の削除: removes caption track `track_id` from media picture `id`.
pub fn remove_caption_track(
    bytes: &[u8],
    slide: usize,
    id: u32,
    track_id: &str,
) -> Result<Vec<u8>> {
    let (mut package, part, slide_xml) = slide_xml(bytes, slide)?;
    let changed = xml::media::remove_caption_track(&mut package, &part, &slide_xml, id, track_id)?;
    package.replace(&part, changed.into_bytes());
    let result = package.write()?;
    jade_pptx_reader::read_deck(&result)?;
    Ok(result)
}

/// Deletes a media picture with its timing.
pub fn delete_media(bytes: &[u8], slide: usize, id: u32) -> Result<Vec<u8>> {
    let (mut package, part, slide_xml) = slide_xml(bytes, slide)?;
    let untimed = xml::media::remove_playback(&slide_xml, id)?;
    let changed = xml::delete_shape(&untimed, id)?;
    package.replace(&part, changed.into_bytes());
    let result = package.write()?;
    jade_pptx_reader::read_deck(&result)?;
    Ok(result)
}

impl EditSession {
    pub fn insert_line(
        &mut self,
        slide: usize,
        from: (f64, f64),
        to: (f64, f64),
        color: &str,
        width_pt: f64,
        arrow: bool,
    ) -> Result<u32> {
        let (next, id) = insert_line(self.bytes(), slide, from, to, color, width_pt, arrow)?;
        self.commit(next);
        Ok(id)
    }

    /// [`edit_ink`] as one step of Undo.
    pub fn edit_ink(&mut self, slide: usize, edits: &[InkEdit]) -> Result<Vec<u32>> {
        let (next, created) = edit_ink(self.bytes(), slide, edits)?;
        self.commit(next);
        Ok(created)
    }

    /// Ink edits on several slides as one step of Undo.
    pub fn edit_ink_on_slides(&mut self, edits: &[(usize, Vec<InkEdit>)]) -> Result<bool> {
        let mut bytes = self.bytes().to_vec();
        for (slide, slide_edits) in edits {
            if !slide_edits.is_empty() {
                bytes = edit_ink(&bytes, *slide, slide_edits)?.0;
            }
        }
        Ok(self.commit(bytes))
    }

    pub fn insert_media(&mut self, slide: usize, media: &MediaInsert<'_>) -> Result<u32> {
        let (next, id) = insert_media(self.bytes(), slide, media)?;
        self.commit(next);
        Ok(id)
    }

    pub fn set_media_playback(
        &mut self,
        slide: usize,
        playback: &MediaPlayback,
        duration_ms: Option<f64>,
    ) -> Result<bool> {
        let next = set_media_playback(self.bytes(), slide, playback, duration_ms)?;
        Ok(self.commit(next))
    }

    pub fn set_media_cuts(&mut self, slide: usize, id: u32, cuts: &MediaCuts) -> Result<bool> {
        let next = set_media_cuts(self.bytes(), slide, id, cuts)?;
        Ok(self.commit(next))
    }

    pub fn set_media_poster(&mut self, slide: usize, id: u32, image: &[u8]) -> Result<bool> {
        let next = set_media_poster(self.bytes(), slide, id, image)?;
        Ok(self.commit(next))
    }

    pub fn replace_media(
        &mut self,
        slide: usize,
        id: u32,
        kind: MediaKind,
        source: &MediaSource<'_>,
    ) -> Result<bool> {
        let next = replace_media(self.bytes(), slide, id, kind, source)?;
        Ok(self.commit(next))
    }

    /// Several media swaps (メディアの圧縮 over the whole deck) as one step.
    pub fn replace_media_many(
        &mut self,
        swaps: &[(usize, u32, MediaKind, MediaSource<'_>)],
    ) -> Result<bool> {
        let mut bytes = self.bytes().to_vec();
        for (slide, id, kind, source) in swaps {
            bytes = replace_media(&bytes, *slide, *id, *kind, source)?;
        }
        Ok(self.commit(bytes))
    }

    pub fn delete_media(&mut self, slide: usize, id: u32) -> Result<bool> {
        let next = delete_media(self.bytes(), slide, id)?;
        Ok(self.commit(next))
    }

    /// 字幕の挿入, one step: the new track's id.
    pub fn add_caption_track(
        &mut self,
        slide: usize,
        id: u32,
        track: &CaptionInsert<'_>,
    ) -> Result<String> {
        let (next, track_id) = add_caption_track(self.bytes(), slide, id, track)?;
        self.commit(next);
        Ok(track_id)
    }

    /// 字幕の削除, one step.
    pub fn remove_caption_track(&mut self, slide: usize, id: u32, track_id: &str) -> Result<bool> {
        let next = remove_caption_track(self.bytes(), slide, id, track_id)?;
        Ok(self.commit(next))
    }
}
