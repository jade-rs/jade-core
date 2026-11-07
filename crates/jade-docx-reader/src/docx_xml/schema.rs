//! What the WordprocessingML schema requires of a written part beyond being
//! well-formed XML: the order of children in a sequence, and a declaration for
//! every namespace prefix in use.
//!
//! Word treats either violation as a damaged file: it asks to repair a part
//! whose `w:rPr` lists `w:sz` before `w:color`, and refuses outright to open
//! one that uses an undeclared prefix. Edits insert properties wherever is
//! convenient, so the writer, not every edit, owns the order (ECMA-376 Part 1,
//! the `CT_*` sequences cited below).

use std::collections::{BTreeSet, HashMap, HashSet};

use super::{XmlElement, XmlNode, XmlNodeId};

/// Where a child goes among its siblings.
#[derive(Clone, Copy)]
enum Unlisted {
    /// Children the table does not list keep the place they have, right after
    /// the sibling they follow. Extension elements (`w14:*`, `mc:*`) land here.
    Follow,
    /// Children the table does not list all sort at this rank. Used where the
    /// table only pins the properties element to the front (`w:p`, `w:r`).
    Rank(u16),
}

struct Sequence {
    children: &'static [&'static [&'static str]],
    unlisted: Unlisted,
}

impl Sequence {
    fn rank(&self, name: &str) -> Option<u16> {
        self.children
            .iter()
            .position(|group| group.contains(&name))
            .map(|position| position as u16)
            .or(match self.unlisted {
                Unlisted::Follow => None,
                Unlisted::Rank(rank) => Some(rank),
            })
    }
}

macro_rules! sequence {
    ($unlisted:expr; $($group:expr),* $(,)?) => {
        Some(&Sequence { children: &[$(&$group),*], unlisted: $unlisted })
    };
}

/// The schema sequence of `parent`'s children, for the containers edits write
/// into. Names inside one group form an `xsd:choice` and may interleave.
/// `grandparent` tells the paragraph properties of a style or list level
/// (CT_PPrGeneral, which has no run or section properties) from a paragraph's.
fn sequence(parent: &str, grandparent: Option<&str>) -> Option<&'static Sequence> {
    use Unlisted::{Follow, Rank};
    match parent {
        // CT_PPrGeneral: what a paragraph's properties allow, less `w:rPr` and
        // `w:sectPr`. A stray one there stays where the author put it.
        "w:pPr"
            if matches!(
                grandparent,
                Some("w:style" | "w:lvl" | "w:pPrDefault" | "w:tblStylePr")
            ) =>
        {
            sequence!(Follow;
                ["w:pStyle"], ["w:keepNext"], ["w:keepLines"], ["w:pageBreakBefore"],
                ["w:framePr"], ["w:widowControl"], ["w:numPr"], ["w:suppressLineNumbers"],
                ["w:pBdr"], ["w:shd"], ["w:tabs"], ["w:suppressAutoHyphens"], ["w:kinsoku"],
                ["w:wordWrap"], ["w:overflowPunct"], ["w:topLinePunct"], ["w:autoSpaceDE"],
                ["w:autoSpaceDN"], ["w:bidi"], ["w:adjustRightInd"], ["w:snapToGrid"],
                ["w:spacing"], ["w:ind"], ["w:contextualSpacing"], ["w:mirrorIndents"],
                ["w:suppressOverlap"], ["w:jc"], ["w:textDirection"], ["w:textAlignment"],
                ["w:textboxTightWrap"], ["w:outlineLvl"], ["w:divId"], ["w:cnfStyle"],
                ["w:pPrChange"],
            )
        }
        // CT_RPr / CT_ParaRPr (§17.3.2.28, §17.3.1.29). The revision marks
        // only occur in a paragraph mark's properties, and come first there.
        "w:rPr" => sequence!(Follow;
            ["w:ins"], ["w:del"], ["w:moveFrom"], ["w:moveTo"],
            ["w:rStyle"], ["w:rFonts"], ["w:b"], ["w:bCs"], ["w:i"], ["w:iCs"],
            ["w:caps"], ["w:smallCaps"], ["w:strike"], ["w:dstrike"], ["w:outline"],
            ["w:shadow"], ["w:emboss"], ["w:imprint"], ["w:noProof"], ["w:snapToGrid"],
            ["w:vanish"], ["w:webHidden"], ["w:color"], ["w:spacing"], ["w:w"],
            ["w:kern"], ["w:position"], ["w:sz"], ["w:szCs"], ["w:highlight"],
            ["w:u"], ["w:effect"], ["w:bdr"], ["w:shd"], ["w:fitText"],
            ["w:vertAlign"], ["w:rtl"], ["w:cs"], ["w:em"], ["w:lang"],
            ["w:eastAsianLayout"], ["w:specVanish"], ["w:oMath"], ["w:rPrChange"],
        ),
        // CT_PPr (§17.3.1.26).
        "w:pPr" => sequence!(Follow;
            ["w:pStyle"], ["w:keepNext"], ["w:keepLines"], ["w:pageBreakBefore"],
            ["w:framePr"], ["w:widowControl"], ["w:numPr"], ["w:suppressLineNumbers"],
            ["w:pBdr"], ["w:shd"], ["w:tabs"], ["w:suppressAutoHyphens"], ["w:kinsoku"],
            ["w:wordWrap"], ["w:overflowPunct"], ["w:topLinePunct"], ["w:autoSpaceDE"],
            ["w:autoSpaceDN"], ["w:bidi"], ["w:adjustRightInd"], ["w:snapToGrid"],
            ["w:spacing"], ["w:ind"], ["w:contextualSpacing"], ["w:mirrorIndents"],
            ["w:suppressOverlap"], ["w:jc"], ["w:textDirection"], ["w:textAlignment"],
            ["w:textboxTightWrap"], ["w:outlineLvl"], ["w:divId"], ["w:cnfStyle"],
            ["w:rPr"], ["w:sectPr"], ["w:pPrChange"],
        ),
        // CT_SectPr (§17.6.17).
        "w:sectPr" => sequence!(Follow;
            ["w:headerReference", "w:footerReference"], ["w:footnotePr"], ["w:endnotePr"],
            ["w:type"], ["w:pgSz"], ["w:pgMar"], ["w:paperSrc"], ["w:pgBorders"],
            ["w:lnNumType"], ["w:pgNumType"], ["w:cols"], ["w:formProt"], ["w:vAlign"],
            ["w:noEndnote"], ["w:titlePg"], ["w:textDirection"], ["w:bidi"],
            ["w:rtlGutter"], ["w:docGrid"], ["w:printerSettings"], ["w:sectPrChange"],
        ),
        // CT_PBdr, CT_PageBorders, CT_TblBorders, CT_TcBorders, CT_TblCellMar.
        "w:pBdr" => sequence!(Follow;
            ["w:top"], ["w:left"], ["w:bottom"], ["w:right"], ["w:between"], ["w:bar"],
        ),
        "w:pgBorders" => sequence!(Follow; ["w:top"], ["w:left"], ["w:bottom"], ["w:right"]),
        "w:tblBorders" | "w:tcBorders" => sequence!(Follow;
            ["w:top"], ["w:left", "w:start"], ["w:bottom"], ["w:right", "w:end"],
            ["w:insideH"], ["w:insideV"], ["w:tl2br"], ["w:tr2bl"],
        ),
        "w:tblCellMar" | "w:tcMar" => sequence!(Follow;
            ["w:top"], ["w:left", "w:start"], ["w:bottom"], ["w:right", "w:end"],
        ),
        // CT_TblPr (§17.4.60), CT_TrPr (§17.4.82), CT_TcPr (§17.4.70).
        "w:tblPr" => sequence!(Follow;
            ["w:tblStyle"], ["w:tblpPr"], ["w:tblOverlap"], ["w:bidiVisual"],
            ["w:tblStyleRowBandSize"], ["w:tblStyleColBandSize"], ["w:tblW"], ["w:jc"],
            ["w:tblCellSpacing"], ["w:tblInd"], ["w:tblBorders"], ["w:shd"],
            ["w:tblLayout"], ["w:tblCellMar"], ["w:tblLook"], ["w:tblCaption"],
            ["w:tblDescription"], ["w:tblPrChange"],
        ),
        "w:trPr" => sequence!(Rank(0); ["w:ins"], ["w:del"], ["w:trPrChange"]),
        "w:tcPr" => sequence!(Follow;
            ["w:cnfStyle"], ["w:tcW"], ["w:gridSpan"], ["w:hMerge"], ["w:vMerge"],
            ["w:tcBorders"], ["w:shd"], ["w:noWrap"], ["w:tcMar"], ["w:textDirection"],
            ["w:tcFitText"], ["w:vAlign"], ["w:hideMark"], ["w:headers"],
            ["w:cellIns", "w:cellDel", "w:cellMerge"], ["w:tcPrChange"],
        ),
        // The properties element leads its paragraph, run, table, row or cell.
        "w:p" => sequence!(Rank(1); ["w:pPr"]),
        "w:r" => sequence!(Rank(1); ["w:rPr"]),
        "w:tbl" => sequence!(Rank(2); ["w:tblPr"], ["w:tblGrid"]),
        "w:tr" => sequence!(Rank(2); ["w:tblPrEx"], ["w:trPr"]),
        "w:tc" => sequence!(Rank(1); ["w:tcPr"]),
        "w:body" => sequence!(Rank(0); [], ["w:sectPr"]),
        "w:numPr" => sequence!(Follow; ["w:ilvl"], ["w:numId"], ["w:numberingChange"], ["w:ins"]),
        // numbering.xml: CT_Numbering, CT_AbstractNum, CT_Lvl, CT_Num.
        "w:numbering" => sequence!(Follow;
            ["w:numPicBullet"], ["w:abstractNum"], ["w:num"], ["w:numIdMacAtCleanup"],
        ),
        "w:abstractNum" => sequence!(Follow;
            ["w:nsid"], ["w:multiLevelType"], ["w:tmpl"], ["w:name"], ["w:styleLink"],
            ["w:numStyleLink"], ["w:lvl"],
        ),
        "w:lvl" => sequence!(Follow;
            ["w:start"], ["w:numFmt"], ["w:lvlRestart"], ["w:pStyle"], ["w:isLgl"],
            ["w:suff"], ["w:lvlText"], ["w:lvlPicBulletId"], ["w:legacy"], ["w:lvlJc"],
            ["w:pPr"], ["w:rPr"],
        ),
        "w:num" => sequence!(Follow; ["w:abstractNumId"], ["w:lvlOverride"]),
        // styles.xml: CT_Style, CT_DocDefaults.
        "w:style" => sequence!(Follow;
            ["w:name"], ["w:aliases"], ["w:basedOn"], ["w:next"], ["w:link"],
            ["w:autoRedefine"], ["w:hidden"], ["w:uiPriority"], ["w:semiHidden"],
            ["w:unhideWhenUsed"], ["w:qFormat"], ["w:locked"], ["w:personal"],
            ["w:personalCompose"], ["w:personalReply"], ["w:rsid"], ["w:pPr"], ["w:rPr"],
            ["w:tblPr"], ["w:trPr"], ["w:tcPr"], ["w:tblStylePr"],
        ),
        "w:docDefaults" => sequence!(Follow; ["w:rPrDefault"], ["w:pPrDefault"]),
        // CT_FtnProps / CT_EdnProps, and the settings variants that list the
        // separator notes after the numbering properties.
        "w:footnotePr" | "w:endnotePr" => sequence!(Follow;
            ["w:pos"], ["w:numFmt"], ["w:numStart"], ["w:numRestart"],
            ["w:footnote", "w:endnote"],
        ),
        // settings.xml: CT_Settings (§17.15.1.78).
        "w:settings" => sequence!(Follow;
            ["w:writeProtection"], ["w:view"], ["w:zoom"], ["w:removePersonalInformation"],
            ["w:removeDateAndTime"], ["w:doNotDisplayPageBoundaries"],
            ["w:displayBackgroundShape"], ["w:printPostScriptOverText"],
            ["w:printFractionalCharacterWidth"], ["w:printFormsData"],
            ["w:embedTrueTypeFonts"], ["w:embedSystemFonts"], ["w:saveSubsetFonts"],
            ["w:saveFormsData"], ["w:mirrorMargins"], ["w:alignBordersAndEdges"],
            ["w:bordersDoNotSurroundHeader"], ["w:bordersDoNotSurroundFooter"],
            ["w:gutterAtTop"], ["w:hideSpellingErrors"], ["w:hideGrammaticalErrors"],
            ["w:activeWritingStyle"], ["w:proofState"], ["w:formsDesign"],
            ["w:attachedTemplate"], ["w:linkStyles"], ["w:stylePaneFormatFilter"],
            ["w:stylePaneSortMethod"], ["w:documentType"], ["w:mailMerge"],
            ["w:revisionView"], ["w:trackRevisions"], ["w:doNotTrackMoves"],
            ["w:doNotTrackFormatting"], ["w:documentProtection"], ["w:autoFormatOverride"],
            ["w:styleLockTheme"], ["w:styleLockQFSet"], ["w:defaultTabStop"],
            ["w:autoHyphenation"], ["w:consecutiveHyphenLimit"], ["w:hyphenationZone"],
            ["w:doNotHyphenateCaps"], ["w:showEnvelope"], ["w:summaryLength"],
            ["w:clickAndTypeStyle"], ["w:defaultTableStyle"], ["w:evenAndOddHeaders"],
            ["w:bookFoldRevPrinting"], ["w:bookFoldPrinting"], ["w:bookFoldPrintingSheets"],
            ["w:drawingGridHorizontalSpacing"], ["w:drawingGridVerticalSpacing"],
            ["w:displayHorizontalDrawingGridEvery"], ["w:displayVerticalDrawingGridEvery"],
            ["w:doNotUseMarginsForDrawingGridOrigin"], ["w:drawingGridHorizontalOrigin"],
            ["w:drawingGridVerticalOrigin"], ["w:doNotShadeFormData"],
            ["w:noPunctuationKerning"], ["w:characterSpacingControl"], ["w:printTwoOnOne"],
            ["w:strictFirstAndLastChars"], ["w:noLineBreaksAfter"], ["w:noLineBreaksBefore"],
            ["w:savePreviewPicture"], ["w:doNotValidateAgainstSchema"], ["w:saveInvalidXml"],
            ["w:ignoreMixedContent"], ["w:alwaysShowPlaceholderText"],
            ["w:doNotDemarcateInvalidXml"], ["w:saveXmlDataOnly"], ["w:useXSLTWhenSaving"],
            ["w:saveThroughXslt"], ["w:showXMLTags"], ["w:alwaysMergeEmptyNamespace"],
            ["w:updateFields"], ["w:hdrShapeDefaults"], ["w:footnotePr"], ["w:endnotePr"],
            ["w:compat"], ["w:docVars"], ["w:rsids"], ["m:mathPr"], ["w:attachedSchema"],
            ["w:themeFontLang"], ["w:clrSchemeMapping"], ["w:doNotIncludeSubdocsInStats"],
            ["w:doNotAutoCompressPictures"], ["w:forceUpgrade"], ["w:captions"],
            ["w:readModeInkLockDown"], ["w:smartTagType"], ["sl:schemaLibrary"],
            ["w:shapeDefaults"], ["w:doNotEmbedSmartTags"], ["w:decimalSymbol"],
            ["w:listSeparator"],
        ),
        _ => None,
    }
}

/// The order to write `element`'s children in, or `None` when they are in
/// schema order already (the common case, which allocates nothing).
pub(super) fn schema_order(element: &XmlElement, parent: Option<&str>) -> Option<Vec<usize>> {
    let sequence = sequence(&element.name, parent)?;
    // A child the table does not place sorts right after the sibling it
    // follows; one that leads the list sorts first.
    let keys = element.children.iter().scan(0u16, |previous, child| {
        let rank = match child {
            XmlNode::Element(child) => sequence.rank(&child.name),
            _ => None,
        };
        *previous = rank.unwrap_or(*previous);
        Some(*previous)
    });
    // Nearly every element is in order already: find that out before
    // allocating, since export asks this of every element it writes.
    if keys.clone().is_sorted() {
        return None;
    }
    let keys: Vec<u16> = keys.collect();
    let mut order: Vec<usize> = (0..keys.len()).collect();
    order.sort_by_key(|&index| keys[index]);
    Some(order)
}

/// Namespaces an edit may introduce into a part that never declared them.
const KNOWN_NAMESPACES: &[(&str, &str)] = &[
    (
        "w",
        "http://schemas.openxmlformats.org/wordprocessingml/2006/main",
    ),
    (
        "r",
        "http://schemas.openxmlformats.org/officeDocument/2006/relationships",
    ),
    (
        "wp",
        "http://schemas.openxmlformats.org/drawingml/2006/wordprocessingDrawing",
    ),
    ("a", "http://schemas.openxmlformats.org/drawingml/2006/main"),
    (
        "pic",
        "http://schemas.openxmlformats.org/drawingml/2006/picture",
    ),
    (
        "c",
        "http://schemas.openxmlformats.org/drawingml/2006/chart",
    ),
    (
        "m",
        "http://schemas.openxmlformats.org/officeDocument/2006/math",
    ),
    (
        "mc",
        "http://schemas.openxmlformats.org/markup-compatibility/2006",
    ),
    ("v", "urn:schemas-microsoft-com:vml"),
    ("o", "urn:schemas-microsoft-com:office:office"),
    ("w10", "urn:schemas-microsoft-com:office:word"),
    (
        "wne",
        "http://schemas.microsoft.com/office/word/2006/wordml",
    ),
    (
        "wpc",
        "http://schemas.microsoft.com/office/word/2010/wordprocessingCanvas",
    ),
    (
        "wpg",
        "http://schemas.microsoft.com/office/word/2010/wordprocessingGroup",
    ),
    (
        "wps",
        "http://schemas.microsoft.com/office/word/2010/wordprocessingShape",
    ),
    (
        "wpi",
        "http://schemas.microsoft.com/office/word/2010/wordprocessingInk",
    ),
    (
        "wp14",
        "http://schemas.microsoft.com/office/word/2010/wordprocessingDrawing",
    ),
    (
        "w14",
        "http://schemas.microsoft.com/office/word/2010/wordml",
    ),
    (
        "w15",
        "http://schemas.microsoft.com/office/word/2012/wordml",
    ),
    (
        "w16",
        "http://schemas.microsoft.com/office/word/2018/wordml",
    ),
    (
        "w16se",
        "http://schemas.microsoft.com/office/word/2015/wordml/symex",
    ),
    (
        "w16cid",
        "http://schemas.microsoft.com/office/word/2016/wordml/cid",
    ),
    (
        "w16cex",
        "http://schemas.microsoft.com/office/word/2018/wordml/cex",
    ),
    (
        "w16sdtdh",
        "http://schemas.microsoft.com/office/word/2020/wordml/sdtdatahash",
    ),
    (
        "w16sdtfl",
        "http://schemas.microsoft.com/office/word/2024/wordml/sdtformatlock",
    ),
    (
        "w16du",
        "http://schemas.microsoft.com/office/word/2023/wordml/word16du",
    ),
];

/// Prefixes for namespaces newer than ECMA-376 1st edition. A consumer that
/// predates one must be allowed to skip it, so a declaration this module adds
/// for one also lists it in `mc:Ignorable`, as Word does.
const IGNORABLE: &[&str] = &[
    "w14", "w15", "w16", "w16se", "w16cid", "w16cex", "w16sdtdh", "w16sdtfl", "w16du", "wp14",
];

/// What the root element needs so every prefix the part uses is declared.
#[derive(Debug, Default)]
pub(super) struct RootNamespaceFix {
    /// `xmlns:*` attributes to append.
    pub declarations: Vec<(String, String)>,
    /// The new value of `mc:Ignorable`, replacing any the root carries.
    pub ignorable: Option<String>,
}

/// What writing a part changes besides the order of children.
#[derive(Debug, Default)]
pub(super) struct WritePlan {
    /// Declarations for prefixes the part uses without declaring them.
    pub namespaces: Option<RootNamespaceFix>,
    /// New `w:id`s for revision elements that repeat an earlier one's, keyed
    /// by node. Splitting a paragraph or run copies the revision wrapping it;
    /// the editor treats the pieces as one revision, but Word wants each
    /// element to carry its own id, as it writes them itself. Fresh ids count
    /// up from the largest `w:id` in the part, so they collide with nothing.
    pub renumbered: HashMap<XmlNodeId, String>,
}

impl WritePlan {
    pub fn is_empty(&self) -> bool {
        self.namespaces.is_none() && self.renumbered.is_empty()
    }
}

/// Revision elements whose `w:id` must be unique (§17.13.1). Range markers
/// (`w:bookmarkStart`/`End`, `w:commentRange*`, `w:move*RangeStart`/`End`)
/// pair up by id and are not listed.
const REVISION_ELEMENTS: &[&str] = &[
    "w:ins",
    "w:del",
    "w:moveFrom",
    "w:moveTo",
    "w:rPrChange",
    "w:pPrChange",
    "w:sectPrChange",
    "w:tblPrChange",
    "w:trPrChange",
    "w:tcPrChange",
    "w:tblGridChange",
    "w:tblPrExChange",
    "w:numberingChange",
    "w:cellIns",
    "w:cellDel",
    "w:cellMerge",
];

/// Plans the write of the tree under `root` in one walk, so export pays for a
/// single extra pass over the part.
pub(super) fn write_plan(root: &XmlElement) -> WritePlan {
    let mut walk = Walk::default();
    walk.visit(root);
    WritePlan {
        namespaces: namespace_fix(root, &walk.missing),
        renumbered: walk
            .duplicates
            .into_iter()
            .zip(walk.maximum + 1..)
            .map(|(node, id)| (node, id.to_string()))
            .collect(),
    }
}

/// A prefix declared further down the tree counts only inside that subtree;
/// prefixes this module does not know are left alone.
fn namespace_fix(root: &XmlElement, missing: &BTreeSet<&str>) -> Option<RootNamespaceFix> {
    let mut fix = RootNamespaceFix::default();
    let mut ignorable: Vec<&str> = Vec::new();
    for prefix in missing {
        if let Some((_, uri)) = KNOWN_NAMESPACES.iter().find(|(known, _)| known == prefix) {
            fix.declarations
                .push((format!("xmlns:{prefix}"), (*uri).to_string()));
            if IGNORABLE.contains(prefix) {
                ignorable.push(prefix);
            }
        }
    }
    if !ignorable.is_empty() {
        let declared = |name: &str| {
            root.attributes
                .iter()
                .any(|attribute| attribute.name == name)
        };
        if !declared("xmlns:mc") && !missing.contains("mc") {
            fix.declarations.push((
                "xmlns:mc".to_string(),
                "http://schemas.openxmlformats.org/markup-compatibility/2006".to_string(),
            ));
        }
        let mut listed: Vec<&str> = root
            .attributes
            .iter()
            .find(|attribute| attribute.name == "mc:Ignorable")
            .map(|attribute| attribute.value.split_whitespace().collect())
            .unwrap_or_default();
        for prefix in ignorable {
            if !listed.contains(&prefix) {
                listed.push(prefix);
            }
        }
        fix.ignorable = Some(listed.join(" "));
    }
    (!fix.declarations.is_empty() || fix.ignorable.is_some()).then_some(fix)
}

/// The walk behind [`write_plan`]. `scope` holds the prefixes declared at the
/// current point; `checked` remembers prefixes already found in it, so the
/// walk does not search `scope` (a root often declares thirty namespaces) for
/// every `w:` name. Entries made inside a subtree that declares its own
/// namespaces are dropped on leaving it.
#[derive(Default)]
struct Walk<'a> {
    scope: Vec<&'a str>,
    checked: Vec<&'a str>,
    missing: BTreeSet<&'a str>,
    revision_ids: HashSet<&'a str>,
    duplicates: Vec<XmlNodeId>,
    maximum: u64,
}

impl<'a> Walk<'a> {
    fn visit(&mut self, element: &'a XmlElement) {
        let depth = self.scope.len();
        let checked_depth = self.checked.len();
        for attribute in &element.attributes {
            if let Some(prefix) = attribute.name.strip_prefix("xmlns:") {
                self.scope.push(prefix);
            }
        }
        self.check_prefix(&element.name);
        for attribute in &element.attributes {
            self.check_prefix(&attribute.name);
            if attribute.name == "w:id" {
                if let Ok(value) = attribute.value.parse::<u64>() {
                    self.maximum = self.maximum.max(value);
                }
                if REVISION_ELEMENTS.contains(&element.name.as_str())
                    && !self.revision_ids.insert(&attribute.value)
                {
                    self.duplicates.push(element.id);
                }
            }
        }
        for child in &element.children {
            if let XmlNode::Element(child) = child {
                self.visit(child);
            }
        }
        if self.scope.len() > depth {
            self.scope.truncate(depth);
            self.checked.truncate(checked_depth);
        }
    }

    fn check_prefix(&mut self, name: &'a str) {
        let Some(colon) = name.as_bytes().iter().position(|&byte| byte == b':') else {
            return;
        };
        // ASCII ':' is a UTF-8 boundary, including in non-ASCII names.
        let prefix = &name[..colon];
        if prefix == "xml" || prefix == "xmlns" || self.checked.contains(&prefix) {
            return;
        }
        if self.scope.contains(&prefix) {
            self.checked.push(prefix);
        } else {
            self.missing.insert(prefix);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::DocxXmlDocument;
    use proptest::prelude::*;

    proptest! {
        #[test]
        fn namespace_prefix_cache_matches_reference(
            names in prop::collection::vec(
                prop_oneof![
                    any::<String>(),
                    (any::<String>(), any::<String>()).prop_map(|(prefix, local)| format!("{prefix}:{local}")),
                    prop::sample::select(vec!["w:t", "m:oMath", "custom:item", "xml:space", "xmlns:w", "other:item"])
                        .prop_map(str::to_owned),
                ],
                0..32,
            ),
        ) {
            let scope = vec!["w", "m", "custom"];
            let mut walk = super::Walk { scope: scope.clone(), ..Default::default() };
            let mut missing = std::collections::BTreeSet::new();
            let mut checked = Vec::new();
            for name in &names {
                walk.check_prefix(name);
                if let Some((prefix, _)) = name.split_once(':') {
                    if !matches!(prefix, "xml" | "xmlns") {
                        if scope.contains(&prefix) {
                            if !checked.contains(&prefix) {
                                checked.push(prefix);
                            }
                        } else {
                            missing.insert(prefix);
                        }
                    }
                }
                prop_assert_eq!(&walk.missing, &missing);
                prop_assert_eq!(&walk.checked, &checked);
                prop_assert_eq!(&walk.scope, &scope);
            }
        }
    }

    const W: &str = r#"xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main""#;

    fn rewrite(body: &str) -> String {
        let xml = format!(r#"<w:document {W}><w:body>{body}</w:body></w:document>"#);
        let written = DocxXmlDocument::parse(&xml)
            .and_then(|document| document.to_string())
            .expect("round trip");
        let start = written.find("<w:body>").expect("body") + "<w:body>".len();
        let end = written.find("</w:body>").expect("body end");
        written[start..end].to_string()
    }

    #[test]
    fn run_properties_are_written_in_schema_order() {
        assert_eq!(
            rewrite(
                r#"<w:p><w:r><w:rPr><w:sz w:val="28"/><w:bCs/><w:color w:val="C00000"/><w:b/><w:rStyle w:val="A"/></w:rPr><w:t>x</w:t></w:r></w:p>"#
            ),
            r#"<w:p><w:r><w:rPr><w:rStyle w:val="A"/><w:b/><w:bCs/><w:color w:val="C00000"/><w:sz w:val="28"/></w:rPr><w:t>x</w:t></w:r></w:p>"#,
        );
    }

    #[test]
    fn a_paragraph_mark_revision_leads_and_properties_lead_their_element() {
        assert_eq!(
            rewrite(
                r#"<w:p><w:r><w:t>x</w:t><w:rPr><w:b/></w:rPr></w:r><w:pPr><w:jc w:val="center"/><w:rPr><w:sz w:val="20"/><w:ins w:id="1" w:author="a"/></w:rPr><w:pStyle w:val="H"/></w:pPr></w:p>"#
            ),
            r#"<w:p><w:pPr><w:pStyle w:val="H"/><w:jc w:val="center"/><w:rPr><w:ins w:id="1" w:author="a"/><w:sz w:val="20"/></w:rPr></w:pPr><w:r><w:rPr><w:b/></w:rPr><w:t>x</w:t></w:r></w:p>"#,
        );
    }

    #[test]
    fn unlisted_children_keep_their_place() {
        // An extension element stays right after the sibling it followed.
        assert_eq!(
            rewrite(
                r#"<w:p><w:r><w:rPr><w:lang w:val="ja-JP"/><w14:glow/><w:b/></w:rPr></w:r></w:p>"#
            ),
            r#"<w:p><w:r><w:rPr><w:b/><w:lang w:val="ja-JP"/><w14:glow/></w:rPr></w:r></w:p>"#,
        );
    }

    #[test]
    fn style_paragraph_properties_leave_a_stray_run_property_alone() {
        // CT_PPrGeneral has no `w:rPr`; moving one Word already rejects would
        // only change which error it reports.
        let xml = format!(
            r#"<w:numbering {W}><w:abstractNum w:abstractNumId="1"><w:lvl w:ilvl="0"><w:pPr><w:rPr/><w:ind w:left="420"/></w:pPr></w:lvl></w:abstractNum></w:numbering>"#
        );
        let written = DocxXmlDocument::parse(&xml)
            .and_then(|document| document.to_string())
            .expect("round trip");
        assert!(written.contains(r#"<w:pPr><w:rPr/><w:ind w:left="420"/></w:pPr>"#));
    }

    #[test]
    fn numbering_definitions_precede_instances() {
        let xml = format!(
            r#"<w:numbering {W}><w:num w:numId="1"><w:abstractNumId w:val="0"/></w:num><w:abstractNum w:abstractNumId="0"><w:lvl w:ilvl="0"><w:lvlText w:val="%1."/><w:suff w:val="space"/><w:start w:val="1"/></w:lvl></w:abstractNum></w:numbering>"#
        );
        let written = DocxXmlDocument::parse(&xml)
            .and_then(|document| document.to_string())
            .expect("round trip");
        assert!(written.contains(
            r#"<w:abstractNum w:abstractNumId="0"><w:lvl w:ilvl="0"><w:start w:val="1"/><w:suff w:val="space"/><w:lvlText w:val="%1."/></w:lvl></w:abstractNum><w:num w:numId="1">"#
        ));
    }

    #[test]
    fn undeclared_prefixes_are_declared_on_the_root() {
        let xml = format!(
            r#"<w:document {W} xmlns:mc="http://schemas.openxmlformats.org/markup-compatibility/2006" mc:Ignorable="w14"><w:body><w:p><w:ins w:id="1" w16du:dateUtc="2026-10-05T00:00:00Z"/></w:p><w:sectPr><w:headerReference r:id="rId1" w:type="default"/></w:sectPr></w:body></w:document>"#
        );
        let written = DocxXmlDocument::parse(&xml)
            .and_then(|document| document.to_string())
            .expect("round trip");
        let root = &written[..written.find("<w:body>").expect("body")];
        assert!(root.contains(
            r#"xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships""#
        ));
        assert!(root.contains(
            r#"xmlns:w16du="http://schemas.microsoft.com/office/word/2023/wordml/word16du""#
        ));
        assert!(root.contains(r#"mc:Ignorable="w14 w16du""#));
        assert_eq!(root.matches("mc:Ignorable").count(), 1);
    }

    #[test]
    fn a_prefix_declared_on_a_descendant_is_not_redeclared() {
        let xml = format!(
            r#"<w:document {W}><w:body><w:p><w:r><w:drawing><pic:pic xmlns:pic="http://schemas.openxmlformats.org/drawingml/2006/picture"><pic:nvPicPr/></pic:pic></w:drawing></w:r></w:p></w:body></w:document>"#
        );
        let written = DocxXmlDocument::parse(&xml)
            .and_then(|document| document.to_string())
            .expect("round trip");
        assert_eq!(written.matches("xmlns:pic=").count(), 1);
    }

    #[test]
    fn repeated_revision_ids_are_renumbered_past_the_largest_id() {
        let body = rewrite(
            r#"<w:p><w:ins w:id="4" w:author="a"><w:r><w:t>a</w:t></w:r></w:ins><w:bookmarkStart w:id="9" w:name="b"/><w:ins w:id="4" w:author="a"><w:r><w:t>b</w:t></w:r></w:ins><w:bookmarkEnd w:id="9"/></w:p>"#,
        );
        assert_eq!(
            body,
            r#"<w:p><w:ins w:id="4" w:author="a"><w:r><w:t>a</w:t></w:r></w:ins><w:bookmarkStart w:id="9" w:name="b"/><w:ins w:id="10" w:author="a"><w:r><w:t>b</w:t></w:r></w:ins><w:bookmarkEnd w:id="9"/></w:p>"#,
        );
    }
}
