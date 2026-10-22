//! 挿入 → 図形's gallery: Word's categories and the names its tooltips give
//! each preset, in Japanese and English, in the gallery's order.

use crate::document::TextBoxShape;

use TextBoxShape as S;

/// One category of the gallery.
pub struct Category {
    pub ja: &'static str,
    pub en: &'static str,
    pub shapes: &'static [(TextBoxShape, &'static str, &'static str)],
}

pub const CATEGORIES: [Category; 8] = [
    Category {
        ja: "線",
        en: "Lines",
        shapes: &[
            (S::Line, "直線", "Line"),
            (S::LineArrow, "直線矢印コネクタ", "Line Arrow"),
            (
                S::LineArrowDouble,
                "双方向矢印コネクタ",
                "Line Arrow: Double",
            ),
            (S::LineBent, "コネクタ: カギ線", "Connector: Elbow"),
            (S::LineCurved, "コネクタ: 曲線", "Connector: Curved"),
        ],
    },
    Category {
        ja: "四角形",
        en: "Rectangles",
        shapes: &[
            (S::Rectangle, "正方形/長方形", "Rectangle"),
            (
                S::RoundedRectangle,
                "四角形: 角を丸くする",
                "Rectangle: Rounded Corners",
            ),
            (
                S::SnipOneRectangle,
                "四角形: 1 つの角を切り取る",
                "Rectangle: Single Corner Snipped",
            ),
            (
                S::SnipTwoSameRectangle,
                "四角形: 上の 2 つの角を切り取る",
                "Rectangle: Top Corners Snipped",
            ),
            (
                S::SnipTwoDiagonalRectangle,
                "四角形: 対角を切り取る",
                "Rectangle: Diagonal Corners Snipped",
            ),
            (
                S::SnipRoundRectangle,
                "四角形: 1 つの角を切り取り 1 つの角を丸める",
                "Rectangle: Top Corners One Rounded and One Snipped",
            ),
            (
                S::RoundOneRectangle,
                "四角形: 1 つの角を丸める",
                "Rectangle: Single Corner Rounded",
            ),
            (
                S::RoundTwoSameRectangle,
                "四角形: 上の 2 つの角を丸める",
                "Rectangle: Top Corners Rounded",
            ),
            (
                S::RoundTwoDiagonalRectangle,
                "四角形: 対角を丸める",
                "Rectangle: Diagonal Corners Rounded",
            ),
        ],
    },
    Category {
        ja: "基本図形",
        en: "Basic Shapes",
        shapes: &[
            (S::Ellipse, "楕円", "Oval"),
            (S::Triangle, "二等辺三角形", "Isosceles Triangle"),
            (S::RightTriangle, "直角三角形", "Right Triangle"),
            (S::Parallelogram, "平行四辺形", "Parallelogram"),
            (S::Trapezoid, "台形", "Trapezoid"),
            (S::Diamond, "ひし形", "Diamond"),
            (S::Pentagon, "五角形", "Regular Pentagon"),
            (S::Hexagon, "六角形", "Hexagon"),
            (S::Octagon, "八角形", "Octagon"),
            (S::Pie, "部分円", "Partial Circle"),
            (S::Chord, "弦", "Chord"),
            (S::Teardrop, "涙形", "Teardrop"),
            (S::Frame, "フレーム", "Frame"),
            (S::HalfFrame, "フレーム (半分)", "Half Frame"),
            (S::Corner, "L 字", "L-Shape"),
            (S::DiagonalStripe, "斜め縞", "Diagonal Stripe"),
            (S::Plus, "十字形", "Cross"),
            (S::Plaque, "ブローチ", "Plaque"),
            (S::Can, "円柱", "Cylinder"),
            (S::Cube, "直方体", "Cube"),
            (S::Bevel, "四角形: 面取り", "Rectangle: Beveled"),
            (S::Donut, "円: 塗りつぶしなし", "Circle: Hollow"),
            (S::NoSmoking, "禁止マーク", "\"Not Allowed\" Symbol"),
            (S::BlockArc, "アーチ", "Block Arc"),
            (S::FoldedCorner, "四角形: メモ", "Rectangle: Folded Corner"),
            (S::SmileyFace, "スマイル", "Smiley Face"),
            (S::Heart, "ハート", "Heart"),
            (S::LightningBolt, "稲妻", "Lightning Bolt"),
            (S::Sun, "太陽", "Sun"),
            (S::Moon, "月", "Moon"),
            (S::Cloud, "雲", "Cloud"),
            (S::Arc, "円弧", "Arc"),
            (S::LeftBracket, "左大かっこ", "Left Bracket"),
            (S::RightBracket, "右大かっこ", "Right Bracket"),
            (S::LeftBrace, "左中かっこ", "Left Brace"),
            (S::RightBrace, "右中かっこ", "Right Brace"),
        ],
    },
    Category {
        ja: "ブロック矢印",
        en: "Block Arrows",
        shapes: &[
            (S::RightArrow, "矢印: 右", "Arrow: Right"),
            (S::LeftArrow, "矢印: 左", "Arrow: Left"),
            (S::UpArrow, "矢印: 上", "Arrow: Up"),
            (S::DownArrow, "矢印: 下", "Arrow: Down"),
            (S::LeftRightArrow, "矢印: 左右", "Arrow: Left-Right"),
            (S::UpDownArrow, "矢印: 上下", "Arrow: Up-Down"),
            (S::QuadArrow, "矢印: 四方向", "Arrow: Quad"),
            (S::BentArrow, "矢印: 折線", "Arrow: Bent"),
            (S::UturnArrow, "矢印: U ターン", "Arrow: U-Turn"),
            (S::CurvedRightArrow, "矢印: 右カーブ", "Arrow: Curved Right"),
            (
                S::StripedRightArrow,
                "矢印: ストライプ",
                "Arrow: Striped Right",
            ),
            (S::NotchedRightArrow, "矢印: V 字型", "Arrow: Notched Right"),
            (S::HomePlate, "矢印: 五方向", "Arrow: Pentagon"),
            (S::Chevron, "矢印: 山形", "Arrow: Chevron"),
        ],
    },
    Category {
        ja: "数式図形",
        en: "Equation Shapes",
        shapes: &[(S::MathPlus, "加算記号", "Plus Sign")],
    },
    Category {
        ja: "フローチャート",
        en: "Flowchart",
        shapes: &[
            (
                S::FlowChartProcess,
                "フローチャート: 処理",
                "Flowchart: Process",
            ),
            (
                S::FlowChartAlternateProcess,
                "フローチャート: 代替処理",
                "Flowchart: Alternate Process",
            ),
            (
                S::FlowChartDecision,
                "フローチャート: 判断",
                "Flowchart: Decision",
            ),
            (
                S::FlowChartPredefinedProcess,
                "フローチャート: 定義済み処理",
                "Flowchart: Predefined Process",
            ),
            (
                S::FlowChartInternalStorage,
                "フローチャート: 内部記憶",
                "Flowchart: Internal Storage",
            ),
            (
                S::FlowChartDocument,
                "フローチャート: 書類",
                "Flowchart: Document",
            ),
            (
                S::FlowChartMultidocument,
                "フローチャート: 複数書類",
                "Flowchart: Multidocument",
            ),
            (
                S::FlowChartTerminator,
                "フローチャート: 端子",
                "Flowchart: Terminator",
            ),
            (
                S::FlowChartPreparation,
                "フローチャート: 準備",
                "Flowchart: Preparation",
            ),
            (
                S::FlowChartManualInput,
                "フローチャート: 手操作入力",
                "Flowchart: Manual Input",
            ),
            (
                S::FlowChartManualOperation,
                "フローチャート: 手作業",
                "Flowchart: Manual Operation",
            ),
            (
                S::FlowChartConnector,
                "フローチャート: 結合子",
                "Flowchart: Connector",
            ),
            (
                S::FlowChartOffpageConnector,
                "フローチャート: 他ページ結合子",
                "Flowchart: Off-page Connector",
            ),
            (
                S::FlowChartPunchedTape,
                "フローチャート: せん孔テープ",
                "Flowchart: Punched Tape",
            ),
            (
                S::FlowChartSummingJunction,
                "フローチャート: 和接合",
                "Flowchart: Summing Junction",
            ),
            (S::FlowChartOr, "フローチャート: 論理和", "Flowchart: Or"),
            (
                S::FlowChartCollate,
                "フローチャート: 照合",
                "Flowchart: Collate",
            ),
            (S::FlowChartSort, "フローチャート: 分類", "Flowchart: Sort"),
            (
                S::FlowChartExtract,
                "フローチャート: 抜き出し",
                "Flowchart: Extract",
            ),
            (
                S::FlowChartMerge,
                "フローチャート: 組合せ",
                "Flowchart: Merge",
            ),
            (
                S::FlowChartDelay,
                "フローチャート: 論理積ゲート",
                "Flowchart: Delay",
            ),
            (
                S::FlowChartMagneticDisk,
                "フローチャート: 磁気ディスク",
                "Flowchart: Magnetic Disk",
            ),
            (
                S::FlowChartMagneticDrum,
                "フローチャート: 直接アクセス記憶",
                "Flowchart: Direct Access Storage",
            ),
            (
                S::FlowChartDisplay,
                "フローチャート: 表示",
                "Flowchart: Display",
            ),
        ],
    },
    Category {
        ja: "星とリボン",
        en: "Stars and Banners",
        shapes: &[
            (S::IrregularSealOne, "爆発: 8 pt", "Explosion: 8 Points"),
            (S::IrregularSealTwo, "爆発: 14 pt", "Explosion: 14 Points"),
            (S::FourPointStar, "星: 4 pt", "Star: 4 Points"),
            (S::FivePointStar, "星: 5 pt", "Star: 5 Points"),
            (S::SixPointStar, "星: 6 pt", "Star: 6 Points"),
            (S::SevenPointStar, "星: 7 pt", "Star: 7 Points"),
            (S::EightPointStar, "星: 8 pt", "Star: 8 Points"),
            (S::TenPointStar, "星: 10 pt", "Star: 10 Points"),
            (S::TwelvePointStar, "星: 12 pt", "Star: 12 Points"),
            (S::SixteenPointStar, "星: 16 pt", "Star: 16 Points"),
            (S::TwentyFourPointStar, "星: 24 pt", "Star: 24 Points"),
            (S::ThirtyTwoPointStar, "星: 32 pt", "Star: 32 Points"),
            (S::RibbonTwo, "リボン: 上に曲がる", "Ribbon: Tilted Up"),
            (S::Ribbon, "リボン: 下に曲がる", "Ribbon: Tilted Down"),
            (S::Wave, "波線", "Wave"),
            (S::DoubleWave, "小波", "Double Wave"),
        ],
    },
    Category {
        ja: "吹き出し",
        en: "Callouts",
        shapes: &[
            (
                S::WedgeRectangleCallout,
                "吹き出し: 四角形",
                "Speech Bubble: Rectangle",
            ),
            (
                S::WedgeRoundedRectangleCallout,
                "吹き出し: 角を丸めた四角形",
                "Speech Bubble: Rectangle with Corners Rounded",
            ),
            (
                S::WedgeEllipseCallout,
                "吹き出し: 円形",
                "Speech Bubble: Oval",
            ),
            (
                S::CloudCallout,
                "思考の吹き出し: 雲形",
                "Thought Bubble: Cloud",
            ),
        ],
    },
];

/// A shape's name in the gallery, in Japanese or English.
pub fn shape_name(shape: TextBoxShape, japanese: bool) -> &'static str {
    CATEGORIES
        .iter()
        .flat_map(|category| category.shapes.iter())
        .find(|(candidate, _, _)| *candidate == shape)
        .map(|(_, ja, en)| if japanese { *ja } else { *en })
        .unwrap_or(if japanese { "図形" } else { "Shape" })
}

/// 図形に合わせてトリミング's shapes: every closed one.
pub fn crop_shapes() -> impl Iterator<Item = (TextBoxShape, &'static str, &'static str)> {
    CATEGORIES
        .iter()
        .skip(1)
        .flat_map(|category| category.shapes.iter().copied())
        .filter(|(shape, _, _)| !shape.is_line() && !shape.is_open_outline())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_preset_jade_draws_is_in_the_gallery_once() {
        let mut listed: Vec<_> = CATEGORIES
            .iter()
            .flat_map(|category| category.shapes.iter().map(|(shape, _, _)| *shape))
            .collect();
        let count = listed.len();
        listed.dedup();
        assert_eq!(listed.len(), count);
        for shape in TextBoxShape::EXACT_PRESETS {
            assert!(
                listed.contains(&shape),
                "{shape:?} is missing from the gallery"
            );
        }
    }
}
