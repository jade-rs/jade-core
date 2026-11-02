//! What kind of layout a layout definition (`dgm:layoutDef`) is, for the
//! layouts jade has no measurements of: its category and the algorithms
//! its layout nodes use pick the built-in layout jade draws it like.

use crate::xml;

/// The family a layout is drawn like.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Algorithm {
    /// Boxes in rows (`snake`).
    Snake,
    /// Boxes in a line with arrows between (`lin`).
    Linear,
    /// Round a circle (`cycle`).
    Cycle,
    /// A tree (`hierRoot`, `hierChild`).
    Hierarchy,
    /// Stacked levels (`pyra`).
    Pyramid,
    /// Overlapping circles.
    Venn,
    /// Four quadrants.
    Matrix,
    /// Pictures with captions.
    Picture,
}

impl Algorithm {
    /// Reads a layout definition: its first category, else the algorithms
    /// it uses.
    pub fn of_layout_def(xml: &str) -> Option<Self> {
        let root = xml::parse(xml).ok()?;
        let category = root
            .child("catLst")
            .and_then(|list| list.children_named("cat").next())
            .and_then(|cat| cat.attr("type"))
            .unwrap_or("");
        if let Some(found) = Self::of_category(category) {
            return Some(found);
        }
        let mut found = Vec::new();
        root.descendants("alg", &mut found);
        let algorithms: Vec<&str> = found.iter().filter_map(|alg| alg.attr("type")).collect();
        let has = |name: &str| algorithms.contains(&name);
        Some(if has("hierRoot") || has("hierChild") {
            Self::Hierarchy
        } else if has("pyra") {
            Self::Pyramid
        } else if has("cycle") {
            Self::Cycle
        } else if has("lin") {
            Self::Linear
        } else {
            Self::Snake
        })
    }

    fn of_category(category: &str) -> Option<Self> {
        Some(match category {
            "list" => Self::Snake,
            "process" => Self::Linear,
            "cycle" => Self::Cycle,
            "hierarchy" => Self::Hierarchy,
            "relationship" => Self::Venn,
            "matrix" => Self::Matrix,
            "pyramid" => Self::Pyramid,
            "picture" => Self::Picture,
            _ => return None,
        })
    }

    /// A guess from a layout's id alone (`…/layout/hProcess7` is a
    /// process), for a layout whose definition is not at hand.
    pub fn guess(id: &str) -> Self {
        let short = id
            .split('#')
            .next()
            .unwrap_or(id)
            .rsplit('/')
            .next()
            .unwrap_or(id)
            .to_ascii_lowercase();
        let has = |word: &str| short.contains(word);
        if has("picture") || has("plist") {
            Self::Picture
        } else if has("org") || has("hierarchy") {
            Self::Hierarchy
        } else if has("cycle") || has("radial") || has("gear") || has("circle") {
            Self::Cycle
        } else if has("pyramid") {
            Self::Pyramid
        } else if has("venn") || has("target") || has("balance") || has("equation") || has("funnel")
        {
            Self::Venn
        } else if has("matrix") {
            Self::Matrix
        } else if has("process") || has("arrow") || has("chevron") || has("step") {
            Self::Linear
        } else {
            Self::Snake
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_category_then_the_algorithms() {
        let with_category = r#"<dgm:layoutDef xmlns:dgm="d"><dgm:catLst><dgm:cat type="cycle" pri="1000"/></dgm:catLst><dgm:layoutNode><dgm:alg type="snake"/></dgm:layoutNode></dgm:layoutDef>"#;
        assert_eq!(
            Algorithm::of_layout_def(with_category),
            Some(Algorithm::Cycle)
        );
        let algorithms = r#"<dgm:layoutDef xmlns:dgm="d"><dgm:layoutNode><dgm:alg type="hierRoot"/></dgm:layoutNode></dgm:layoutDef>"#;
        assert_eq!(
            Algorithm::of_layout_def(algorithms),
            Some(Algorithm::Hierarchy)
        );
        assert_eq!(
            Algorithm::guess("urn:microsoft.com/office/officeart/2005/8/layout/hProcess7"),
            Algorithm::Linear
        );
    }
}
