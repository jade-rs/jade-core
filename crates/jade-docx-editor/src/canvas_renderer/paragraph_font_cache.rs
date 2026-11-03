//! Owned inputs for Windows paragraph font-line summaries.
use jade_docx_layout::line_metrics::run_font_slots;
use jade_docx_reader::document::{Paragraph, Run, RunProperties};
use rustc_hash::FxHashMap;

const MAX_ENTRIES: usize = 128;
const MAX_RUNS: usize = 64;
pub(super) type FontLines = (Option<f32>, Option<f32>);

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) struct Context {
    pub east_asian: bool,
    pub underline: bool,
    pub char_spacing: Option<u64>,
}

#[derive(Clone, Copy, Hash, PartialEq, Eq)]
enum Key {
    Node(u32),
    Address(usize),
}
fn key(paragraph: &Paragraph) -> Key {
    paragraph.node_id.map_or_else(
        || Key::Address(paragraph as *const Paragraph as usize),
        |id| Key::Node(id.raw()),
    )
}
fn contributing_runs(paragraph: &Paragraph) -> impl Iterator<Item = &Run> {
    paragraph.runs.iter().filter(|run| {
        (!run.text.is_empty() || run.note_separator.is_some())
            && !run.suppresses_line_height_contribution()
    })
}
fn style_id(paragraph: &Paragraph) -> Option<&str> {
    paragraph
        .properties
        .as_ref()
        .and_then(|props| props.style.as_deref())
}
// Match the inputs consumed by font-family/style resolution, border extent
// and space-for-underline sizing. Paint colors, revision/export provenance and
// other decorations cannot change this pure font-line summary. The painter's
// independent activation prepass still runs after a cache hit.
fn same_font_inputs(a: Option<&RunProperties>, b: Option<&RunProperties>) -> bool {
    let (Some(a), Some(b)) = (a, b) else {
        return a.is_none() && b.is_none();
    };
    a.style == b.style
        && a.font_family == b.font_family
        && a.fonts == b.fonts
        && a.font_size == b.font_size
        && a.font_size_complex == b.font_size_complex
        && a.bold == b.bold
        && a.italic == b.italic
        && a.bold_complex == b.bold_complex
        && a.italic_complex == b.italic_complex
        && a.baseline == b.baseline
        && a.text_position == b.text_position
        && a.kerning == b.kerning
        && a.text_effects == b.text_effects
        && a.authored_east_asia_font_hint == b.authored_east_asia_font_hint
        && a.authored_east_asia_font_name == b.authored_east_asia_font_name
        && a.language == b.language
        && a.east_asia_language == b.east_asia_language
        && a.border == b.border
        && a.underline == b.underline
        && a.underline_kind == b.underline_kind
        && a.char_spacing == b.char_spacing
        && a.text_scale == b.text_scale
        && a.fit_text == b.fit_text
        && a.caps == b.caps
        && a.small_caps == b.small_caps
        && a.ruby == b.ruby
        && a.east_asian_layout == b.east_asian_layout
}

struct RunInput {
    properties: Option<RunProperties>,
    slots: (bool, bool),
}
struct Entry {
    context: Context,
    style: Option<String>,
    runs: Vec<RunInput>,
    lines: FontLines,
}

#[derive(Default)]
pub(super) struct ParagraphFontCache {
    entries: FxHashMap<Key, Entry>,
}
impl ParagraphFontCache {
    pub(super) fn clear(&mut self) {
        self.entries.clear();
    }
    pub(super) fn get(&self, paragraph: &Paragraph, context: Context) -> Option<FontLines> {
        let entry = self.entries.get(&key(paragraph))?;
        if entry.context != context || entry.style.as_deref() != style_id(paragraph) {
            return None;
        }
        // Node IDs and addresses are lookup hints only. Font inputs are owned
        // and compared again, including after in-place public run mutations.
        let mut inputs = entry.runs.iter();
        for run in contributing_runs(paragraph) {
            let input = inputs.next()?;
            if !same_font_inputs(input.properties.as_ref(), run.properties.as_ref())
                || input.slots != run_font_slots(run, context.east_asian)
            {
                return None;
            }
        }
        inputs.next().is_none().then_some(entry.lines)
    }
    pub(super) fn insert(&mut self, paragraph: &Paragraph, context: Context, lines: FontLines) {
        let key = key(paragraph);
        // Keep warm small-document entries when a large corpus exceeds the
        // bound, instead of repeatedly clearing and cloning every paragraph.
        if paragraph.runs.len() > MAX_RUNS
            || (self.entries.len() >= MAX_ENTRIES && !self.entries.contains_key(&key))
        {
            return;
        }
        let runs = contributing_runs(paragraph)
            .map(|run| RunInput {
                properties: run.properties.clone(),
                slots: run_font_slots(run, context.east_asian),
            })
            .collect();
        self.entries.insert(
            key,
            Entry {
                context,
                style: style_id(paragraph).map(str::to_owned),
                runs,
                lines,
            },
        );
    }
}
