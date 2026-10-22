//! 数式: the equation editor both shells run, over any [`WordSession`].
//!
//! Word edits an equation in place: the caret moves inside its structure —
//! into a fraction's numerator, down to its denominator, out past it — typed
//! text goes into the argument the caret is in, and the Equation tab inserts
//! structures (分数, 上付き/下付き文字, べき乗根 …) and symbols at the caret.
//! Typing linear format builds up as it goes (`a/b` and a space becomes a
//! fraction), Math AutoCorrect turns `\alpha` into α, and the whole equation
//! switches between its 2D and linear forms in UnicodeMath or LaTeX.
//!
//! The editor holds the equation's OMML tree and a caret in it: an argument
//! (by its path in the tree) and a position among the argument's atoms — each
//! character of its text, and each structure as one. Every edit rewrites the
//! equation through `ReplaceEquation`, and the renderer's record of where the
//! equation was drawn ([`MathZone`]) places the caret and answers clicks.

use crate::canvas_renderer::MathZone;
pub use crate::document::MathElement;
use crate::document::MathElementKind as K;
use jade_docx_headless_api::{HeadlessCommand, HeadlessEquationPlacement};
pub use jade_docx_layout::math_layout::linear::LinearFormat;
use jade_docx_layout::math_layout::linear::{
    apply_math_autocorrect, linear_to_omath, math_run, parse_linear, to_linear,
};
use jade_docx_layout::math_layout::styled::MathBox;

use super::WordSession;

/// The text Word shows in an empty equation.
pub const EQUATION_PLACEHOLDER_KEY: &str = "equation.placeholder";

/// Where an equation is: what `ReplaceEquation` needs to rewrite it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EquationTarget {
    /// The paragraph it is drawn in (a display equation's own).
    pub paragraph: usize,
    /// Its math run, for an inline equation.
    pub run_index: Option<usize>,
    pub display: bool,
}

/// The caret inside an equation: the argument it is in, by path from the
/// equation's root, and the atom it stands before.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MathCaret {
    pub slot: Vec<u16>,
    pub index: usize,
}

/// One atom of an argument: a character of a run, or a structure.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Atom {
    /// The `char`-th character of the text of child `child`.
    Char {
        child: usize,
        text: usize,
        char: usize,
    },
    Structure {
        child: usize,
    },
}

/// Where the caret is drawn, relative to the page it is on.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CaretGeometry {
    /// 1-based page.
    pub page: usize,
    pub x: f64,
    pub top: f64,
    pub height: f64,
}

/// A rectangle on a page: an empty argument's placeholder, the equation's
/// frame.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PageRect {
    pub page: usize,
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

/// The equation being edited.
#[derive(Clone, Debug)]
pub struct EquationEditor {
    pub target: EquationTarget,
    /// The equation's root: an `m:oMath`, or a display equation's
    /// `m:oMathPara`.
    root: MathElement,
    pub caret: MathCaret,
    /// A structure Backspace has selected; the next Backspace deletes it, as
    /// Word asks before taking a whole structure away.
    pub selected: Option<Vec<u16>>,
    /// The linear format typing builds up from and the linear form shows.
    pub format: LinearFormat,
    /// 標準テキスト: typed text goes in as normal (non-math) text.
    pub normal_text: bool,
}

/// What the Equation tab inserts.
#[derive(Clone, Debug, PartialEq)]
pub enum EquationInsert {
    /// A built-in equation from the 数式 gallery.
    BuiltIn(usize),
    /// A structure from one of the 構造 lists, by group and item.
    Structure(StructureGroup, usize),
    /// A symbol from the 記号と特殊文字 gallery.
    Symbol(char),
}

impl EquationEditor {
    /// An editor over `root` (an `m:oMath`, or an `m:oMathPara`), its caret
    /// at the end.
    pub fn new(target: EquationTarget, root: MathElement) -> Self {
        let root = if target.display && root.kind == K::Math {
            MathElement::new(K::MathParagraph, vec![root])
        } else {
            root
        };
        let mut editor = Self {
            target,
            root,
            caret: MathCaret {
                slot: Vec::new(),
                index: 0,
            },
            selected: None,
            format: LinearFormat::UnicodeMath,
            normal_text: false,
        };
        editor.caret = editor.end_of(editor.top_slot());
        editor
    }

    /// Starts editing the equation drawn in `zone`, with the caret at the end.
    pub fn open(session: &dyn WordSession, zone: &MathZone) -> Option<Self> {
        let target = EquationTarget {
            paragraph: zone.paragraph,
            run_index: zone.run_index,
            display: zone.display,
        };
        let root = equation_root(session, &target)?;
        Some(Self::new(target, root))
    }

    /// The equation's root element.
    pub fn root(&self) -> &MathElement {
        &self.root
    }

    /// The equation as OMML.
    pub fn omml(&self) -> String {
        self.root.to_omml()
    }

    /// Whether the equation holds nothing yet: Word shows its placeholder.
    pub fn is_empty(&self) -> bool {
        !has_content(&self.root)
    }

    /// The argument the caret starts in: the `m:oMath` (in a display
    /// equation, the first one of its `m:oMathPara`).
    fn top_slot(&self) -> Vec<u16> {
        if self.root.kind == K::MathParagraph {
            let index = self
                .root
                .children
                .iter()
                .position(|c| c.kind == K::Math)
                .unwrap_or(0);
            vec![0, index as u16]
        } else {
            vec![0]
        }
    }

    fn element(&self, path: &[u16]) -> Option<&MathElement> {
        let (first, rest) = path.split_first()?;
        if *first != 0 {
            return None;
        }
        let mut e = &self.root;
        for &i in rest {
            e = e.children.get(usize::from(i))?;
        }
        Some(e)
    }

    fn element_mut(&mut self, path: &[u16]) -> Option<&mut MathElement> {
        let (first, rest) = path.split_first()?;
        if *first != 0 {
            return None;
        }
        let mut e = &mut self.root;
        for &i in rest {
            e = e.children.get_mut(usize::from(i))?;
        }
        Some(e)
    }

    fn atoms(&self, slot: &[u16]) -> Vec<Atom> {
        self.element(slot).map(atoms_of).unwrap_or_default()
    }

    fn end_of(&self, slot: Vec<u16>) -> MathCaret {
        let index = self.atoms(&slot).len();
        MathCaret { slot, index }
    }

    // ---- typing ------------------------------------------------------------

    /// Types `text` at the caret. A space after linear format builds the
    /// argument up; a control word becomes its symbol.
    pub fn type_text(&mut self, text: &str) {
        self.selected = None;
        if text == " " && !self.normal_text && self.build_up() {
            return;
        }
        self.insert_text(text);
    }

    fn insert_text(&mut self, text: &str) {
        let slot = self.caret.slot.clone();
        let index = self.caret.index;
        let atoms = self.atoms(&slot);
        let normal = self.normal_text;
        let Some(element) = self.element_mut(&slot) else {
            return;
        };
        // Into the run the caret is in or beside, when it is a run of the
        // same kind (math or normal text).
        let fits = |run: &MathElement| run.kind == K::Run && is_normal_run(run) == normal;
        let target = if index > 0 {
            match atoms.get(index - 1) {
                Some(Atom::Char { child, text, char }) if fits(&element.children[*child]) => {
                    Some((*child, *text, char + 1))
                }
                _ => None,
            }
        } else {
            None
        }
        .or_else(|| match atoms.get(index) {
            Some(Atom::Char { child, text, char }) if fits(&element.children[*child]) => {
                Some((*child, *text, *char))
            }
            _ => None,
        });
        match target {
            Some((child, text_index, at)) => {
                let t = &mut element.children[child].children[text_index];
                let current = t.text.get_or_insert_with(String::new);
                let byte = current
                    .char_indices()
                    .nth(at)
                    .map_or(current.len(), |(b, _)| b);
                current.insert_str(byte, text);
            }
            None => {
                let position = child_position(element, &atoms, index);
                let run = if normal {
                    normal_run(text)
                } else {
                    math_run(text, Vec::new())
                };
                element.children.insert(position, run);
            }
        }
        self.caret.index += text.chars().count();
    }

    /// Builds up the linear format in the caret's argument, as Word does
    /// when a space follows it. Returns whether anything changed.
    pub fn build_up(&mut self) -> bool {
        let slot = self.caret.slot.clone();
        let Some(element) = self.element(&slot) else {
            return false;
        };
        // Only text builds up; structures already are.
        if element
            .children
            .iter()
            .any(|c| is_content(c) && c.kind != K::Run)
        {
            return false;
        }
        let text: String = element
            .children
            .iter()
            .filter(|c| c.kind == K::Run && !is_normal_run(c))
            .map(run_text)
            .collect();
        let corrected = apply_math_autocorrect(&text);
        let builds = corrected != text
            || corrected.chars().any(|c| {
                matches!(
                    c,
                    '/' | '^'
                        | '_'
                        | '√'
                        | '∛'
                        | '∜'
                        | '▒'
                        | '■'
                        | '█'
                        | '¦'
                        | '∕'
                        | '⁄'
                        | '┬'
                        | '┴'
                        | '□'
                        | '▭'
                        | '¯'
                        | '▁'
                        | '⏞'
                        | '⏟'
                ) || jade_docx_layout::math_layout::linear::is_nary(c)
            });
        if !builds || text.is_empty() {
            return false;
        }
        let elements = parse_linear(&corrected, self.format);
        let Some(element) = self.element_mut(&slot) else {
            return false;
        };
        element.children.retain(|c| !is_content(c));
        element.children.extend(elements);
        self.caret = self.end_of(slot);
        true
    }

    /// Inserts a structure template (empty arguments) at the caret and puts
    /// the caret in its first empty argument.
    pub fn insert_structure(&mut self, template: MathElement) {
        self.selected = None;
        let slot = self.caret.slot.clone();
        let index = self.caret.index;
        let atoms = self.atoms(&slot);
        let Some(element) = self.element_mut(&slot) else {
            return;
        };
        let position = split_at(element, &atoms, index);
        element.children.insert(position, template);
        let mut structure = slot.clone();
        structure.push(position as u16);
        match self.first_empty_slot(&structure) {
            Some(path) => {
                self.caret = MathCaret {
                    slot: path,
                    index: 0,
                }
            }
            None => {
                let index = self
                    .atoms(&slot)
                    .iter()
                    .position(|a| matches!(a, Atom::Structure { child } if *child == position))
                    .map_or(0, |i| i + 1);
                self.caret = MathCaret { slot, index };
            }
        }
    }

    /// Inserts a whole equation (a built-in) at the caret: its content.
    pub fn insert_equation(&mut self, equation: &MathElement) {
        let content: Vec<MathElement> = match equation.kind {
            K::MathParagraph => equation
                .children
                .iter()
                .filter(|c| c.kind == K::Math)
                .flat_map(|m| m.children.clone())
                .collect(),
            K::Math => equation.children.clone(),
            _ => vec![equation.clone()],
        };
        let slot = self.caret.slot.clone();
        let index = self.caret.index;
        let atoms = self.atoms(&slot);
        let Some(element) = self.element_mut(&slot) else {
            return;
        };
        let position = split_at(element, &atoms, index);
        let count = content.len();
        for (offset, child) in content.into_iter().enumerate() {
            element.children.insert(position + offset, child);
        }
        let after = position + count;
        let index = atoms_of(element)
            .iter()
            .position(|a| child_of(a) >= after)
            .unwrap_or_else(|| atoms_of(element).len());
        self.caret = MathCaret { slot, index };
    }

    /// Inserts what the Equation tab offers.
    pub fn insert(&mut self, item: &EquationInsert) {
        match item {
            EquationInsert::BuiltIn(index) => {
                if let Some(equation) = built_in_equation(*index) {
                    self.insert_equation(&equation);
                }
            }
            EquationInsert::Structure(group, index) => {
                if let Some(template) = structure_template(*group, *index) {
                    self.insert_structure(template);
                }
            }
            EquationInsert::Symbol(ch) => self.type_text(&ch.to_string()),
        }
    }

    fn first_empty_slot(&self, path: &[u16]) -> Option<Vec<u16>> {
        let element = self.element(path)?;
        for (i, c) in element.children.iter().enumerate() {
            let mut child_path = path.to_vec();
            child_path.push(i as u16);
            if is_slot(c) && !has_content(c) && !hidden_slot(element, c) {
                return Some(child_path);
            }
            if let Some(found) = self.first_empty_slot(&child_path) {
                return Some(found);
            }
        }
        None
    }

    // ---- deleting ------------------------------------------------------------

    /// Backspace: the character before the caret, or (asked twice) the
    /// structure before it; at the start of an argument, out of it, or the
    /// structure away when it is empty.
    pub fn backspace(&mut self) {
        let slot = self.caret.slot.clone();
        let index = self.caret.index;
        let atoms = self.atoms(&slot);
        if index == 0 {
            self.leave_backwards(true);
            return;
        }
        match atoms[index - 1] {
            Atom::Char { child, text, char } => {
                self.selected = None;
                self.remove_char(&slot, child, text, char);
                self.caret.index -= 1;
            }
            Atom::Structure { child } => {
                let mut path = slot.clone();
                path.push(child as u16);
                if self.selected.as_ref() == Some(&path) {
                    self.selected = None;
                    if let Some(element) = self.element_mut(&slot) {
                        element.children.remove(child);
                    }
                    self.caret.index -= 1;
                } else {
                    self.selected = Some(path);
                }
            }
        }
    }

    /// Delete: the character or structure after the caret.
    pub fn delete_forward(&mut self) {
        let slot = self.caret.slot.clone();
        let index = self.caret.index;
        let atoms = self.atoms(&slot);
        match atoms.get(index).copied() {
            Some(Atom::Char { child, text, char }) => {
                self.selected = None;
                self.remove_char(&slot, child, text, char);
            }
            Some(Atom::Structure { child }) => {
                let mut path = slot.clone();
                path.push(child as u16);
                if self.selected.as_ref() == Some(&path) {
                    self.selected = None;
                    if let Some(element) = self.element_mut(&slot) {
                        element.children.remove(child);
                    }
                } else {
                    self.selected = Some(path);
                }
            }
            None => {}
        }
    }

    fn remove_char(&mut self, slot: &[u16], child: usize, text: usize, char: usize) {
        let Some(element) = self.element_mut(slot) else {
            return;
        };
        let t = &mut element.children[child].children[text];
        if let Some(value) = t.text.as_mut() {
            if let Some((byte, ch)) = value.char_indices().nth(char) {
                value.replace_range(byte..byte + ch.len_utf8(), "");
            }
        }
        if run_text(&element.children[child]).is_empty() {
            element.children.remove(child);
        }
    }

    /// Leaves the caret's argument backwards: to the previous argument of
    /// its structure, or before the structure. With `deleting`, an empty
    /// structure is removed instead.
    fn leave_backwards(&mut self, deleting: bool) {
        let slot = self.caret.slot.clone();
        if slot.len() <= self.top_slot().len() {
            return;
        }
        let structure_path = slot[..slot.len() - 1].to_vec();
        if deleting
            && self
                .element(&structure_path)
                .is_some_and(|s| !has_content(s))
        {
            let parent = structure_path[..structure_path.len() - 1].to_vec();
            let child = usize::from(*structure_path.last().unwrap());
            if let Some(parent_element) = self.element_mut(&parent) {
                let atoms = atoms_of(parent_element);
                let index = atoms
                    .iter()
                    .position(|a| child_of(a) >= child)
                    .unwrap_or(atoms.len());
                parent_element.children.remove(child);
                self.caret = MathCaret {
                    slot: parent,
                    index,
                };
            }
            return;
        }
        if let Some(previous) = self.sibling_slot(&slot, false) {
            self.caret = self.end_of(previous);
        } else {
            self.caret = self.before_structure(&structure_path);
        }
    }

    // ---- moving --------------------------------------------------------------

    pub fn move_right(&mut self) {
        self.selected = None;
        let slot = self.caret.slot.clone();
        let atoms = self.atoms(&slot);
        match atoms.get(self.caret.index).copied() {
            Some(Atom::Char { .. }) => self.caret.index += 1,
            Some(Atom::Structure { child }) => {
                let mut path = slot;
                path.push(child as u16);
                match self.first_slot(&path, true) {
                    Some(inner) => {
                        self.caret = MathCaret {
                            slot: inner,
                            index: 0,
                        }
                    }
                    None => self.caret.index += 1,
                }
            }
            None => {
                if slot.len() <= self.top_slot().len() {
                    return;
                }
                if let Some(next) = self.sibling_slot(&slot, true) {
                    self.caret = MathCaret {
                        slot: next,
                        index: 0,
                    };
                } else {
                    let structure = slot[..slot.len() - 1].to_vec();
                    self.caret = self.after_structure(&structure);
                }
            }
        }
    }

    pub fn move_left(&mut self) {
        self.selected = None;
        let slot = self.caret.slot.clone();
        let atoms = self.atoms(&slot);
        if self.caret.index == 0 {
            self.leave_backwards(false);
            return;
        }
        match atoms[self.caret.index - 1] {
            Atom::Char { .. } => self.caret.index -= 1,
            Atom::Structure { child } => {
                let mut path = slot;
                path.push(child as u16);
                match self.first_slot(&path, false) {
                    Some(inner) => self.caret = self.end_of(inner),
                    None => self.caret.index -= 1,
                }
            }
        }
    }

    /// Moves to the argument above or below inside the caret's structures
    /// (numerator ↔ denominator, superscript ↔ subscript, matrix rows),
    /// using where they were drawn.
    pub fn move_vertically(&mut self, zone: &MathZone, down: bool) {
        self.selected = None;
        let Some(caret) = self.caret_in_plan(zone) else {
            return;
        };
        let current = self.caret.slot.clone();
        // The slots of the nearest enclosing structure that has one in that
        // direction.
        let mut ancestor = current.clone();
        while ancestor.len() > self.top_slot().len() {
            ancestor.pop();
            let candidates: Vec<&MathBox> = zone
                .plan
                .boxes
                .iter()
                .filter(|b| {
                    b.path.len() > ancestor.len()
                        && b.path.starts_with(&ancestor)
                        && !current.starts_with(&b.path)
                        && is_slot_kind(&b.kind)
                        && if down {
                            b.baseline - b.ascent > caret.1
                        } else {
                            b.baseline + b.descent < caret.0
                        }
                })
                .collect();
            if let Some(best) = candidates.into_iter().min_by(|a, b| {
                let da = vertical_distance(a, caret, down) + horizontal_distance(a, caret.2);
                let db = vertical_distance(b, caret, down) + horizontal_distance(b, caret.2);
                da.total_cmp(&db)
            }) {
                let path = best.path.clone();
                self.caret = self.nearest_in_slot(zone, path, caret.2);
                return;
            }
        }
    }

    pub fn move_home(&mut self) {
        self.selected = None;
        self.caret.index = 0;
    }

    pub fn move_end(&mut self) {
        self.selected = None;
        self.caret = self.end_of(self.caret.slot.clone());
    }

    /// Tab: the next argument of the equation, in order.
    pub fn next_slot(&mut self) {
        self.selected = None;
        let slots = self.all_slots();
        let position = slots.iter().position(|s| *s == self.caret.slot);
        if let Some(next) = position.and_then(|p| slots.get(p + 1)) {
            self.caret = MathCaret {
                slot: next.clone(),
                index: 0,
            };
        }
    }

    fn all_slots(&self) -> Vec<Vec<u16>> {
        fn walk(
            e: &MathElement,
            path: &mut Vec<u16>,
            out: &mut Vec<Vec<u16>>,
            parent: Option<&MathElement>,
        ) {
            if is_slot(e) && !parent.is_some_and(|p| hidden_slot(p, e)) {
                out.push(path.clone());
            }
            for (i, c) in e.children.iter().enumerate() {
                path.push(i as u16);
                walk(c, path, out, Some(e));
                path.pop();
            }
        }
        let mut out = Vec::new();
        walk(&self.root, &mut vec![0], &mut out, None);
        out
    }

    /// The first (or last) visible argument of the structure at `path`.
    fn first_slot(&self, path: &[u16], first: bool) -> Option<Vec<u16>> {
        let element = self.element(path)?;
        let mut slots: Vec<Vec<u16>> = Vec::new();
        collect_slots(element, path.to_vec(), &mut slots);
        if first {
            slots.into_iter().next()
        } else {
            slots.into_iter().last()
        }
    }

    /// The next (or previous) argument of the structure `slot` belongs to.
    fn sibling_slot(&self, slot: &[u16], forwards: bool) -> Option<Vec<u16>> {
        // Up to the structure that owns the argument (a matrix row is not
        // one; its matrix is).
        let mut owner = slot[..slot.len() - 1].to_vec();
        if self.element(&owner).is_some_and(|e| e.kind == K::MatrixRow) {
            owner.pop();
        }
        let element = self.element(&owner)?;
        let mut slots = Vec::new();
        collect_slots(element, owner.clone(), &mut slots);
        let position = slots.iter().position(|s| s.as_slice() == slot)?;
        if forwards {
            slots.get(position + 1).cloned()
        } else {
            position.checked_sub(1).and_then(|p| slots.get(p).cloned())
        }
    }

    fn before_structure(&self, structure: &[u16]) -> MathCaret {
        let mut parent = structure[..structure.len() - 1].to_vec();
        let child = usize::from(*structure.last().unwrap());
        if self
            .element(&parent)
            .is_some_and(|e| e.kind == K::MatrixRow)
        {
            // A cell's structure is the matrix the row belongs to.
            parent.pop();
            return self.before_structure(&parent);
        }
        let index = self
            .atoms(&parent)
            .iter()
            .position(|a| child_of(a) >= child)
            .unwrap_or(0);
        MathCaret {
            slot: parent,
            index,
        }
    }

    fn after_structure(&self, structure: &[u16]) -> MathCaret {
        let mut caret = self.before_structure(&owner_structure(self, structure));
        caret.index += 1;
        caret
    }

    // ---- geometry ------------------------------------------------------------

    /// The caret's (top, bottom, x) in the plan's coordinates.
    fn caret_in_plan(&self, zone: &MathZone) -> Option<(f64, f64, f64)> {
        let plan = &zone.plan;
        let slot_box = plan.boxes.iter().find(|b| b.path == self.caret.slot)?;
        let atoms = self.atoms(&self.caret.slot);
        let size = slot_box.size_px.max(1.0);
        let (top, bottom) = (
            slot_box.baseline - slot_box.ascent.max(size * 0.75),
            slot_box.baseline + slot_box.descent.max(size * 0.25),
        );
        let x = if let Some(atom) = atoms.get(self.caret.index) {
            self.atom_left(zone, atom)?
        } else if let Some(atom) = self.caret.index.checked_sub(1).and_then(|i| atoms.get(i)) {
            self.atom_right(zone, atom)?
        } else {
            slot_box.x + if slot_box.empty { 0.0 } else { slot_box.width }
        };
        Some((top, bottom, x))
    }

    fn atom_box<'z>(&self, zone: &'z MathZone, atom: &Atom) -> Option<&'z MathBox> {
        let mut path = self.caret.slot.clone();
        match *atom {
            Atom::Char { child, text, .. } => {
                path.push(child as u16);
                path.push(text as u16);
            }
            Atom::Structure { child } => path.push(child as u16),
        }
        zone.plan.boxes.iter().find(|b| b.path == path)
    }

    fn atom_left(&self, zone: &MathZone, atom: &Atom) -> Option<f64> {
        let b = self.atom_box(zone, atom)?;
        match *atom {
            Atom::Char { char, .. } => {
                let glyph = zone.plan.glyphs.get(b.glyphs.start + char)?;
                Some(glyph.x)
            }
            Atom::Structure { .. } => Some(b.x),
        }
    }

    fn atom_right(&self, zone: &MathZone, atom: &Atom) -> Option<f64> {
        let b = self.atom_box(zone, atom)?;
        match *atom {
            Atom::Char { char, .. } => {
                let glyph = zone.plan.glyphs.get(b.glyphs.start + char)?;
                Some(glyph.x + glyph.width)
            }
            Atom::Structure { .. } => Some(b.x + b.width),
        }
    }

    /// Where the caret is drawn on its page.
    pub fn caret_geometry(&self, zone: &MathZone) -> Option<CaretGeometry> {
        let (top, bottom, x) = self.caret_in_plan(zone)?;
        Some(CaretGeometry {
            page: zone.page,
            x: zone.x + x,
            top: zone.baseline + top,
            height: (bottom - top).max(1.0),
        })
    }

    /// The equation's frame (Word draws one round the equation being
    /// edited) and its empty arguments' placeholders, on the page.
    pub fn frames(&self, zone: &MathZone) -> (PageRect, Vec<PageRect>) {
        let plan = &zone.plan;
        let size = plan.boxes.first().map_or(14.0, |b| b.size_px).max(1.0);
        let ascent = plan.metrics.ascent.max(size * 0.9);
        let descent = plan.metrics.descent.max(size * 0.3);
        let width = plan.metrics.width.max(size * 0.6);
        let frame = PageRect {
            page: zone.page,
            x: zone.x - 2.0,
            y: zone.baseline - ascent - 2.0,
            width: width + 4.0,
            height: ascent + descent + 4.0,
        };
        let placeholders = plan
            .boxes
            .iter()
            .filter(|b| b.empty)
            .map(|b| PageRect {
                page: zone.page,
                x: zone.x + b.x,
                y: zone.baseline + b.baseline - b.ascent,
                width: b.width,
                height: b.ascent + b.descent.max(b.size_px * 0.15),
            })
            .collect();
        (frame, placeholders)
    }

    /// Puts the caret where a click at (`x`, `y`) on the page lands.
    pub fn click(&mut self, zone: &MathZone, x: f64, y: f64) {
        self.selected = None;
        let px = x - zone.x;
        let py = y - zone.baseline;
        // The innermost argument under the point.
        let slot = zone
            .plan
            .boxes
            .iter()
            .filter(|b| is_slot_kind(&b.kind) && self.element(&b.path).is_some())
            .filter(|b| {
                px >= b.x - 1.0
                    && px <= b.x + b.width + 1.0
                    && py >= b.baseline - b.ascent - 1.0
                    && py <= b.baseline + b.descent + 1.0
            })
            .max_by_key(|b| b.path.len())
            .map(|b| b.path.clone())
            .unwrap_or_else(|| self.top_slot());
        self.caret = self.nearest_in_slot(zone, slot, px);
    }

    fn nearest_in_slot(&self, zone: &MathZone, slot: Vec<u16>, x: f64) -> MathCaret {
        let probe = Self {
            caret: MathCaret {
                slot: slot.clone(),
                index: 0,
            },
            ..self.clone()
        };
        let atoms = self.atoms(&slot);
        let mut best = (f64::INFINITY, 0usize);
        for index in 0..=atoms.len() {
            let position = if let Some(atom) = atoms.get(index) {
                probe.atom_left(zone, atom)
            } else if index > 0 {
                probe.atom_right(zone, &atoms[index - 1])
            } else {
                zone.plan.boxes.iter().find(|b| b.path == slot).map(|b| b.x)
            };
            if let Some(position) = position {
                let distance = (position - x).abs();
                if distance < best.0 {
                    best = (distance, index);
                }
            }
        }
        MathCaret {
            slot,
            index: best.1,
        }
    }

    // ---- matrices, equation arrays, line breaks and formatting ---------------

    /// The matrix or equation array the caret is in, if any.
    pub fn table_context(&self) -> Option<MathTableContext> {
        let slot = &self.caret.slot;
        if slot.len() < 2 {
            return None;
        }
        let parent_path = &slot[..slot.len() - 1];
        let parent = self.element(parent_path)?;
        let child = usize::from(*slot.last()?);
        match parent.kind {
            K::MatrixRow => {
                let matrix_path = &parent_path[..parent_path.len() - 1];
                let matrix = self.element(matrix_path)?;
                let row_child = usize::from(*parent_path.last()?);
                let rows: Vec<usize> = matrix
                    .children
                    .iter()
                    .enumerate()
                    .filter(|(_, c)| c.kind == K::MatrixRow)
                    .map(|(i, _)| i)
                    .collect();
                let row = rows.iter().position(|&i| i == row_child)?;
                let cells: Vec<usize> = parent
                    .children
                    .iter()
                    .enumerate()
                    .filter(|(_, c)| c.kind == K::Base)
                    .map(|(i, _)| i)
                    .collect();
                let column = cells.iter().position(|&i| i == child)?;
                let columns = matrix
                    .children
                    .iter()
                    .filter(|c| c.kind == K::MatrixRow)
                    .map(|r| r.children.iter().filter(|c| c.kind == K::Base).count())
                    .max()
                    .unwrap_or(0);
                let justification = column_justifications(matrix, columns)
                    .get(column)
                    .cloned()
                    .unwrap_or_else(|| "center".into());
                Some(MathTableContext {
                    matrix: true,
                    path: matrix_path.to_vec(),
                    row,
                    column,
                    rows: rows.len(),
                    columns,
                    justification,
                })
            }
            K::EquationArray => {
                let rows: Vec<usize> = parent
                    .children
                    .iter()
                    .enumerate()
                    .filter(|(_, c)| c.kind == K::Base)
                    .map(|(i, _)| i)
                    .collect();
                let row = rows.iter().position(|&i| i == child)?;
                Some(MathTableContext {
                    matrix: false,
                    path: parent_path.to_vec(),
                    row,
                    column: 0,
                    rows: rows.len(),
                    columns: 1,
                    justification: "center".into(),
                })
            }
            _ => None,
        }
    }

    /// 行を上に挿入 / 行を下に挿入: a row of empty cells in the caret's
    /// matrix or equation array, the caret in its first cell.
    pub fn insert_row(&mut self, below: bool) -> bool {
        let Some(context) = self.table_context() else {
            return false;
        };
        let Some(table) = self.element_mut(&context.path) else {
            return false;
        };
        let row_kind = if context.matrix {
            K::MatrixRow
        } else {
            K::Base
        };
        let positions: Vec<usize> = table
            .children
            .iter()
            .enumerate()
            .filter(|(_, c)| c.kind == row_kind)
            .map(|(i, _)| i)
            .collect();
        let at = positions[context.row] + usize::from(below);
        let row = if context.matrix {
            MathElement::new(
                K::MatrixRow,
                (0..context.columns)
                    .map(|_| MathElement::new(K::Base, Vec::new()))
                    .collect(),
            )
        } else {
            MathElement::new(K::Base, Vec::new())
        };
        table.children.insert(at, row);
        let mut slot = context.path.clone();
        slot.push(at as u16);
        if context.matrix {
            slot.push(0);
        }
        self.caret = MathCaret { slot, index: 0 };
        true
    }

    /// 列を左に挿入 / 列を右に挿入: a column of empty cells in the caret's
    /// matrix, centred.
    pub fn insert_column(&mut self, after: bool) -> bool {
        let Some(context) = self.table_context().filter(|c| c.matrix) else {
            return false;
        };
        let column = context.column + usize::from(after);
        let Some(matrix) = self.element_mut(&context.path) else {
            return false;
        };
        let mut justifications = column_justifications(matrix, context.columns);
        justifications.insert(column.min(justifications.len()), "center".into());
        for row in matrix
            .children
            .iter_mut()
            .filter(|c| c.kind == K::MatrixRow)
        {
            let cells: Vec<usize> = row
                .children
                .iter()
                .enumerate()
                .filter(|(_, c)| c.kind == K::Base)
                .map(|(i, _)| i)
                .collect();
            let at = cells
                .get(column)
                .copied()
                .unwrap_or_else(|| cells.last().map_or(row.children.len(), |i| i + 1));
            row.children
                .insert(at, MathElement::new(K::Base, Vec::new()));
        }
        set_column_justifications(matrix, &justifications);
        let mut slot = context.path.clone();
        let row_child = matrix
            .children
            .iter()
            .enumerate()
            .filter(|(_, c)| c.kind == K::MatrixRow)
            .nth(context.row)
            .map(|(i, _)| i)
            .unwrap_or(0);
        let cell_child = matrix.children[row_child]
            .children
            .iter()
            .enumerate()
            .filter(|(_, c)| c.kind == K::Base)
            .nth(column)
            .map(|(i, _)| i)
            .unwrap_or(0);
        slot.push(row_child as u16);
        slot.push(cell_child as u16);
        self.caret = MathCaret { slot, index: 0 };
        true
    }

    /// 行の削除: the caret's row, unless it is the only one.
    pub fn delete_row(&mut self) -> bool {
        let Some(context) = self.table_context().filter(|c| c.rows > 1) else {
            return false;
        };
        let row_kind = if context.matrix {
            K::MatrixRow
        } else {
            K::Base
        };
        let Some(table) = self.element_mut(&context.path) else {
            return false;
        };
        let positions: Vec<usize> = table
            .children
            .iter()
            .enumerate()
            .filter(|(_, c)| c.kind == row_kind)
            .map(|(i, _)| i)
            .collect();
        table.children.remove(positions[context.row]);
        let next = positions[context.row.min(positions.len() - 2)];
        let mut slot = context.path.clone();
        slot.push(next as u16);
        if context.matrix {
            let first = self
                .element(&slot)
                .and_then(|row| row.children.iter().position(|c| c.kind == K::Base))
                .unwrap_or(0);
            slot.push(first as u16);
        }
        self.caret = MathCaret { slot, index: 0 };
        true
    }

    /// 列の削除: the caret's column, unless it is the only one.
    pub fn delete_column(&mut self) -> bool {
        let Some(context) = self.table_context().filter(|c| c.matrix && c.columns > 1) else {
            return false;
        };
        let Some(matrix) = self.element_mut(&context.path) else {
            return false;
        };
        let mut justifications = column_justifications(matrix, context.columns);
        if context.column < justifications.len() {
            justifications.remove(context.column);
        }
        for row in matrix
            .children
            .iter_mut()
            .filter(|c| c.kind == K::MatrixRow)
        {
            if let Some(at) = row
                .children
                .iter()
                .enumerate()
                .filter(|(_, c)| c.kind == K::Base)
                .nth(context.column)
                .map(|(i, _)| i)
            {
                row.children.remove(at);
            }
        }
        set_column_justifications(matrix, &justifications);
        let mut slot = context.path.clone();
        let row_child = matrix
            .children
            .iter()
            .enumerate()
            .filter(|(_, c)| c.kind == K::MatrixRow)
            .nth(context.row)
            .map(|(i, _)| i)
            .unwrap_or(0);
        let cell_child = matrix.children[row_child]
            .children
            .iter()
            .enumerate()
            .filter(|(_, c)| c.kind == K::Base)
            .nth(context.column.min(context.columns - 2))
            .map(|(i, _)| i)
            .unwrap_or(0);
        slot.push(row_child as u16);
        slot.push(cell_child as u16);
        self.caret = MathCaret { slot, index: 0 };
        true
    }

    /// 列の配置: the caret's matrix column left, centred or right.
    pub fn set_column_justification(&mut self, justification: &str) -> bool {
        let Some(context) = self.table_context().filter(|c| c.matrix) else {
            return false;
        };
        let Some(matrix) = self.element_mut(&context.path) else {
            return false;
        };
        let mut justifications = column_justifications(matrix, context.columns);
        if let Some(slot) = justifications.get_mut(context.column) {
            *slot = justification.to_string();
        }
        set_column_justifications(matrix, &justifications);
        true
    }

    /// 手動改行を挿入: a manual line break before the caret, at the top
    /// level of a display equation (`m:brk` on the run that starts there).
    pub fn insert_line_break(&mut self) -> bool {
        self.mark_run(|pr| {
            if !pr.children.iter().any(|c| c.local_name() == "brk") {
                let mut brk = MathElement::tagged("brk", Vec::new());
                brk.kind = K::Unknown;
                pr.children.push(brk);
            }
        })
    }

    /// この文字で揃える: an alignment point (`m:aln`) before the caret.
    pub fn set_alignment_point(&mut self) -> bool {
        self.mark_run(|pr| {
            if !pr.children.iter().any(|c| c.local_name() == "aln") {
                let mut aln = MathElement::tagged("aln", Vec::new());
                aln.kind = K::Unknown;
                pr.children.push(aln);
            }
        })
    }

    /// Splits the caret's run so a run starts at the caret, and sets a math
    /// run property on it.
    fn mark_run(&mut self, set: impl FnOnce(&mut MathElement)) -> bool {
        self.selected = None;
        let slot = self.caret.slot.clone();
        if slot != self.top_slot() {
            return false;
        }
        let index = self.caret.index;
        let atoms = self.atoms(&slot);
        let Some(element) = self.element_mut(&slot) else {
            return false;
        };
        let position = split_at(element, &atoms, index);
        if element
            .children
            .get(position)
            .is_none_or(|c| c.kind != K::Run)
        {
            element.children.insert(position, math_run("", Vec::new()));
        }
        let run = &mut element.children[position];
        let pr_index = match run
            .children
            .iter()
            .position(|c| c.local_name() == "rPr" && c.namespace.is_none())
        {
            Some(i) => i,
            None => {
                let mut pr = MathElement::tagged("rPr", Vec::new());
                pr.kind = K::Unknown;
                run.children.insert(0, pr);
                0
            }
        };
        set(&mut run.children[pr_index]);
        true
    }

    /// Formats the equation's text, as Word applies ホーム's font size,
    /// colour and bold to an equation: every run of it.
    pub fn format_runs(&mut self, format: EquationFormat) {
        fn walk(e: &mut MathElement, format: &EquationFormat) {
            if e.kind == K::Run {
                format_run(e, format);
            }
            for c in &mut e.children {
                walk(c, format);
            }
        }
        walk(&mut self.root, &format);
    }

    // ---- linear and professional ---------------------------------------------

    /// The equation in its linear form. An equation already in its linear
    /// form (text only) is its text.
    pub fn linear_text(&self) -> String {
        let content: Vec<MathElement> = self.math_contents();
        if content
            .iter()
            .filter(|c| is_content(c))
            .all(|c| c.kind == K::Run && !is_normal_run(c))
        {
            return content.iter().map(run_text).collect();
        }
        to_linear(&content, self.format)
    }

    fn math_contents(&self) -> Vec<MathElement> {
        match self.root.kind {
            K::MathParagraph => self
                .root
                .children
                .iter()
                .filter(|c| c.kind == K::Math)
                .flat_map(|m| m.children.clone())
                .collect(),
            _ => self.root.children.clone(),
        }
    }

    /// 行形式: the equation becomes one run of its linear format.
    pub fn to_linear_form(&mut self) {
        let text = self.linear_text();
        let slot = self.top_slot();
        if let Some(element) = self.element_mut(&slot) {
            element.children.retain(|c| !is_content(c));
            if !text.is_empty() {
                element.children.push(math_run(&text, Vec::new()));
            }
        }
        // Other lines of a display equation fold into the first.
        if self.root.kind == K::MathParagraph {
            let first = usize::from(slot[1]);
            let mut index = 0;
            self.root.children.retain(|c| {
                let keep = c.kind != K::Math || index == first;
                if c.kind == K::Math {
                    index += 1;
                }
                keep
            });
        }
        self.caret = self.end_of(slot);
    }

    /// 2 次元形式: the equation's text, read in its linear format, built up.
    pub fn to_professional_form(&mut self) {
        let text: String = self
            .math_contents()
            .iter()
            .filter(|c| c.kind == K::Run)
            .map(run_text)
            .collect();
        if self
            .math_contents()
            .iter()
            .any(|c| is_content(c) && c.kind != K::Run)
        {
            // Already built up.
            return;
        }
        let elements = parse_linear(&apply_math_autocorrect(&text), self.format);
        let slot = self.top_slot();
        if let Some(element) = self.element_mut(&slot) {
            element.children.retain(|c| !is_content(c));
            element.children.extend(elements);
        }
        self.caret = self.end_of(slot);
    }

    /// Writes the equation back into the document as one undo step.
    pub fn commit(&self, session: &mut dyn WordSession) -> Result<(), String> {
        session.apply_command(
            HeadlessCommand::ReplaceEquation {
                paragraph: self.target.paragraph,
                run_index: self.target.run_index,
                omml: self.omml(),
            },
            true,
        )
    }

    /// Follows the equation after an edit moved it (a display equation's
    /// paragraph stays; an inline one keeps its run).
    pub fn zone<'z>(&self, zones: &'z [MathZone]) -> Option<&'z MathZone> {
        zones.iter().find(|z| {
            z.paragraph == self.target.paragraph
                && z.display == self.target.display
                && (self.target.display || z.run_index == self.target.run_index)
        })
    }
}

/// Where the caret is in a matrix or an equation array.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MathTableContext {
    /// A matrix (rows and columns), else an equation array (rows).
    pub matrix: bool,
    path: Vec<u16>,
    pub row: usize,
    pub column: usize,
    pub rows: usize,
    pub columns: usize,
    /// The caret column's `mcJc`.
    pub justification: String,
}

/// What ホーム's commands set on an equation.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct EquationFormat {
    /// Points.
    pub size: Option<f32>,
    /// `RRGGBB`, or `auto`.
    pub color: Option<String>,
    /// Bold (`m:sty` `bi`) or not.
    pub bold: Option<bool>,
}

fn format_run(run: &mut MathElement, format: &EquationFormat) {
    let word_pr = match run
        .children
        .iter()
        .position(|c| c.local_name() == "rPr" && c.namespace.as_deref() == Some("w"))
    {
        Some(i) => i,
        None => {
            let mut pr = MathElement::tagged("rPr", Vec::new());
            pr.kind = K::Unknown;
            pr.namespace = Some("w".into());
            let at = run
                .children
                .iter()
                .position(|c| c.kind == K::Text)
                .unwrap_or(run.children.len());
            run.children.insert(at, pr);
            at
        }
    };
    let set_word = |pr: &mut MathElement, tag: &str, value: Option<String>| {
        pr.children.retain(|c| c.local_name() != tag);
        if let Some(value) = value {
            let mut p = MathElement::tagged(tag, Vec::new());
            p.kind = K::Unknown;
            p.namespace = Some("w".into());
            p.attributes.insert("val".into(), value);
            pr.children.push(p);
        }
    };
    if let Some(size) = format.size {
        let half_points = (size * 2.0).round() as u32;
        set_word(
            &mut run.children[word_pr],
            "sz",
            Some(half_points.to_string()),
        );
    }
    if let Some(color) = &format.color {
        set_word(&mut run.children[word_pr], "color", Some(color.clone()));
    }
    if let Some(bold) = format.bold {
        let pr_index = match run
            .children
            .iter()
            .position(|c| c.local_name() == "rPr" && c.namespace.is_none())
        {
            Some(i) => i,
            None => {
                let mut pr = MathElement::tagged("rPr", Vec::new());
                pr.kind = K::Unknown;
                run.children.insert(0, pr);
                0
            }
        };
        let pr = &mut run.children[pr_index];
        let upright = pr.children.iter().any(|c| {
            c.local_name() == "sty"
                && c.attributes
                    .get("val")
                    .is_some_and(|v| v == "p" || v == "b")
        });
        pr.children.retain(|c| c.local_name() != "sty");
        let value = match (bold, upright) {
            (true, true) => Some("b"),
            (true, false) => Some("bi"),
            (false, true) => Some("p"),
            (false, false) => None,
        };
        if let Some(value) = value {
            pr.children.push(MathElement::property("sty", value));
        }
    }
}

/// A matrix's column justifications (`m:mcs`), one per column.
fn column_justifications(matrix: &MathElement, columns: usize) -> Vec<String> {
    let mut out = Vec::new();
    if let Some(mcs) = matrix
        .children
        .iter()
        .find(|c| c.local_name() == "mPr")
        .and_then(|pr| pr.children.iter().find(|c| c.local_name() == "mcs"))
    {
        for mc in mcs.children.iter().filter(|c| c.local_name() == "mc") {
            let pr = mc.children.iter().find(|c| c.local_name() == "mcPr");
            let value = |name: &str| {
                pr.and_then(|p| p.children.iter().find(|c| c.local_name() == name))
                    .and_then(|p| p.attributes.get("val").cloned())
            };
            let count = value("count")
                .and_then(|v| v.parse::<usize>().ok())
                .unwrap_or(1)
                .clamp(1, 64);
            let jc = value("mcJc").unwrap_or_else(|| "center".into());
            out.extend(std::iter::repeat_n(jc, count));
        }
    }
    out.resize(columns, "center".into());
    out.truncate(columns);
    out
}

/// Writes a matrix's column justifications back as `m:mcs`, one `m:mc`
/// per run of equal columns.
fn set_column_justifications(matrix: &mut MathElement, justifications: &[String]) {
    let mut runs: Vec<(String, usize)> = Vec::new();
    for jc in justifications {
        match runs.last_mut() {
            Some((last, count)) if last == jc => *count += 1,
            _ => runs.push((jc.clone(), 1)),
        }
    }
    let mcs = {
        let mut mcs = MathElement::tagged(
            "mcs",
            runs.iter()
                .map(|(jc, count)| {
                    let mut mc = MathElement::tagged(
                        "mc",
                        vec![{
                            let mut pr = MathElement::tagged(
                                "mcPr",
                                vec![
                                    MathElement::property("count", &count.to_string()),
                                    MathElement::property("mcJc", jc),
                                ],
                            );
                            pr.kind = K::Unknown;
                            pr
                        }],
                    );
                    mc.kind = K::Unknown;
                    mc
                })
                .collect(),
        );
        mcs.kind = K::Unknown;
        mcs
    };
    let pr_index = match matrix.children.iter().position(|c| c.local_name() == "mPr") {
        Some(i) => i,
        None => {
            let mut pr = MathElement::tagged("mPr", Vec::new());
            pr.kind = K::Unknown;
            matrix.children.insert(0, pr);
            0
        }
    };
    let pr = &mut matrix.children[pr_index];
    pr.children.retain(|c| c.local_name() != "mcs");
    pr.children.push(mcs);
}

fn owner_structure(editor: &EquationEditor, structure: &[u16]) -> Vec<u16> {
    let parent = &structure[..structure.len() - 1];
    if editor
        .element(parent)
        .is_some_and(|e| e.kind == K::MatrixRow)
    {
        parent[..parent.len() - 1].to_vec()
    } else {
        structure.to_vec()
    }
}

fn vertical_distance(b: &MathBox, caret: (f64, f64, f64), down: bool) -> f64 {
    if down {
        (b.baseline - b.ascent - caret.1).abs()
    } else {
        (caret.0 - (b.baseline + b.descent)).abs()
    }
}

fn horizontal_distance(b: &MathBox, x: f64) -> f64 {
    if x < b.x {
        b.x - x
    } else if x > b.x + b.width {
        x - (b.x + b.width)
    } else {
        0.0
    }
}

fn child_of(atom: &Atom) -> usize {
    match *atom {
        Atom::Char { child, .. } | Atom::Structure { child } => child,
    }
}

/// The OMML element kinds that hold a sequence: arguments of structures and
/// the equation itself.
fn is_slot_kind(kind: &K) -> bool {
    matches!(
        kind,
        K::Math
            | K::Base
            | K::Numerator
            | K::Denominator
            | K::SubArgument
            | K::SuperArgument
            | K::Degree
            | K::FunctionName
            | K::Limit
    )
}

fn is_slot(e: &MathElement) -> bool {
    is_slot_kind(&e.kind)
}

/// An argument its structure does not show (a hidden degree or limit).
fn hidden_slot(parent: &MathElement, slot: &MathElement) -> bool {
    let flag = |pr: &str, name: &str| {
        parent
            .children
            .iter()
            .find(|c| c.local_name() == pr)
            .and_then(|p| p.children.iter().find(|c| c.local_name() == name))
            .is_some_and(|p| {
                p.attributes
                    .get("val")
                    .is_none_or(|v| matches!(v.as_str(), "1" | "true" | "on"))
            })
    };
    match (&parent.kind, &slot.kind) {
        (K::Radical, K::Degree) => flag("radPr", "degHide"),
        (K::Nary, K::SubArgument) => flag("naryPr", "subHide"),
        (K::Nary, K::SuperArgument) => flag("naryPr", "supHide"),
        _ => false,
    }
}

fn collect_slots(element: &MathElement, path: Vec<u16>, out: &mut Vec<Vec<u16>>) {
    for (i, c) in element.children.iter().enumerate() {
        let mut child_path = path.clone();
        child_path.push(i as u16);
        if is_slot(c) {
            if !hidden_slot(element, c) {
                out.push(child_path);
            }
        } else if c.kind == K::MatrixRow {
            collect_slots(c, child_path, out);
        }
    }
}

fn is_content(e: &MathElement) -> bool {
    !(e.local_name().ends_with("Pr") || e.local_name() == "ctrlPr")
}

fn has_content(e: &MathElement) -> bool {
    match e.kind {
        K::Text => e.text.as_deref().is_some_and(|t| !t.is_empty()),
        K::Run => e
            .children
            .iter()
            .any(|c| c.kind == K::Text && has_content(c)),
        _ if is_slot(e) || matches!(e.kind, K::MathParagraph | K::Unknown) => {
            e.children.iter().any(|c| is_content(c) && has_content(c))
        }
        _ => e.children.iter().any(|c| is_content(c) && has_content(c)),
    }
}

fn run_text(run: &MathElement) -> String {
    run.children
        .iter()
        .filter(|c| c.kind == K::Text)
        .filter_map(|c| c.text.clone())
        .collect()
}

fn is_normal_run(run: &MathElement) -> bool {
    run.children
        .iter()
        .filter(|c| c.local_name() == "rPr" && c.namespace.is_none())
        .any(|pr| pr.children.iter().any(|p| p.local_name() == "nor"))
}

fn normal_run(text: &str) -> MathElement {
    let mut nor = MathElement::tagged("nor", Vec::new());
    nor.kind = K::Unknown;
    math_run(text, vec![nor])
}

/// The atoms of an argument, in order.
fn atoms_of(element: &MathElement) -> Vec<Atom> {
    let mut atoms = Vec::new();
    for (child, c) in element.children.iter().enumerate() {
        if !is_content(c) {
            continue;
        }
        if c.kind == K::Run {
            for (text, t) in c.children.iter().enumerate() {
                if t.kind != K::Text {
                    continue;
                }
                for char in 0..t.text.as_deref().unwrap_or_default().chars().count() {
                    atoms.push(Atom::Char { child, text, char });
                }
            }
        } else {
            atoms.push(Atom::Structure { child });
        }
    }
    atoms
}

/// The child position atom `index` stands at, without splitting a run.
fn child_position(element: &MathElement, atoms: &[Atom], index: usize) -> usize {
    match atoms.get(index) {
        Some(atom) => child_of(atom),
        None => atoms
            .last()
            .map_or_else(|| first_content_position(element), |a| child_of(a) + 1),
    }
}

fn first_content_position(element: &MathElement) -> usize {
    element
        .children
        .iter()
        .position(is_content)
        .unwrap_or(element.children.len())
}

/// Splits the run the caret is inside (if it is) and returns the child
/// position the caret stands at.
fn split_at(element: &mut MathElement, atoms: &[Atom], index: usize) -> usize {
    match atoms.get(index) {
        Some(Atom::Char { child, text, char }) if *char > 0 => {
            let run = &mut element.children[*child];
            let mut tail = run.clone();
            let value = run.children[*text].text.clone().unwrap_or_default();
            let byte = value
                .char_indices()
                .nth(*char)
                .map_or(value.len(), |(b, _)| b);
            run.children[*text].text = Some(value[..byte].to_string());
            tail.children[*text].text = Some(value[byte..].to_string());
            element.children.insert(child + 1, tail);
            child + 1
        }
        _ => child_position(element, atoms, index),
    }
}

/// The equation's root element, read from the document.
fn equation_root(session: &dyn WordSession, target: &EquationTarget) -> Option<MathElement> {
    let document = session.word_document();
    if target.display {
        let math = document.math_paragraph(target.paragraph)?;
        let root = math
            .elements
            .iter()
            .find(|e| matches!(e.kind, K::MathParagraph | K::Math))?
            .clone();
        if root.kind == K::Math {
            return Some(MathElement::new(K::MathParagraph, vec![root]));
        }
        Some(root)
    } else {
        let paragraph = document.paragraph(target.paragraph)?;
        let math = paragraph.runs.get(target.run_index?)?.math.as_ref()?;
        math.elements.iter().find(|e| e.kind == K::Math).cloned()
    }
}

// ---- inserting new equations --------------------------------------------------

/// 挿入 > 数式: a new equation at the caret — display when the paragraph is
/// empty, inline inside text, as Word chooses — and its target.
pub fn insert_new_equation(
    session: &mut dyn WordSession,
    (paragraph, offset): (usize, usize),
    content: Option<&MathElement>,
) -> Result<EquationTarget, String> {
    let at = jade_selection::TextPosition::new(paragraph, offset);
    session.set_selection_range(Some(jade_selection::SelectionRange::collapsed(at)));
    let paragraph_text: String = session
        .word_document()
        .paragraph(at.paragraph)
        .map(|p| p.runs.iter().map(|r| r.text.as_str()).collect())
        .unwrap_or_default();
    let display = paragraph_text.trim().is_empty();
    let content: Vec<MathElement> = match content {
        Some(e) if e.kind == K::MathParagraph => e
            .children
            .iter()
            .filter(|c| c.kind == K::Math)
            .flat_map(|m| m.children.clone())
            .collect(),
        Some(e) if e.kind == K::Math => e.children.clone(),
        Some(e) => vec![e.clone()],
        None => Vec::new(),
    };
    let math = MathElement::new(K::Math, content);
    let (omml, placement) = if display {
        (
            MathElement::new(K::MathParagraph, vec![math]).to_omml(),
            HeadlessEquationPlacement::Block,
        )
    } else {
        (math.to_omml(), HeadlessEquationPlacement::Inline)
    };
    let model_offset = session
        .word_document()
        .rendered_to_model_text_position(at)
        .offset;
    let inserted = omml.clone();
    session.apply_command(
        HeadlessCommand::InsertEquation {
            paragraph: at.paragraph,
            offset: at.offset,
            omml,
            placement,
        },
        false,
    )?;
    if display {
        // The paragraph splits around the equation, which is its own
        // paragraph after the first half.
        return Ok(EquationTarget {
            paragraph: at.paragraph + 1,
            run_index: None,
            display: true,
        });
    }
    // The new math run: at the caret's model offset, holding what was
    // inserted.
    let paragraph = session
        .word_document()
        .paragraph(at.paragraph)
        .ok_or_else(|| "the paragraph is gone".to_string())?;
    let mut offset = 0;
    let mut run_index = None;
    for (index, run) in paragraph.runs.iter().enumerate() {
        if offset == model_offset
            && run
                .math
                .as_ref()
                .is_some_and(|m| m.raw_omml == inserted.trim())
        {
            run_index = Some(index);
            break;
        }
        offset += run.text.len();
    }
    Ok(EquationTarget {
        paragraph: at.paragraph,
        run_index,
        display: false,
    })
}

/// The zone of a target in the last paint.
pub fn zone_for<'z>(zones: &'z [MathZone], target: &EquationTarget) -> Option<&'z MathZone> {
    zones.iter().find(|z| {
        z.paragraph == target.paragraph
            && z.display == target.display
            && (target.display || z.run_index == target.run_index)
    })
}

/// The equation drawn at a point on a page, if any.
pub fn zone_at(zones: &[MathZone], page: usize, x: f64, y: f64) -> Option<&MathZone> {
    zones.iter().find(|z| {
        let m = &z.plan.metrics;
        let size = z.plan.boxes.first().map_or(14.0, |b| b.size_px);
        z.page == page
            && x >= z.x - 2.0
            && x <= z.x + m.width.max(size * 0.6) + 2.0
            && y >= z.baseline - m.ascent.max(size * 0.8) - 2.0
            && y <= z.baseline + m.descent.max(size * 0.3) + 2.0
    })
}

// ---- the Equation tab's galleries --------------------------------------------

/// Word's built-in equations (挿入 > 数式 ▾): their i18n keys and the
/// equation in UnicodeMath.
pub const BUILT_IN_EQUATIONS: &[(&str, &str)] = &[
    ("equation.builtin.quadratic", "x=(−b±√(b^2−4ac))/2a"),
    (
        "equation.builtin.binomial",
        "(x+a)^n=∑_(k=0)^n▒(n¦k) x^k a^(n−k)",
    ),
    ("equation.builtin.circle", "A=πr^2"),
    (
        "equation.builtin.fourier",
        "f(x)=a_0+∑_(n=1)^∞▒(a_n cos⁡〖nπx/L〗+b_n sin⁡〖nπx/L〗)",
    ),
    ("equation.builtin.pythagorean", "a^2+b^2=c^2"),
    ("equation.builtin.sum", "(1+x)^n=1+nx/1!+(n(n−1)x^2)/2!+⋯"),
    (
        "equation.builtin.taylor",
        "e^x=1+x/1!+x^2/2!+x^3/3!+⋯,    −∞<x<∞",
    ),
    (
        "equation.builtin.trig1",
        "sin⁡α±sin⁡β=2 sin⁡〖1/2 (α±β)〗 cos⁡〖1/2 (α∓β)〗",
    ),
    (
        "equation.builtin.trig2",
        "cos⁡α+cos⁡β=2 cos⁡〖1/2 (α+β)〗 cos⁡〖1/2 (α−β)〗",
    ),
];

/// A built-in equation as an `m:oMathPara`, as Word inserts it.
pub fn built_in_equation(index: usize) -> Option<MathElement> {
    let (_, text) = BUILT_IN_EQUATIONS.get(index)?;
    let math = linear_to_omath(text, LinearFormat::UnicodeMath);
    Some(MathElement::new(K::MathParagraph, vec![math]))
}

/// The Equation tab's 構造 groups.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum StructureGroup {
    Fraction,
    Script,
    Radical,
    Integral,
    LargeOperator,
    Bracket,
    Function,
    Accent,
    LimitLog,
    Operator,
    Matrix,
}

impl StructureGroup {
    pub const ALL: [StructureGroup; 11] = [
        StructureGroup::Fraction,
        StructureGroup::Script,
        StructureGroup::Radical,
        StructureGroup::Integral,
        StructureGroup::LargeOperator,
        StructureGroup::Bracket,
        StructureGroup::Function,
        StructureGroup::Accent,
        StructureGroup::LimitLog,
        StructureGroup::Operator,
        StructureGroup::Matrix,
    ];

    /// The shell's i18n key for the group's button.
    pub fn key(self) -> &'static str {
        match self {
            StructureGroup::Fraction => "equation.structure.fraction",
            StructureGroup::Script => "equation.structure.script",
            StructureGroup::Radical => "equation.structure.radical",
            StructureGroup::Integral => "equation.structure.integral",
            StructureGroup::LargeOperator => "equation.structure.large_operator",
            StructureGroup::Bracket => "equation.structure.bracket",
            StructureGroup::Function => "equation.structure.function",
            StructureGroup::Accent => "equation.structure.accent",
            StructureGroup::LimitLog => "equation.structure.limit_log",
            StructureGroup::Operator => "equation.structure.operator",
            StructureGroup::Matrix => "equation.structure.matrix",
        }
    }

    /// The group's items: how each previews (UnicodeMath, `⬚` an empty
    /// argument, as Word's galleries show one) and the template it inserts.
    pub fn items(self) -> &'static [&'static str] {
        match self {
            StructureGroup::Fraction => &["⬚/⬚", "⬚⁄⬚", "⬚∕⬚", "(⬚¦⬚)", "dy/dx", "π/2"],
            StructureGroup::Script => &["⬚^⬚", "⬚_⬚", "⬚_⬚^⬚", "_⬚^⬚ ⬚", "x^2", "e^(−iωt)"],
            StructureGroup::Radical => &["√⬚", "√(⬚&⬚)", "∛⬚", "∜⬚", "√(a^2+b^2)"],
            StructureGroup::Integral => &[
                "∫▒⬚",
                "∫_⬚^⬚▒⬚",
                "∬▒⬚",
                "∬_⬚^⬚▒⬚",
                "∭▒⬚",
                "∮▒⬚",
                "∯▒⬚",
                "∰▒⬚",
                "dx",
                "dθ",
            ],
            StructureGroup::LargeOperator => &[
                "∑▒⬚",
                "∑_⬚^⬚▒⬚",
                "∑_⬚▒⬚",
                "∏▒⬚",
                "∏_⬚^⬚▒⬚",
                "∐_⬚^⬚▒⬚",
                "⋃_⬚^⬚▒⬚",
                "⋂_⬚^⬚▒⬚",
                "⋁_⬚^⬚▒⬚",
                "⋀_⬚^⬚▒⬚",
            ],
            StructureGroup::Bracket => &[
                "(⬚)",
                "[⬚]",
                "{⬚}",
                "⟨⬚⟩",
                "|⬚|",
                "‖⬚‖",
                "⌊⬚⌋",
                "⌈⬚⌉",
                "(⬚│⬚)",
                "{■(⬚@⬚)┤",
                "(■(⬚@⬚))",
            ],
            StructureGroup::Function => &[
                "sin⁡⬚",
                "cos⁡⬚",
                "tan⁡⬚",
                "csc⁡⬚",
                "sec⁡⬚",
                "cot⁡⬚",
                "sinh⁡⬚",
                "cosh⁡⬚",
                "tanh⁡⬚",
                "sin^(−1)⁡⬚",
                "cos^(−1)⁡⬚",
                "tan^(−1)⁡⬚",
            ],
            StructureGroup::Accent => &[
                "⬚\u{0307}",
                "⬚\u{0308}",
                "⬚\u{20DB}",
                "⬚\u{0302}",
                "⬚\u{030C}",
                "⬚\u{0301}",
                "⬚\u{0300}",
                "⬚\u{0306}",
                "⬚\u{0303}",
                "⬚\u{0305}",
                "⬚\u{20D7}",
                "⬚\u{20D6}",
                "⬚\u{20E1}",
                "¯(⬚)",
                "▁(⬚)",
                "⏞(⬚)",
                "⏟(⬚)",
                "▭(⬚)",
            ],
            StructureGroup::LimitLog => &[
                "log_⬚⁡⬚",
                "log⁡⬚",
                "lim_⬚⁡⬚",
                "min_⬚⁡⬚",
                "max_⬚⁡⬚",
                "ln⁡⬚",
                "lim_(n→∞)⁡〖(1+1/n)^n〗",
            ],
            StructureGroup::Operator => &[
                "≔",
                "==",
                "+=",
                "−=",
                "≝",
                "≞",
                "≜",
                "⬚┴⬚",
                "⬚┬⬚",
                "→┴⬚",
                "←┴⬚",
                "⇒┴⬚",
                "⏞(⬚)^⬚",
                "⏟(⬚)_⬚",
            ],
            StructureGroup::Matrix => &[
                "■(⬚&⬚)",
                "■(⬚@⬚)",
                "■(⬚&⬚&⬚)",
                "■(⬚@⬚@⬚)",
                "■(⬚&⬚@⬚&⬚)",
                "■(⬚&⬚&⬚@⬚&⬚&⬚@⬚&⬚&⬚)",
                "⋯",
                "⋮",
                "⋱",
                "(■(1&0@0&1))",
                "[■(⬚&⬚@⬚&⬚)]",
                "|■(⬚&⬚@⬚&⬚)|",
            ],
        }
    }
}

/// The template a structure item inserts: its preview read with `⬚` as an
/// empty argument.
pub fn structure_template(group: StructureGroup, index: usize) -> Option<MathElement> {
    let text = group.items().get(index)?;
    let elements = parse_linear(text, LinearFormat::UnicodeMath);
    let mut elements = elements;
    for e in &mut elements {
        empty_placeholders(e);
    }
    match elements.len() {
        0 => None,
        1 => elements.into_iter().next(),
        _ => Some(MathElement::new(
            K::Box,
            vec![MathElement::new(K::Base, elements)],
        )),
    }
}

/// Turns `⬚` runs into empty arguments.
fn empty_placeholders(e: &mut MathElement) {
    if is_slot(e) || e.kind == K::Math {
        e.children
            .retain(|c| !(c.kind == K::Run && run_text(c) == "⬚"));
        for c in &mut e.children {
            if c.kind == K::Run {
                for t in &mut c.children {
                    if let Some(text) = t.text.as_mut() {
                        *text = text.replace('⬚', "");
                    }
                }
            }
        }
        e.children
            .retain(|c| !(c.kind == K::Run && run_text(c).is_empty()));
    }
    for c in &mut e.children {
        empty_placeholders(c);
    }
}

/// The 記号と特殊文字 gallery's categories (Word's 基本数式, ギリシャ文字 …).
pub const SYMBOL_CATEGORIES: &[(&str, &str)] = &[
    (
        "equation.symbols.basic",
        "±∞=≠~×÷!∝<≪>≫≤≥∓≅≈≡∀∁∂√∛∜∪∩∅%°℉℃∆∇∃∄∈∋←↑→↓↔∴+−¬αβγδεϵθϑμπρστφω∗∙⋮⋯⋰⋱ℵℶ∎",
    ),
    (
        "equation.symbols.greek",
        "αβγδεϵζηθϑικλμνξοπϖρϱσςτυφϕχψωΑΒΓΔΕΖΗΘΙΚΛΜΝΞΟΠΡΣΤΥΦΧΨΩ",
    ),
    (
        "equation.symbols.letterlike",
        "∀∁ℂ∂ðℇϜℊℋℌℍℎℏℐℑℒℓℕ№℘ℙℚℛℜℝℤ℧Åℬℭ℮ℯℰℱℳℴℵℶℷℸ",
    ),
    (
        "equation.symbols.operators",
        "+−±∓∖×∗∘∙√∛∜∝∞∟∠∡∢∣∤∥∦∧∨∩∪∫∬∭∮∯∰∱∲∳∴∵∶∷∸∹∺∻∼∽∾≀≁≂≃≄≅≆≇≈≉≊≋≌≍≎≏≐≑≒≓≔≕≖≗≘≙≚≛≜≝≞≟≠≡≢≣≤≥≦≧≨≩≪≫≬≮≯≰≱≲≳≴≵≶≷≸≹≺≻≼≽≾≿⊀⊁⊂⊃⊄⊅⊆⊇⊈⊉⊊⊋⊌⊍⊎⊏⊐⊑⊒⊓⊔⊕⊖⊗⊘⊙⊚⊛⊜⊝⊞⊟⊠⊡⊢⊣⊤⊥",
    ),
    (
        "equation.symbols.arrows",
        "←↑→↓↔↕↖↗↘↙↚↛↜↝↞↟↠↡↢↣↤↥↦↧↨↩↪↫↬↭↮↯↰↱↲↳↶↷↺↻↼↽↾↿⇀⇁⇂⇃⇄⇅⇆⇇⇈⇉⇊⇋⇌⇍⇎⇏⇐⇑⇒⇓⇔⇕⇖⇗⇘⇙⇚⇛⟵⟶⟷⟸⟹⟺⟼",
    ),
    (
        "equation.symbols.negated",
        "≠≢≮≯≰≱≁≄≇≉≭⊀⊁⊄⊅⊈⊉∉∌∤∦⋢⋣⋦⋧⋨⋩⋪⋫⋬⋭",
    ),
    (
        "equation.symbols.geometry",
        "∟∠∡∢⊾⊿⋕⊥∤∥∦∶∷∴∵∎△▭▱□▫◇◊○◯",
    ),
];

#[cfg(test)]
mod tests;
