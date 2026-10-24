//! Browser entry points for 図の形式 (修整, 色, アート効果, 透明度, 背景の削除,
//! 図の枠線, 図の効果) and SmartArt — the [`crate::word`] models run on the
//! view as a session, on the selected object.

use super::word_models::{from_js, to_js};
use super::*;
use crate::smartart::{Diagram, DiagramNode};
use crate::word::diagrams::{WordDiagrams, SMARTART_LAYOUTS};
use crate::word::picture_format::{
    Effect, OutlineEdit, Recolor, ReflectionPreset, ShadowPreset, WordPictureFormat,
    ARTISTIC_EFFECTS,
};
use serde_json::json;

/// One 図の形式 command, as the ribbon sends it.
#[derive(serde::Deserialize)]
#[serde(tag = "type", rename_all = "camelCase", deny_unknown_fields)]
enum PictureFormatJs {
    /// 修整: brightness and contrast, -1 … 1.
    Corrections { brightness: f64, contrast: f64 },
    /// 色の彩度, 0 … 4 (1 unchanged).
    Saturation { value: f64 },
    /// 色のトーン in kelvin (6500 unchanged).
    Temperature { kelvin: f64 },
    /// 色の変更: `none`, `grayscale`, `sepia`, `blackAndWhite` (with a
    /// threshold) or `accent` (a theme slot, dark or light).
    #[serde(rename_all = "camelCase")]
    Recolor {
        mode: String,
        #[serde(default)]
        threshold: Option<f64>,
        #[serde(default)]
        slot: Option<String>,
        #[serde(default)]
        light: bool,
    },
    /// 透明度, 0 … 1.
    Transparency { value: f64 },
    /// 透明色を指定: `RRGGBB`.
    TransparentColor { color: String },
    /// アート効果 by name, or none.
    ArtisticEffect { effect: Option<String> },
    /// 背景の削除 (`false`: 変更をすべて破棄).
    RemoveBackground { remove: bool },
    /// 図の枠線: none, a colour, or a width in points.
    Outline {
        #[serde(default)]
        color: Option<String>,
        #[serde(default)]
        width: Option<f64>,
        #[serde(default)]
        none: bool,
    },
    /// 影: an outer (0–8) or inner (0–2) preset, or none.
    Shadow {
        #[serde(default)]
        outer: Option<usize>,
        #[serde(default)]
        inner: Option<usize>,
    },
    /// 反射: `tight`, `half`, `full`, or none.
    Reflection { preset: Option<String> },
    /// 光彩: radius in points and `RRGGBB`, or none.
    Glow {
        #[serde(default)]
        radius: Option<f64>,
        #[serde(default)]
        color: Option<String>,
    },
    /// ぼかし: radius in points, or none.
    SoftEdge { radius: Option<f64> },
    /// 面取り: an `a:bevelT` preset, or none.
    Bevel { preset: Option<String> },
    /// 図のリセット (`size`: 図とサイズのリセット).
    Reset { size: bool },
}

/// A SmartArt point and the points under it.
#[derive(serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct DiagramNodeJs {
    text: String,
    #[serde(default)]
    children: Vec<DiagramNodeJs>,
}

impl From<DiagramNodeJs> for DiagramNode {
    fn from(node: DiagramNodeJs) -> Self {
        DiagramNode {
            text: node.text,
            children: node.children.into_iter().map(DiagramNode::from).collect(),
        }
    }
}

impl From<&DiagramNode> for DiagramNodeJs {
    fn from(node: &DiagramNode) -> Self {
        DiagramNodeJs {
            text: node.text.clone(),
            children: node.children.iter().map(DiagramNodeJs::from).collect(),
        }
    }
}

#[derive(serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct DiagramJs {
    layout: String,
    nodes: Vec<DiagramNodeJs>,
}

fn known_layout(layout: &str) -> Result<(), JsValue> {
    SMARTART_LAYOUTS
        .iter()
        .any(|(id, ..)| *id == layout)
        .then_some(())
        .ok_or_else(|| JsValue::from_str(&format!("unknown SmartArt layout {layout}")))
}

#[wasm_bindgen]
impl OffscreenDocumentView {
    /// The galleries 図の形式 offers: the アート効果 and SmartArt layouts.
    #[wasm_bindgen(js_name = pictureFormatGallery)]
    pub fn picture_format_gallery(&self) -> JsValue {
        to_js(&json!({
            "artisticEffects": ARTISTIC_EFFECTS,
            "outerShadows": ShadowPreset::OUTER.len(),
            "innerShadows": ShadowPreset::INNER.len(),
            "smartArtLayouts": SMARTART_LAYOUTS
                .iter()
                .map(|(id, name, english, _)| json!({ "id": id, "name": name, "english": english }))
                .collect::<Vec<_>>(),
        }))
    }

    /// One 図の形式 command on the selected picture or shape, as one undo
    /// step.
    #[wasm_bindgen(js_name = applyPictureFormat)]
    pub fn apply_picture_format(&self, action: JsValue) -> Result<JsValue, JsValue> {
        let action: PictureFormatJs = from_js(action, "picture format")?;
        let target = self
            .state
            .borrow()
            .object_selection
            .ok_or_else(|| JsValue::from_str("図が選択されていません"))?;
        let artistic = match &action {
            PictureFormatJs::ArtisticEffect {
                effect: Some(effect),
            } => Some(
                ARTISTIC_EFFECTS
                    .into_iter()
                    .find(|known| known == effect)
                    .ok_or_else(|| {
                        JsValue::from_str(&format!("unknown artistic effect {effect}"))
                    })?,
            ),
            _ => None,
        };
        self.run_word_session("word_picture_format", move |session| match action {
            PictureFormatJs::Corrections {
                brightness,
                contrast,
            } => session.set_picture_corrections(target, brightness, contrast),
            PictureFormatJs::Saturation { value } => session.set_picture_saturation(target, value),
            PictureFormatJs::Temperature { kelvin } => {
                session.set_picture_temperature(target, kelvin)
            }
            PictureFormatJs::Recolor {
                mode,
                threshold,
                slot,
                light,
            } => {
                let recolor = match mode.as_str() {
                    "none" => Recolor::None,
                    "grayscale" => Recolor::Grayscale,
                    "sepia" => Recolor::Sepia,
                    "blackAndWhite" => Recolor::BlackAndWhite(threshold.unwrap_or(0.5)),
                    "accent" => Recolor::Accent {
                        slot: slot.unwrap_or_else(|| "accent1".to_string()),
                        light,
                    },
                    other => return Err(format!("unknown recolor {other}")),
                };
                session.recolor_picture(target, recolor)
            }
            PictureFormatJs::Transparency { value } => {
                session.set_picture_transparency(target, value)
            }
            PictureFormatJs::TransparentColor { color } => {
                session.set_transparent_color(target, &color)
            }
            PictureFormatJs::ArtisticEffect { .. } => session.set_artistic_effect(target, artistic),
            PictureFormatJs::RemoveBackground { remove } => {
                session.remove_picture_background(target, remove)
            }
            PictureFormatJs::Outline { color, width, none } => {
                let edit = match (none, color, width) {
                    (true, ..) => OutlineEdit::None,
                    (false, Some(color), _) => OutlineEdit::Color(color),
                    (false, None, Some(width)) => OutlineEdit::Width(width),
                    (false, None, None) => {
                        return Err("図の枠線: 色か太さを指定してください".into())
                    }
                };
                session.set_picture_outline(target, edit)
            }
            PictureFormatJs::Shadow { outer, inner } => {
                let preset = match (outer, inner) {
                    (Some(index), _) => Some(
                        *ShadowPreset::OUTER
                            .get(index)
                            .ok_or_else(|| format!("no outer shadow {index}"))?,
                    ),
                    (None, Some(index)) => Some(
                        *ShadowPreset::INNER
                            .get(index)
                            .ok_or_else(|| format!("no inner shadow {index}"))?,
                    ),
                    (None, None) => None,
                };
                session.set_object_effect(target, Effect::Shadow(preset))
            }
            PictureFormatJs::Reflection { preset } => {
                let preset = match preset.as_deref() {
                    None => None,
                    Some("tight") => Some(ReflectionPreset::Tight),
                    Some("half") => Some(ReflectionPreset::Half),
                    Some("full") => Some(ReflectionPreset::Full),
                    Some(other) => return Err(format!("unknown reflection {other}")),
                };
                session.set_object_effect(target, Effect::Reflection(preset))
            }
            PictureFormatJs::Glow { radius, color } => session.set_object_effect(
                target,
                Effect::Glow(
                    radius.map(|radius| (radius, color.unwrap_or_else(|| "4472C4".into()))),
                ),
            ),
            PictureFormatJs::SoftEdge { radius } => {
                session.set_object_effect(target, Effect::SoftEdge(radius))
            }
            PictureFormatJs::Bevel { preset } => {
                session.set_object_effect(target, Effect::Bevel(preset))
            }
            PictureFormatJs::Reset { size } => session.reset_picture(target, size),
        })
    }

    /// SmartArt グラフィックの挿入 at the caret: `layout` is one of
    /// `pictureFormatGallery().smartArtLayouts`, `nodes` its points (the
    /// layout's sample text when left out).
    #[wasm_bindgen(js_name = insertSmartArt)]
    pub fn insert_smart_art(&self, layout: String, nodes: JsValue) -> Result<JsValue, JsValue> {
        known_layout(&layout)?;
        let nodes: Option<Vec<DiagramNodeJs>> = if nodes.is_undefined() || nodes.is_null() {
            None
        } else {
            Some(from_js(nodes, "SmartArt points")?)
        };
        let paragraph = self.current_selection().focus.paragraph;
        let diagrams = |view: &Self| -> Vec<(usize, Option<String>)> {
            let state = view.state.borrow();
            state
                .word_doc
                .as_ref()
                .and_then(|document| document.paragraph(paragraph))
                .map(|paragraph| {
                    paragraph
                        .runs
                        .iter()
                        .enumerate()
                        .filter_map(|(index, run)| {
                            let image = run.image.as_ref()?;
                            image.diagram_data_relationship_id.as_ref()?;
                            Some((index, image.diagram_data_relationship_id.clone()))
                        })
                        .collect()
                })
                .unwrap_or_default()
        };
        let before = diagrams(self);
        let payload = self.run_word_session("word_insert_smartart", move |session| {
            session.insert_smartart(
                &layout,
                nodes.map(|nodes| nodes.into_iter().map(DiagramNode::from).collect()),
            )
        })?;
        // The new graphic is selected, as Word selects what it inserts.
        let inserted = diagrams(self)
            .into_iter()
            .find(|(_, id)| !before.iter().any(|(_, known)| known == id));
        if let Some((run_index, _)) = inserted {
            self.state.borrow_mut().object_selection = Some(FloatingObjectTarget {
                paragraph_index: paragraph,
                run_index,
                kind: FloatingObjectKind::Image,
            });
        }
        Ok(payload)
    }

    /// The selected SmartArt graphic's layout and points: the text pane.
    #[wasm_bindgen(js_name = smartArtAtSelection)]
    pub fn smart_art_at_selection(&self) -> Result<JsValue, JsValue> {
        let Some(target) = self.state.borrow().object_selection else {
            return Ok(JsValue::NULL);
        };
        let diagram = self.read_word_session(|session| session.smartart(target))?;
        Ok(diagram.map_or(JsValue::NULL, |diagram| {
            to_js(&DiagramJs {
                layout: diagram.layout,
                nodes: diagram.nodes.iter().map(DiagramNodeJs::from).collect(),
            })
        }))
    }

    /// The text pane's edit, or レイアウトの変更, on the selected SmartArt
    /// graphic.
    #[wasm_bindgen(js_name = setSmartArt)]
    pub fn set_smart_art(&self, diagram: JsValue) -> Result<JsValue, JsValue> {
        let diagram: DiagramJs = from_js(diagram, "SmartArt")?;
        known_layout(&diagram.layout)?;
        let target = self
            .state
            .borrow()
            .object_selection
            .ok_or_else(|| JsValue::from_str("SmartArt が選択されていません"))?;
        self.run_word_session("word_set_smartart", move |session| {
            session.set_smartart(
                target,
                Diagram {
                    layout: diagram.layout,
                    nodes: diagram.nodes.into_iter().map(DiagramNode::from).collect(),
                },
            )
        })
    }

    /// 画像 → このデバイス for any picture Word takes — PNG, JPEG, GIF, or an
    /// SVG, which goes in as Word puts one in: a PNG fallback as the picture
    /// and the SVG beside it. In line at the caret, at its own size unless
    /// wider than the text; one undo step.
    #[wasm_bindgen(js_name = insertPicture)]
    pub fn insert_picture_js(&self, bytes: Vec<u8>) -> Result<JsValue, JsValue> {
        use crate::word::picture::WordPicture;
        self.run_word_session("word_insert_picture", move |session| {
            session.insert_picture(bytes)
        })
    }
}
