//! Ink, video and audio on a slide.
//!
//! - Ink (描画): PowerPoint 2010 and later keep each ink drawing as a
//!   `p:contentPart` relating to an InkML part, inside an
//!   `mc:AlternateContent` whose `mc:Choice Requires="p14"` holds the
//!   content part and whose `mc:Fallback` (often left out) a picture of it.
//!   [`Ink`] is the drawing with its strokes read from the part.
//! - Video and audio: a `p:pic` whose `p:nvPr` has an `a:videoFile` or
//!   `a:audioFile` (`r:link` to the media) and, from PowerPoint 2010, a
//!   `p14:media` extension (`r:embed` for an embedded file, `r:link` for a
//!   linked one) with the trim, the fades and the bookmarks. The picture is
//!   the poster frame (or the speaker icon). [`Media`] is that, with the
//!   relationships resolved.
//! - How media plays lives in the slide's `p:timing`: a `p:video` or
//!   `p:audio` node per media shape (volume, loop, hiding, full screen,
//!   slides to play across) and the `mediacall` effects that start it.
//!   [`parse_media_playback`] reads those into [`MediaPlayback`].

use serde::{Deserialize, Serialize};

pub use jade_ink::inkml::{InkBrush, InkTool, InkTrace};

use crate::slide::Transform;
use crate::xml_tree::{parse_tree, XmlElement};

/// The `p:ext` URI PowerPoint 2010 media extensions sit under.
pub const MEDIA_EXT_URI: &str = "{DAA4B4D4-6D71-4841-9C94-3DE7FCFB9230}";
/// PowerPoint 2010's namespace.
pub const P14_NAMESPACE: &str = "http://schemas.microsoft.com/office/powerpoint/2010/main";
/// The relationship from a media shape to its media part (`p14:media`).
pub const MEDIA_RELATIONSHIP_TYPE: &str =
    "http://schemas.microsoft.com/office/2007/relationships/media";
/// The relationship `a:videoFile` points with.
pub const VIDEO_RELATIONSHIP_TYPE: &str =
    "http://schemas.openxmlformats.org/officeDocument/2006/relationships/video";
/// The relationship `a:audioFile` points with.
pub const AUDIO_RELATIONSHIP_TYPE: &str =
    "http://schemas.openxmlformats.org/officeDocument/2006/relationships/audio";
/// The relationship a `p:contentPart` points at its InkML part with.
pub const INK_RELATIONSHIP_TYPE: &str = jade_ink::inkml::INK_RELATIONSHIP_TYPE;

/// An ink drawing (`p:contentPart`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Ink {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<u32>,
    pub name: String,
    /// `p14:xfrm`: the box the strokes fill.
    pub transform: Option<Transform>,
    /// The content part's `r:id`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rel_id: Option<String>,
    /// The InkML part the id resolves to, filled in by `read_deck`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ink_part: Option<String>,
    /// The strokes, in px within the box (fitted to it), read by
    /// `read_deck`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub traces: Vec<InkTrace>,
}

/// Video or audio.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum MediaKind {
    Video,
    Audio,
}

/// A media bookmark (`p14:bmk`): a named time in the media.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MediaBookmark {
    pub name: String,
    /// From the media's start (not the trimmed start), in ms.
    pub time_ms: f64,
}

/// A video or audio shape's media: what it plays and the cuts made to it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Media {
    pub kind: MediaKind,
    /// `a:videoFile`/`a:audioFile r:link`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file_rel_id: Option<String>,
    /// `p14:media r:embed`: the embedded media.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub embed_rel_id: Option<String>,
    /// `p14:media r:link`: a linked file.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub link_rel_id: Option<String>,
    /// The embedded media part, filled in by `read_deck`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub media_part: Option<String>,
    /// A linked file's target as written (a `file:` URL or a path), filled
    /// in by `read_deck`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub linked_target: Option<String>,
    /// `p14:trim`: ms cut from the start and from the end.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trim_start_ms: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trim_end_ms: Option<f64>,
    /// `p14:fade`: fade-in and fade-out, in ms.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fade_in_ms: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fade_out_ms: Option<f64>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub bookmarks: Vec<MediaBookmark>,
}

impl Media {
    /// Whether the media is a linked file rather than one in the package.
    pub fn is_linked(&self) -> bool {
        self.embed_rel_id.is_none()
    }
}

fn time_ms(value: Option<&str>) -> Option<f64> {
    value
        .and_then(|value| value.trim().parse::<f64>().ok())
        .filter(|value| value.is_finite() && *value >= 0.0)
}

/// Reads the media of a `p:pic`'s `p:nvPr`, if it is video or audio.
pub(crate) fn parse_media(nv_pr: &XmlElement) -> Option<Media> {
    let (kind, file) = nv_pr
        .children
        .iter()
        .find_map(|child| match child.name.as_str() {
            "videoFile" | "quickTimeFile" => Some((MediaKind::Video, child)),
            "audioFile" | "wavAudioFile" => Some((MediaKind::Audio, child)),
            _ => None,
        })?;
    let p14 = nv_pr
        .child("extLst")
        .into_iter()
        .flat_map(|list| list.children.iter())
        .filter(|ext| ext.name == "ext")
        .find_map(|ext| ext.child("media"));
    let mut media = Media {
        kind,
        file_rel_id: file
            .attr("link")
            .or_else(|| file.attr("embed"))
            .map(str::to_string),
        embed_rel_id: p14.and_then(|m| m.attr("embed")).map(str::to_string),
        link_rel_id: p14.and_then(|m| m.attr("link")).map(str::to_string),
        media_part: None,
        linked_target: None,
        trim_start_ms: None,
        trim_end_ms: None,
        fade_in_ms: None,
        fade_out_ms: None,
        bookmarks: Vec::new(),
    };
    if let Some(p14) = p14 {
        if let Some(trim) = p14.child("trim") {
            media.trim_start_ms = time_ms(trim.attr("st")).filter(|ms| *ms > 0.0);
            media.trim_end_ms = time_ms(trim.attr("end")).filter(|ms| *ms > 0.0);
        }
        if let Some(fade) = p14.child("fade") {
            media.fade_in_ms = time_ms(fade.attr("in")).filter(|ms| *ms > 0.0);
            media.fade_out_ms = time_ms(fade.attr("out")).filter(|ms| *ms > 0.0);
        }
        if let Some(list) = p14.child("bmkLst") {
            media.bookmarks = list
                .children
                .iter()
                .filter(|bmk| bmk.name == "bmk")
                .filter_map(|bmk| {
                    Some(MediaBookmark {
                        name: bmk.attr("name")?.to_string(),
                        time_ms: time_ms(bmk.attr("time"))?,
                    })
                })
                .collect();
        }
    }
    Some(media)
}

/// Reads a `p:contentPart`, if it is one PowerPoint draws as ink.
pub(crate) fn parse_content_part(part: &XmlElement) -> Ink {
    let c_nv_pr = part.descendant("cNvPr");
    Ink {
        id: c_nv_pr
            .and_then(|c| c.attr("id"))
            .and_then(|id| id.parse().ok()),
        name: c_nv_pr
            .and_then(|c| c.attr("name"))
            .unwrap_or_default()
            .to_string(),
        transform: part.child("xfrm").map(crate::slide::parse_transform),
        rel_id: part.attr("id").map(str::to_string),
        ink_part: None,
        traces: Vec::new(),
    }
}

/// How a media shape starts in a slide show (再生 › 開始).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
pub enum MediaStart {
    /// 一連のクリック動作: with the slide's next click, in turn.
    #[default]
    InClickSequence,
    /// 自動: as soon as the slide shows (after what comes before it).
    Automatically,
    /// クリック時: when the media itself is clicked.
    WhenClicked,
}

/// How a media shape plays in a slide show: the 再生 tab's options, as the
/// slide's `p:timing` holds them.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MediaPlayback {
    /// The media shape (`p:spTgt spid`).
    pub shape_id: u32,
    pub kind: MediaKind,
    pub start: MediaStart,
    /// `cMediaNode vol`: 0 to 100000 (100%).
    pub volume: u32,
    pub mute: bool,
    /// 停止するまで繰り返す: the media node's `repeatCount="indefinite"`.
    pub loop_until_stopped: bool,
    /// 再生が終了したら巻き戻す: the media node's `fill="remove"`.
    pub rewind: bool,
    /// 全画面再生 (`p:video fullScrn`).
    pub full_screen: bool,
    /// 再生中のみ表示 / スライド ショーを実行中にサウンドのアイコンを隠す:
    /// `showWhenStopped="0"`.
    pub hide_when_not_playing: bool,
    /// How many slides it plays across (`numSld`): 1 stops it with the
    /// slide; 999 is スライド切り替え後も再生.
    pub slides: u32,
}

impl MediaPlayback {
    /// PowerPoint's settings for newly inserted media: in the click
    /// sequence, at 80% volume.
    pub fn new(shape_id: u32, kind: MediaKind) -> Self {
        Self {
            shape_id,
            kind,
            start: MediaStart::InClickSequence,
            volume: 80_000,
            mute: false,
            loop_until_stopped: false,
            rewind: false,
            full_screen: false,
            hide_when_not_playing: false,
            slides: 1,
        }
    }

    /// スライド切り替え後も再生.
    pub fn plays_across_slides(&self) -> bool {
        self.slides > 1
    }
}

fn target_spid(element: &XmlElement) -> Option<u32> {
    element
        .child("tgtEl")
        .and_then(|target| target.child("spTgt"))
        .and_then(|target| target.attr("spid"))
        .and_then(|id| id.parse().ok())
}

/// Walks every element under `element`, outermost first, with the chain
/// of its ancestors.
fn walk<'a>(
    element: &'a XmlElement,
    ancestors: &mut Vec<&'a XmlElement>,
    visit: &mut dyn FnMut(&'a XmlElement, &[&'a XmlElement]),
) {
    visit(element, ancestors);
    ancestors.push(element);
    for child in &element.children {
        walk(child, ancestors, visit);
    }
    ancestors.pop();
}

/// The media shapes' playback options in a slide's XML.
pub fn parse_media_playback(slide_xml: &str) -> Vec<MediaPlayback> {
    let Ok(root) = parse_tree(slide_xml) else {
        return Vec::new();
    };
    let Some(timing) = root.child("timing") else {
        return Vec::new();
    };
    let mut found: Vec<MediaPlayback> = Vec::new();
    // The media nodes.
    let mut nodes = Vec::new();
    walk(timing, &mut Vec::new(), &mut |element, _| {
        if matches!(element.name.as_str(), "video" | "audio") {
            nodes.push(element);
        }
    });
    for node in nodes {
        let Some(media) = node.child("cMediaNode") else {
            continue;
        };
        let Some(shape_id) = target_spid(media) else {
            continue;
        };
        let kind = if node.name == "video" {
            MediaKind::Video
        } else {
            MediaKind::Audio
        };
        let c_tn = media.child("cTn");
        found.push(MediaPlayback {
            shape_id,
            kind,
            start: MediaStart::WhenClicked,
            // ST_PositiveFixedPercentage, 50% by default.
            volume: media
                .attr("vol")
                .and_then(|vol| vol.parse::<u32>().ok())
                .unwrap_or(50_000)
                .min(100_000),
            mute: matches!(media.attr("mute"), Some("1" | "true")),
            loop_until_stopped: c_tn.and_then(|c| c.attr("repeatCount")) == Some("indefinite"),
            rewind: c_tn.and_then(|c| c.attr("fill")) == Some("remove"),
            full_screen: matches!(node.attr("fullScrn"), Some("1" | "true")),
            hide_when_not_playing: matches!(media.attr("showWhenStopped"), Some("0" | "false")),
            slides: media
                .attr("numSld")
                .and_then(|n| n.parse().ok())
                .unwrap_or(1)
                .max(1),
        });
    }
    // How each starts: a play command in the main sequence starts it with
    // a click (or by itself, as an after/with effect); without one it
    // plays when clicked on.
    walk(timing, &mut Vec::new(), &mut |element, ancestors| {
        if element.name != "cmd" || element.attr("type") != Some("call") {
            return;
        }
        let Some(command) = element.attr("cmd") else {
            return;
        };
        if !command.starts_with("playFrom") {
            return;
        }
        let Some(shape_id) = element.child("cBhvr").and_then(target_spid) else {
            return;
        };
        let in_main = ancestors.iter().any(|ancestor| {
            ancestor.name == "cTn" && ancestor.attr("nodeType") == Some("mainSeq")
                || ancestor.name == "seq"
                    && ancestor.child("cTn").and_then(|c| c.attr("nodeType")) == Some("mainSeq")
        });
        if !in_main {
            return;
        }
        let effect = ancestors
            .iter()
            .rev()
            .find(|ancestor| ancestor.name == "cTn" && ancestor.attr("presetClass").is_some());
        let start = match effect.and_then(|effect| effect.attr("nodeType")) {
            Some("afterEffect" | "withEffect") => MediaStart::Automatically,
            _ => MediaStart::InClickSequence,
        };
        if let Some(playback) = found
            .iter_mut()
            .find(|playback| playback.shape_id == shape_id)
        {
            playback.start = start;
        }
    });
    found
}

/// How long media plays, in ms, read from its container: ISO media
/// (`mp4`, `m4v`, `mov`, `m4a`) by its movie header, WAV by its data
/// size, MP3 by its frames' bit rate. `None` for what it cannot tell.
pub fn media_duration_ms(bytes: &[u8]) -> Option<f64> {
    iso_duration(bytes)
        .or_else(|| wav_duration(bytes))
        .or_else(|| mp3_duration(bytes))
        .filter(|ms| ms.is_finite() && *ms > 0.0)
}

fn be_u32(bytes: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_be_bytes(bytes.get(at..at + 4)?.try_into().ok()?))
}

fn be_u64(bytes: &[u8], at: usize) -> Option<u64> {
    Some(u64::from_be_bytes(bytes.get(at..at + 8)?.try_into().ok()?))
}

fn le_u32(bytes: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes(bytes.get(at..at + 4)?.try_into().ok()?))
}

/// The `moov/mvhd` duration of ISO base media.
fn iso_duration(bytes: &[u8]) -> Option<f64> {
    fn boxes(bytes: &[u8], mut at: usize, end: usize, want: &[u8; 4]) -> Option<(usize, usize)> {
        while at + 8 <= end {
            let size = be_u32(bytes, at)? as usize;
            let kind = bytes.get(at + 4..at + 8)?;
            let (header, size) = match size {
                1 => (16, be_u64(bytes, at + 8)? as usize),
                0 => (8, end - at),
                size => (8, size),
            };
            if size < header || at + size > end {
                return None;
            }
            if kind == want {
                return Some((at + header, at + size));
            }
            at += size;
        }
        None
    }
    if bytes.get(4..8) != Some(b"ftyp") {
        return None;
    }
    let (moov, moov_end) = boxes(bytes, 0, bytes.len(), b"moov")?;
    let (mvhd, _) = boxes(bytes, moov, moov_end, b"mvhd")?;
    let version = *bytes.get(mvhd)?;
    let (scale, duration) = if version == 1 {
        (be_u32(bytes, mvhd + 20)?, be_u64(bytes, mvhd + 24)?)
    } else {
        (
            be_u32(bytes, mvhd + 12)?,
            u64::from(be_u32(bytes, mvhd + 16)?),
        )
    };
    (scale > 0).then(|| duration as f64 * 1000.0 / f64::from(scale))
}

/// A WAV's `data` size over its byte rate.
fn wav_duration(bytes: &[u8]) -> Option<f64> {
    if bytes.get(0..4) != Some(b"RIFF") || bytes.get(8..12) != Some(b"WAVE") {
        return None;
    }
    let mut at = 12;
    let mut byte_rate = None;
    while at + 8 <= bytes.len() {
        let kind = bytes.get(at..at + 4)?;
        let size = le_u32(bytes, at + 4)? as usize;
        if kind == b"fmt " {
            byte_rate = le_u32(bytes, at + 16);
        } else if kind == b"data" {
            let rate = byte_rate.filter(|rate| *rate > 0)?;
            let size = size.min(bytes.len() - at - 8);
            return Some(size as f64 * 1000.0 / f64::from(rate));
        }
        at += 8 + size + (size & 1);
    }
    None
}

/// An MP3's length from its first frame's bit rate (exact for constant bit
/// rate, close for the rest).
fn mp3_duration(bytes: &[u8]) -> Option<f64> {
    let mut at = 0;
    // An ID3v2 tag in front.
    if bytes.get(0..3) == Some(b"ID3") {
        let size = bytes.get(6..10)?;
        let size = size
            .iter()
            .fold(0usize, |n, byte| (n << 7) | usize::from(byte & 0x7f));
        at = 10 + size;
    }
    while at + 4 <= bytes.len() {
        if bytes[at] == 0xff && bytes[at + 1] & 0xe0 == 0xe0 {
            let version = (bytes[at + 1] >> 3) & 0b11;
            let layer = (bytes[at + 1] >> 1) & 0b11;
            let index = usize::from(bytes[at + 2] >> 4);
            if layer == 0b01 && (1..15).contains(&index) {
                // MPEG-1 layer III, else MPEG-2/2.5 layer III, kbit/s.
                const V1: [u32; 15] = [
                    0, 32, 40, 48, 56, 64, 80, 96, 112, 128, 160, 192, 224, 256, 320,
                ];
                const V2: [u32; 15] =
                    [0, 8, 16, 24, 32, 40, 48, 56, 64, 80, 96, 112, 128, 144, 160];
                let kbps = if version == 0b11 {
                    V1[index]
                } else {
                    V2[index]
                };
                let audio = (bytes.len() - at) as f64;
                return Some(audio * 8.0 / f64::from(kbps));
            }
        }
        at += 1;
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    const SLIDE: &str = r#"<p:sld xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships" xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main"><p:cSld><p:spTree/></p:cSld><p:timing><p:tnLst><p:par><p:cTn id="1" dur="indefinite" restart="never" nodeType="tmRoot"><p:childTnLst><p:seq concurrent="1" nextAc="seek"><p:cTn id="2" dur="indefinite" nodeType="mainSeq"><p:childTnLst><p:par><p:cTn id="3" fill="hold"><p:stCondLst><p:cond delay="indefinite"/><p:cond evt="onBegin" delay="0"><p:tn val="2"/></p:cond></p:stCondLst><p:childTnLst><p:par><p:cTn id="4" fill="hold"><p:stCondLst><p:cond delay="0"/></p:stCondLst><p:childTnLst><p:par><p:cTn id="5" presetID="1" presetClass="mediacall" presetSubtype="0" fill="hold" nodeType="afterEffect"><p:stCondLst><p:cond delay="0"/></p:stCondLst><p:childTnLst><p:cmd type="call" cmd="playFrom(0.0)"><p:cBhvr><p:cTn id="6" dur="3239" fill="hold"/><p:tgtEl><p:spTgt spid="4"/></p:tgtEl></p:cBhvr></p:cmd></p:childTnLst></p:cTn></p:par></p:childTnLst></p:cTn></p:par></p:childTnLst></p:cTn></p:par></p:childTnLst></p:cTn></p:seq><p:audio><p:cMediaNode vol="25000" numSld="999" showWhenStopped="0"><p:cTn id="7" repeatCount="indefinite" fill="remove" display="0"><p:stCondLst><p:cond delay="indefinite"/></p:stCondLst></p:cTn><p:tgtEl><p:spTgt spid="4"/></p:tgtEl></p:cMediaNode></p:audio><p:video fullScrn="1"><p:cMediaNode vol="80000" mute="1"><p:cTn id="8" fill="hold" display="0"><p:stCondLst><p:cond delay="indefinite"/></p:stCondLst></p:cTn><p:tgtEl><p:spTgt spid="5"/></p:tgtEl></p:cMediaNode></p:video></p:childTnLst></p:cTn></p:par></p:tnLst></p:timing></p:sld>"#;

    #[test]
    fn playback_options_are_read_from_the_timing() {
        let playback = parse_media_playback(SLIDE);
        assert_eq!(playback.len(), 2);
        let audio = &playback[0];
        assert_eq!(audio.shape_id, 4);
        assert_eq!(audio.kind, MediaKind::Audio);
        assert_eq!(audio.start, MediaStart::Automatically);
        assert_eq!(audio.volume, 25_000);
        assert!(audio.loop_until_stopped && audio.rewind && audio.hide_when_not_playing);
        assert!(audio.plays_across_slides());
        let video = &playback[1];
        assert_eq!(video.start, MediaStart::WhenClicked);
        assert!(video.mute && video.full_screen && !video.loop_until_stopped);
        assert_eq!(video.slides, 1);
    }

    #[test]
    fn durations_are_read_from_the_containers() {
        // An mp4 whose movie header says 3.5 s at a 1000 timescale.
        let mut mvhd = vec![0u8; 100];
        mvhd[12..16].copy_from_slice(&1000u32.to_be_bytes());
        mvhd[16..20].copy_from_slice(&3500u32.to_be_bytes());
        let mut mp4 = Vec::new();
        mp4.extend_from_slice(&16u32.to_be_bytes());
        mp4.extend_from_slice(b"ftypisom\0\0\0\0");
        mp4.extend_from_slice(&(8 + 8 + mvhd.len() as u32).to_be_bytes());
        mp4.extend_from_slice(b"moov");
        mp4.extend_from_slice(&(8 + mvhd.len() as u32).to_be_bytes());
        mp4.extend_from_slice(b"mvhd");
        mp4.extend_from_slice(&mvhd);
        assert_eq!(media_duration_ms(&mp4), Some(3500.0));
        // A WAV of 2 s at 8000 bytes a second.
        let mut wav = Vec::new();
        wav.extend_from_slice(b"RIFF\0\0\0\0WAVEfmt ");
        wav.extend_from_slice(&16u32.to_le_bytes());
        let mut fmt = [0u8; 16];
        fmt[8..12].copy_from_slice(&8000u32.to_le_bytes());
        wav.extend_from_slice(&fmt);
        wav.extend_from_slice(b"data");
        wav.extend_from_slice(&16000u32.to_le_bytes());
        wav.extend(std::iter::repeat_n(0u8, 16000));
        assert_eq!(media_duration_ms(&wav), Some(2000.0));
        assert_eq!(media_duration_ms(b"not media"), None);
    }

    #[test]
    fn media_trim_fades_and_bookmarks_are_read() {
        let xml = r#"<p:nvPr xmlns:p="p" xmlns:a="a" xmlns:r="r" xmlns:p14="p14"><a:videoFile r:link="rId2"/><p:extLst><p:ext uri="{DAA4B4D4-6D71-4841-9C94-3DE7FCFB9230}"><p14:media r:embed="rId1"><p14:trim st="1500" end="250.5"/><p14:fade in="500" out="750"/><p14:bmkLst><p14:bmk name="Intro" time="1000"/><p14:bmk name="End" time="2750.25"/></p14:bmkLst></p14:media></p:ext></p:extLst></p:nvPr>"#;
        let media = parse_media(&parse_tree(xml).unwrap()).unwrap();
        assert_eq!(media.kind, MediaKind::Video);
        assert_eq!(media.file_rel_id.as_deref(), Some("rId2"));
        assert_eq!(media.embed_rel_id.as_deref(), Some("rId1"));
        assert!(!media.is_linked());
        assert_eq!(media.trim_start_ms, Some(1500.0));
        assert_eq!(media.trim_end_ms, Some(250.5));
        assert_eq!(media.fade_in_ms, Some(500.0));
        assert_eq!(media.fade_out_ms, Some(750.0));
        assert_eq!(media.bookmarks.len(), 2);
        assert_eq!(media.bookmarks[1].name, "End");
        assert_eq!(media.bookmarks[1].time_ms, 2750.25);
    }
}
