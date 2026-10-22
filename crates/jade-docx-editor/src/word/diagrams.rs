//! 挿入 → SmartArt and グラフ over any [`WordSession`], and what their
//! contextual tools do: a SmartArt graphic from the gallery, its text and
//! structure edited from the text pane's outline, its layout changed; a
//! chart with its embedded workbook, its data edited in a sheet, its type,
//! title and legend.

use crate::commands::edit_drawing::replace_drawing_command;
use crate::commands::insert_chart::InsertChartCommand;
use crate::commands::insert_shape::InsertShapeCommand;
use crate::commands::update_chart::UpdateChartCommand;
use crate::commands::Command;
pub use crate::document::ChartKind;
use crate::document::{ChartSeries, RunChart, TextBoxShape};
/// SmartArt's layouts the gallery offers: name, Japanese, English, family.
pub use crate::smartart::LAYOUTS as SMARTART_LAYOUTS;
use crate::smartart::{self, Diagram, DiagramNode};
use crate::word_document::objects::parse_element;
use crate::word_document::{
    ChartInsertSpec, FloatingObjectKind, FloatingObjectTarget, ShapeInsertSpec,
};

use super::objects::{object_run, WordObjects};
use super::shapes::{InsertThen, Inserted};
use super::WordSession;

/// Word's default SmartArt frame: 15.24 cm × 8.89 cm.
pub const SMARTART_EXTENT_EMU: (i64, i64) = (5_486_400, 3_200_400);

const DIAGRAM_DATA: &str =
    "http://schemas.openxmlformats.org/officeDocument/2006/relationships/diagramData";
const DIAGRAM_LAYOUT: &str =
    "http://schemas.openxmlformats.org/officeDocument/2006/relationships/diagramLayout";
const DIAGRAM_STYLE: &str =
    "http://schemas.openxmlformats.org/officeDocument/2006/relationships/diagramQuickStyle";
const DIAGRAM_COLORS: &str =
    "http://schemas.openxmlformats.org/officeDocument/2006/relationships/diagramColors";
const DIAGRAM_DRAWING: &str =
    "http://schemas.microsoft.com/office/2007/relationships/diagramDrawing";

/// The points a new graphic of `layout` starts with, as Word puts them in:
/// empty, three of them (a root with three under it for a hierarchy).
pub fn starter_nodes(layout: &str) -> Vec<DiagramNode> {
    match smartart::family(layout) {
        smartart::Family::Hierarchy => vec![DiagramNode::with(
            "",
            vec![
                DiagramNode::with("", vec![DiagramNode::new("")]),
                DiagramNode::new(""),
            ],
        )],
        smartart::Family::BlockList => (0..5).map(|_| DiagramNode::new("")).collect(),
        _ => (0..3).map(|_| DiagramNode::new("")).collect(),
    }
}

/// Word's sample data for a new chart: four categories, three series.
pub fn sample_chart(kind: ChartKind, japanese: bool) -> ChartInsertSpec {
    let (category, series) = if japanese {
        ("分類", "系列")
    } else {
        ("Category", "Series")
    };
    let values = [
        [4.3, 2.4, 2.0],
        [2.5, 4.4, 2.0],
        [3.5, 1.8, 3.0],
        [4.5, 2.8, 5.0],
    ];
    let series_count = if kind == ChartKind::Pie { 1 } else { 3 };
    ChartInsertSpec {
        kind,
        title: if japanese {
            "グラフ タイトル"
        } else {
            "Chart Title"
        }
        .to_string(),
        categories: (1..=4).map(|index| format!("{category} {index}")).collect(),
        series: (0..series_count)
            .map(|column| ChartSeries {
                name: format!("{series} {}", column + 1),
                values: values.iter().map(|row| Some(row[column])).collect(),
            })
            .collect(),
        width_px: 576.0,
        height_px: 336.0,
    }
}

/// The diagram data relationship of the SmartArt graphic at `target`.
fn diagram_relationship(
    session: &(impl WordSession + ?Sized),
    target: FloatingObjectTarget,
) -> Option<String> {
    object_run(session.word_document(), target)?
        .image
        .as_ref()?
        .diagram_data_relationship_id
        .clone()
}

/// A placeholder object at the caret, which a drawing then replaces.
fn placeholder(paragraph: usize, offset: usize) -> InsertShapeCommand {
    InsertShapeCommand::new(
        paragraph,
        offset,
        ShapeInsertSpec {
            shape: TextBoxShape::Rectangle,
            width_px: 12.0,
            height_px: 12.0,
            fill_color: None,
            border_color: None,
            border_width_px: 0.0,
            text: String::new(),
            text_color: None,
            text_font_size_pt: None,
            text_bold: None,
            text_italic: None,
            text_alignment: None,
        },
    )
}

/// SmartArt and charts, for every session.
pub trait WordDiagrams: WordSession + WordObjects {
    /// 挿入 → SmartArt: a graphic of `layout` (a `layoutDef` id from
    /// [`smartart::LAYOUTS`]) in line at the caret, with `nodes` (or the
    /// layout's starter points).
    fn insert_smartart(
        &mut self,
        layout: &str,
        nodes: Option<Vec<DiagramNode>>,
    ) -> Result<(), String> {
        let range = self.selection_range().ok_or("no caret")?;
        let (start, _) = range.normalized();
        let diagram = Diagram {
            layout: layout.to_string(),
            nodes: nodes.unwrap_or_else(|| starter_nodes(layout)),
        };
        self.execute_command(
            Box::new(InsertThen::new(
                Inserted::Shape(placeholder(start.paragraph, start.offset)),
                move |target| {
                    let diagram = diagram.clone();
                    vec![Box::new(replace_drawing_command("insert_smartart", target, move |document, drawing| {
                        let (cx, cy) = SMARTART_EXTENT_EMU;
                        let drawing_part = smartart::drawing_xml(&diagram, cx as f64, cy as f64);
                        let drawing_rel = document.add_object_part(
                            "word/diagrams/drawing",
                            "xml",
                            drawing_part.as_bytes(),
                            "application/vnd.ms-office.drawingml.diagramDrawing+xml",
                            DIAGRAM_DRAWING,
                        )?;
                        let data = document.add_object_part(
                            "word/diagrams/data",
                            "xml",
                            smartart::data_xml(&diagram, Some(&drawing_rel)).as_bytes(),
                            "application/vnd.openxmlformats-officedocument.drawingml.diagramData+xml",
                            DIAGRAM_DATA,
                        )?;
                        let layout = document.add_object_part(
                            "word/diagrams/layout",
                            "xml",
                            smartart::layout_xml(&diagram.layout).as_bytes(),
                            "application/vnd.openxmlformats-officedocument.drawingml.diagramLayout+xml",
                            DIAGRAM_LAYOUT,
                        )?;
                        let style = document.add_object_part(
                            "word/diagrams/quickStyle",
                            "xml",
                            smartart::quick_style_xml().as_bytes(),
                            "application/vnd.openxmlformats-officedocument.drawingml.diagramStyle+xml",
                            DIAGRAM_STYLE,
                        )?;
                        let colors = document.add_object_part(
                            "word/diagrams/colors",
                            "xml",
                            smartart::colors_xml().as_bytes(),
                            "application/vnd.openxmlformats-officedocument.drawingml.diagramColors+xml",
                            DIAGRAM_COLORS,
                        )?;
                        let id = document.new_drawing_object_id();
                        *drawing = parse_element(&format!(
                            r#"<w:drawing><wp:inline distT="0" distB="0" distL="0" distR="0"><wp:extent cx="{cx}" cy="{cy}"/><wp:effectExtent l="0" t="0" r="0" b="0"/><wp:docPr id="{id}" name="図表 {id}"/><wp:cNvGraphicFramePr/><a:graphic><a:graphicData uri="http://schemas.openxmlformats.org/drawingml/2006/diagram"><dgm:relIds xmlns:dgm="http://schemas.openxmlformats.org/drawingml/2006/diagram" r:dm="{data}" r:lo="{layout}" r:qs="{style}" r:cs="{colors}"/></a:graphicData></a:graphic></wp:inline></w:drawing>"#
                        ))?;
                        Ok(())
                    })) as Box<dyn Command>]
                },
            )),
            false,
        )
    }

    /// The SmartArt graphic's layout and text, for the text pane.
    fn smartart(&self, target: FloatingObjectTarget) -> Option<Diagram> {
        let relationship = diagram_relationship(self, target)?;
        let bytes = self.word_document().object_media(&relationship)?;
        // The data names its layout (`loTypeId`), as Word writes it.
        smartart::parse_data(std::str::from_utf8(&bytes).ok()?, None)
    }

    /// The text pane's edit: the graphic's points become `diagram`'s, and
    /// its drawing is laid out again (with `diagram.layout`, which may be a
    /// new one: レイアウトの変更).
    fn set_smartart(
        &mut self,
        target: FloatingObjectTarget,
        diagram: Diagram,
    ) -> Result<(), String> {
        let relationship =
            diagram_relationship(self, target).ok_or("the object is not a SmartArt graphic")?;
        let (width, height) = object_run(self.word_document(), target)
            .and_then(|run| run.image.as_ref())
            .map(|image| {
                (
                    image.width_px.unwrap_or(576.0) * 9525.0,
                    image.height_px.unwrap_or(336.0) * 9525.0,
                )
            })
            .unwrap_or((SMARTART_EXTENT_EMU.0 as f64, SMARTART_EXTENT_EMU.1 as f64));
        self.edit_object(target, "smartart_text", move |document, drawing| {
            let drawing_rel = document
                .related_part_relationship(&relationship, "diagramDrawing")
                .or_else(|| {
                    // A graphic with no cached drawing gets one.
                    document
                        .add_object_part(
                            "word/diagrams/drawing",
                            "xml",
                            b"<dsp:drawing/>",
                            "application/vnd.ms-office.drawingml.diagramDrawing+xml",
                            DIAGRAM_DRAWING,
                        )
                        .ok()
                })
                .ok_or("the graphic's drawing cannot be written")?;
            document.replace_object_part(
                &relationship,
                smartart::data_xml(&diagram, Some(&drawing_rel)).as_bytes(),
            )?;
            document.replace_object_part(
                &drawing_rel,
                smartart::drawing_xml(&diagram, width, height).as_bytes(),
            )?;
            // A new layout goes into the layout part too.
            if let Some(layout_rel) = crate::drawing_xml::descendant(drawing, "dgm:relIds")
                .and_then(|ids| crate::drawing_xml::get_attr(ids, "r:lo"))
                .map(str::to_string)
            {
                document.replace_object_part(
                    &layout_rel,
                    smartart::layout_xml(&diagram.layout).as_bytes(),
                )?;
            }
            Ok(())
        })
    }

    /// 挿入 → グラフ: Word's sample chart of `kind` in line at the caret, with
    /// its embedded workbook.
    fn insert_chart(&mut self, kind: ChartKind, japanese: bool) -> Result<(), String> {
        let range = self.selection_range().ok_or("no caret")?;
        self.execute_command(
            Box::new(InsertChartCommand::new(range, sample_chart(kind, japanese))),
            false,
        )
    }

    /// The chart's data, as データの編集 shows it.
    fn chart_data(&self, target: FloatingObjectTarget) -> Option<RunChart> {
        object_run(self.word_document(), target)?
            .image
            .as_ref()?
            .chart
            .clone()
    }

    /// データの編集 / グラフの種類の変更 / グラフ タイトル: the chart becomes
    /// `chart`, its embedded workbook too.
    fn set_chart(&mut self, target: FloatingObjectTarget, chart: RunChart) -> Result<(), String> {
        if target.kind != FloatingObjectKind::Image {
            return Err("the object is not a chart".into());
        }
        self.execute_command(Box::new(UpdateChartCommand::new(target, chart)), true)
    }

    /// 凡例: where the chart's legend is (`r`, `t`, `l`, `b`), or none.
    fn set_chart_legend(
        &mut self,
        target: FloatingObjectTarget,
        position: Option<&'static str>,
    ) -> Result<(), String> {
        let relationship = object_run(self.word_document(), target)
            .and_then(|run| run.image.as_ref())
            .filter(|image| image.chart.is_some())
            .and_then(|image| image.relationship_id.clone())
            .ok_or("the object is not a chart")?;
        self.edit_object(target, "chart_legend", move |document, _| {
            let xml = document
                .object_media(&relationship)
                .and_then(|bytes| String::from_utf8(bytes).ok())
                .ok_or("the chart part is missing")?;
            let xml = set_legend(&xml, position)?;
            document.replace_object_part(&relationship, xml.as_bytes())
        })
    }
}

impl<T: WordSession + ?Sized> WordDiagrams for T {}

/// The chart XML with its legend at `position`, or without one.
pub fn set_legend(xml: &str, position: Option<&str>) -> Result<String, String> {
    let without = match (xml.find("<c:legend>"), xml.find("</c:legend>")) {
        (Some(start), Some(end)) => {
            format!("{}{}", &xml[..start], &xml[end + "</c:legend>".len()..])
        }
        _ => xml.to_string(),
    };
    let Some(position) = position else {
        return Ok(without);
    };
    let legend =
        format!(r#"<c:legend><c:legendPos val="{position}"/><c:overlay val="0"/></c:legend>"#);
    // The legend follows the plot area.
    let at = without
        .find("</c:plotArea>")
        .map(|at| at + "</c:plotArea>".len())
        .ok_or("the chart has no plot area")?;
    Ok(format!("{}{legend}{}", &without[..at], &without[at..]))
}
