//! The built-in layouts, colour schemes and SmartArt styles jade offers,
//! as PowerPoint names and groups them in its galleries.

use crate::data::{COLORS_PREFIX, LAYOUT_PREFIX, STYLE_PREFIX};

/// A gallery category of the SmartArt dialog (SmartArt グラフィックの選択).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Category {
    List,
    Process,
    Cycle,
    Hierarchy,
    Relationship,
    Matrix,
    Pyramid,
    Picture,
}

impl Category {
    pub const ALL: [Category; 8] = [
        Category::List,
        Category::Process,
        Category::Cycle,
        Category::Hierarchy,
        Category::Relationship,
        Category::Matrix,
        Category::Pyramid,
        Category::Picture,
    ];

    /// The `loCatId` the data part names.
    pub fn id(self) -> &'static str {
        match self {
            Category::List => "list",
            Category::Process => "process",
            Category::Cycle => "cycle",
            Category::Hierarchy => "hierarchy",
            Category::Relationship => "relationship",
            Category::Matrix => "matrix",
            Category::Pyramid => "pyramid",
            Category::Picture => "picture",
        }
    }

    pub fn name_ja(self) -> &'static str {
        match self {
            Category::List => "リスト",
            Category::Process => "手順",
            Category::Cycle => "循環",
            Category::Hierarchy => "階層構造",
            Category::Relationship => "集合関係",
            Category::Matrix => "マトリックス",
            Category::Pyramid => "ピラミッド",
            Category::Picture => "図",
        }
    }

    pub fn name_en(self) -> &'static str {
        match self {
            Category::List => "List",
            Category::Process => "Process",
            Category::Cycle => "Cycle",
            Category::Hierarchy => "Hierarchy",
            Category::Relationship => "Relationship",
            Category::Matrix => "Matrix",
            Category::Pyramid => "Pyramid",
            Category::Picture => "Picture",
        }
    }
}

/// A layout jade lays out.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LayoutInfo {
    /// The name after the built-in prefix (`process1`), or the whole id
    /// for layouts from later releases.
    pub short: &'static str,
    pub name_ja: &'static str,
    pub name_en: &'static str,
    pub category: Category,
    /// The presentation name PowerPoint gives a bullet's main shape, which
    /// a shape's own formatting is filed under.
    pub node_name: &'static str,
    /// Whether the layout draws an assistant apart (組織図).
    pub assistants: bool,
    /// Whether the layout has picture placeholders.
    pub pictures: bool,
}

impl LayoutInfo {
    pub fn id(&self) -> String {
        layout_id(self.short)
    }
}

const fn entry(
    short: &'static str,
    name_ja: &'static str,
    name_en: &'static str,
    category: Category,
    node_name: &'static str,
) -> LayoutInfo {
    LayoutInfo {
        short,
        name_ja,
        name_en,
        category,
        node_name,
        assistants: false,
        pictures: false,
    }
}

const fn picture(
    short: &'static str,
    name_ja: &'static str,
    name_en: &'static str,
    node_name: &'static str,
) -> LayoutInfo {
    LayoutInfo {
        short,
        name_ja,
        name_en,
        category: Category::Picture,
        node_name,
        assistants: false,
        pictures: true,
    }
}

const ORG: &str = "urn:microsoft.com/office/officeart/2008/layout/NameandTitleOrganizationalChart";

/// The layouts, in gallery order within each category.
pub const LAYOUTS: [LayoutInfo; 36] = [
    entry(
        "default",
        "基本ブロック リスト",
        "Basic Block List",
        Category::List,
        "node",
    ),
    entry(
        "vList2",
        "縦方向箇条書きリスト",
        "Vertical Bullet List",
        Category::List,
        "parentText",
    ),
    entry(
        "hList1",
        "横方向箇条書きリスト",
        "Horizontal Bullet List",
        Category::List,
        "parTx",
    ),
    entry(
        "lProcess2",
        "グループ リスト",
        "Grouped List",
        Category::List,
        "aNode",
    ),
    entry(
        "vList5",
        "縦方向ブロック リスト",
        "Vertical Block List",
        Category::List,
        "parentText",
    ),
    entry(
        "process1",
        "基本ステップ",
        "Basic Process",
        Category::Process,
        "node",
    ),
    entry(
        "process2",
        "縦方向ステップ",
        "Vertical Process",
        Category::Process,
        "node",
    ),
    entry(
        "chevron1",
        "基本 V 字型プロセス",
        "Basic Chevron Process",
        Category::Process,
        "parTx",
    ),
    entry(
        "hProcess9",
        "連続ブロック プロセス",
        "Continuous Block Process",
        Category::Process,
        "textNode",
    ),
    entry(
        "bProcess4",
        "基本蛇行ステップ",
        "Basic Bending Process",
        Category::Process,
        "node",
    ),
    entry(
        "cycle2",
        "基本の循環",
        "Basic Cycle",
        Category::Cycle,
        "node",
    ),
    entry(
        "cycle1",
        "テキスト循環",
        "Text Cycle",
        Category::Cycle,
        "node",
    ),
    entry(
        "cycle3",
        "ブロックの循環",
        "Block Cycle",
        Category::Cycle,
        "nodeFirstNode",
    ),
    entry(
        "radial1",
        "基本放射",
        "Basic Radial",
        Category::Cycle,
        "centerShape",
    ),
    entry(
        "chart3",
        "基本円グラフ",
        "Basic Pie",
        Category::Cycle,
        "wedge1",
    ),
    LayoutInfo {
        assistants: true,
        ..entry(
            "orgChart1",
            "組織図",
            "Organization Chart",
            Category::Hierarchy,
            "rootText1",
        )
    },
    LayoutInfo {
        assistants: true,
        ..entry(
            ORG,
            "氏名/役職名付き組織図",
            "Name and Title Organization Chart",
            Category::Hierarchy,
            "rootText1",
        )
    },
    entry(
        "hierarchy1",
        "階層",
        "Hierarchy",
        Category::Hierarchy,
        "text",
    ),
    entry(
        "hierarchy2",
        "横方向階層",
        "Horizontal Hierarchy",
        Category::Hierarchy,
        "LevelOneTextNode",
    ),
    entry(
        "hierarchy3",
        "階層リスト",
        "Hierarchy List",
        Category::Hierarchy,
        "rootText",
    ),
    entry(
        "venn1",
        "基本ベン図",
        "Basic Venn",
        Category::Relationship,
        "circ1",
    ),
    entry(
        "venn3",
        "横方向ベン図",
        "Linear Venn",
        Category::Relationship,
        "Name5",
    ),
    entry(
        "venn2",
        "積み重ねベン図",
        "Stacked Venn",
        Category::Relationship,
        "circle1",
    ),
    entry(
        "target1",
        "ネスト ターゲット",
        "Nested Target",
        Category::Relationship,
        "text1",
    ),
    entry(
        "equation1",
        "数式",
        "Equation",
        Category::Relationship,
        "node",
    ),
    entry(
        "matrix3",
        "基本マトリックス",
        "Basic Matrix",
        Category::Matrix,
        "quad1",
    ),
    entry(
        "matrix1",
        "タイトル付きマトリックス",
        "Titled Matrix",
        Category::Matrix,
        "centerTile",
    ),
    entry(
        "matrix2",
        "グリッド マトリックス",
        "Grid Matrix",
        Category::Matrix,
        "rect1",
    ),
    entry(
        "pyramid1",
        "基本ピラミッド",
        "Basic Pyramid",
        Category::Pyramid,
        "level",
    ),
    entry(
        "pyramid3",
        "反転ピラミッド",
        "Inverted Pyramid",
        Category::Pyramid,
        "level",
    ),
    entry(
        "pyramid2",
        "ピラミッド リスト",
        "Pyramid List",
        Category::Pyramid,
        "aNode",
    ),
    entry(
        "pyramid4",
        "分割ピラミッド",
        "Segmented Pyramid",
        Category::Pyramid,
        "triangle1",
    ),
    picture(
        "pList1",
        "画像キャプション リスト",
        "Picture Caption List",
        "textRect",
    ),
    picture(
        "urn:microsoft.com/office/officeart/2008/layout/TitledPictureBlocks",
        "タイトル付き画像ブロック",
        "Titled Picture Blocks",
        "ParentText",
    ),
    picture(
        "urn:microsoft.com/office/officeart/2008/layout/PictureStrips",
        "画像ストリップ",
        "Picture Strips",
        "rect1",
    ),
    picture(
        "urn:microsoft.com/office/officeart/2008/layout/CaptionedPictures",
        "キャプション付き画像",
        "Captioned Pictures",
        "Parent",
    ),
];

/// The full `uniqueId` for a layout named by its short name or its id.
pub fn layout_id(name: &str) -> String {
    if name.contains(':') {
        name.to_string()
    } else {
        format!("{LAYOUT_PREFIX}{name}")
    }
}

/// The layout with `id` (full or short), when jade lays it out by name.
/// Office numbers a layout it changed (`process1#1`); that is the same
/// layout.
pub fn layout(id: &str) -> Option<&'static LayoutInfo> {
    let id = id.split('#').next().unwrap_or(id);
    let full = layout_id(id);
    LAYOUTS.iter().find(|info| info.id() == full)
}

/// A colour scheme of 色の変更.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ColorScheme {
    pub short: &'static str,
    pub name_ja: &'static str,
    pub name_en: &'static str,
    /// Its gallery group, which is also the `csCatId`.
    pub category: &'static str,
}

impl ColorScheme {
    pub fn id(&self) -> String {
        colors_id(self.short)
    }
}

const fn scheme(
    short: &'static str,
    name_ja: &'static str,
    name_en: &'static str,
    category: &'static str,
) -> ColorScheme {
    ColorScheme {
        short,
        name_ja,
        name_en,
        category,
    }
}

/// The colour schemes, in gallery order: the theme's main colours, the
/// colourful ones, then five per accent.
pub const COLOR_SCHEMES: [ColorScheme; 38] = [
    scheme(
        "accent0_1",
        "濃色 1 アウトライン",
        "Dark 1 Outline",
        "mainScheme",
    ),
    scheme(
        "accent0_2",
        "濃色 2 アウトライン",
        "Dark 2 Outline",
        "mainScheme",
    ),
    scheme(
        "accent0_3",
        "濃色 2 塗りつぶし",
        "Dark 2 Fill",
        "mainScheme",
    ),
    scheme(
        "colorful1",
        "カラフル - アクセント カラー",
        "Colorful - Accent Colors",
        "colorful",
    ),
    scheme(
        "colorful2",
        "カラフル範囲 - アクセント カラー 2 から 3",
        "Colorful Range - Accent Colors 2 to 3",
        "colorful",
    ),
    scheme(
        "colorful3",
        "カラフル範囲 - アクセント カラー 3 から 4",
        "Colorful Range - Accent Colors 3 to 4",
        "colorful",
    ),
    scheme(
        "colorful4",
        "カラフル範囲 - アクセント カラー 4 から 5",
        "Colorful Range - Accent Colors 4 to 5",
        "colorful",
    ),
    scheme(
        "colorful5",
        "カラフル範囲 - アクセント カラー 5 から 6",
        "Colorful Range - Accent Colors 5 to 6",
        "colorful",
    ),
    scheme(
        "accent1_1",
        "枠線のみ - アクセント 1",
        "Colored Outline - Accent 1",
        "accent1",
    ),
    scheme(
        "accent1_2",
        "塗りつぶし - アクセント 1",
        "Colored Fill - Accent 1",
        "accent1",
    ),
    scheme(
        "accent1_3",
        "グラデーション範囲 - アクセント 1",
        "Gradient Range - Accent 1",
        "accent1",
    ),
    scheme(
        "accent1_4",
        "グラデーション ループ - アクセント 1",
        "Gradient Loop - Accent 1",
        "accent1",
    ),
    scheme(
        "accent1_5",
        "透過グラデーション範囲 - アクセント 1",
        "Transparent Gradient Range - Accent 1",
        "accent1",
    ),
    scheme(
        "accent2_1",
        "枠線のみ - アクセント 2",
        "Colored Outline - Accent 2",
        "accent2",
    ),
    scheme(
        "accent2_2",
        "塗りつぶし - アクセント 2",
        "Colored Fill - Accent 2",
        "accent2",
    ),
    scheme(
        "accent2_3",
        "グラデーション範囲 - アクセント 2",
        "Gradient Range - Accent 2",
        "accent2",
    ),
    scheme(
        "accent2_4",
        "グラデーション ループ - アクセント 2",
        "Gradient Loop - Accent 2",
        "accent2",
    ),
    scheme(
        "accent2_5",
        "透過グラデーション範囲 - アクセント 2",
        "Transparent Gradient Range - Accent 2",
        "accent2",
    ),
    scheme(
        "accent3_1",
        "枠線のみ - アクセント 3",
        "Colored Outline - Accent 3",
        "accent3",
    ),
    scheme(
        "accent3_2",
        "塗りつぶし - アクセント 3",
        "Colored Fill - Accent 3",
        "accent3",
    ),
    scheme(
        "accent3_3",
        "グラデーション範囲 - アクセント 3",
        "Gradient Range - Accent 3",
        "accent3",
    ),
    scheme(
        "accent3_4",
        "グラデーション ループ - アクセント 3",
        "Gradient Loop - Accent 3",
        "accent3",
    ),
    scheme(
        "accent3_5",
        "透過グラデーション範囲 - アクセント 3",
        "Transparent Gradient Range - Accent 3",
        "accent3",
    ),
    scheme(
        "accent4_1",
        "枠線のみ - アクセント 4",
        "Colored Outline - Accent 4",
        "accent4",
    ),
    scheme(
        "accent4_2",
        "塗りつぶし - アクセント 4",
        "Colored Fill - Accent 4",
        "accent4",
    ),
    scheme(
        "accent4_3",
        "グラデーション範囲 - アクセント 4",
        "Gradient Range - Accent 4",
        "accent4",
    ),
    scheme(
        "accent4_4",
        "グラデーション ループ - アクセント 4",
        "Gradient Loop - Accent 4",
        "accent4",
    ),
    scheme(
        "accent4_5",
        "透過グラデーション範囲 - アクセント 4",
        "Transparent Gradient Range - Accent 4",
        "accent4",
    ),
    scheme(
        "accent5_1",
        "枠線のみ - アクセント 5",
        "Colored Outline - Accent 5",
        "accent5",
    ),
    scheme(
        "accent5_2",
        "塗りつぶし - アクセント 5",
        "Colored Fill - Accent 5",
        "accent5",
    ),
    scheme(
        "accent5_3",
        "グラデーション範囲 - アクセント 5",
        "Gradient Range - Accent 5",
        "accent5",
    ),
    scheme(
        "accent5_4",
        "グラデーション ループ - アクセント 5",
        "Gradient Loop - Accent 5",
        "accent5",
    ),
    scheme(
        "accent5_5",
        "透過グラデーション範囲 - アクセント 5",
        "Transparent Gradient Range - Accent 5",
        "accent5",
    ),
    scheme(
        "accent6_1",
        "枠線のみ - アクセント 6",
        "Colored Outline - Accent 6",
        "accent6",
    ),
    scheme(
        "accent6_2",
        "塗りつぶし - アクセント 6",
        "Colored Fill - Accent 6",
        "accent6",
    ),
    scheme(
        "accent6_3",
        "グラデーション範囲 - アクセント 6",
        "Gradient Range - Accent 6",
        "accent6",
    ),
    scheme(
        "accent6_4",
        "グラデーション ループ - アクセント 6",
        "Gradient Loop - Accent 6",
        "accent6",
    ),
    scheme(
        "accent6_5",
        "透過グラデーション範囲 - アクセント 6",
        "Transparent Gradient Range - Accent 6",
        "accent6",
    ),
];

pub fn colors_id(name: &str) -> String {
    if name.contains(':') {
        name.to_string()
    } else {
        format!("{COLORS_PREFIX}{name}")
    }
}

pub fn color_scheme(id: &str) -> Option<&'static ColorScheme> {
    let full = colors_id(id.split('#').next().unwrap_or(id));
    COLOR_SCHEMES.iter().find(|scheme| scheme.id() == full)
}

/// A SmartArt style of SmartArt のスタイル.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Style {
    pub short: &'static str,
    pub name_ja: &'static str,
    pub name_en: &'static str,
    /// `simple` (ドキュメントに最適なスタイル) or `3D`.
    pub category: &'static str,
}

impl Style {
    pub fn id(&self) -> String {
        style_id(self.short)
    }
}

const fn look(
    short: &'static str,
    name_ja: &'static str,
    name_en: &'static str,
    category: &'static str,
) -> Style {
    Style {
        short,
        name_ja,
        name_en,
        category,
    }
}

pub const STYLES: [Style; 13] = [
    look("simple1", "シンプル", "Simple Fill", "simple"),
    look("simple2", "白枠", "White Outline", "simple"),
    look("simple3", "グラデーション", "Subtle Effect", "simple"),
    look("simple4", "中", "Moderate Effect", "simple"),
    look("simple5", "光沢", "Intense Effect", "simple"),
    look("3d1", "パステル", "Polished", "3D"),
    look("3d2", "はめ込み", "Inset", "3D"),
    look("3d3", "漫画", "Cartoon", "3D"),
    look("3d4", "パウダー", "Powder", "3D"),
    look("3d5", "レンガ", "Brick Scene", "3D"),
    look("3d6", "フラット", "Flat Scene", "3D"),
    look("3d7", "メタリック", "Metallic Scene", "3D"),
    look("3d9", "鳥瞰", "Bird's Eye Scene", "3D"),
];

pub fn style_id(name: &str) -> String {
    if name.contains(':') {
        name.to_string()
    } else {
        format!("{STYLE_PREFIX}{name}")
    }
}

pub fn style(id: &str) -> Option<&'static Style> {
    let full = style_id(id.split('#').next().unwrap_or(id));
    STYLES.iter().find(|style| style.id() == full)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_category_has_layouts() {
        for category in Category::ALL {
            assert!(
                LAYOUTS.iter().filter(|l| l.category == category).count() >= 3,
                "{category:?}"
            );
        }
        assert_eq!(
            layout("urn:microsoft.com/office/officeart/2005/8/layout/process1#1")
                .unwrap()
                .short,
            "process1"
        );
        assert!(color_scheme("colorful1").is_some());
        assert!(style("3d1").is_some());
    }
}
