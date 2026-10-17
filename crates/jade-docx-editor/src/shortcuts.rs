//! Keyboard shortcut parsing and matching helpers used by the editing surface.
//!
//! The goal is to keep key binding logic in one place so that commands can be
//! triggered both from built-in bindings (Backspace/Delete) and optionally from
//! user supplied overrides passed through `Options::set_shortcuts`.

use std::fmt;

/// Represents the logical command a shortcut refers to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ShortcutAction {
    DeleteBackward,
    DeleteForward,
    Undo,
    Redo,
    Copy,
    Paste,
    Cut,
    ToggleBold,
    ToggleItalic,
    ToggleUnderline,
    ToggleWordUnderline,
    ToggleDoubleUnderline,
    ToggleSubscript,
    ToggleSuperscript,
    ClearFormatting,
    IncreaseFontSize,
    DecreaseFontSize,
    AlignLeft,
    AlignCenter,
    AlignRight,
    AlignJustify,
    /// Word's Ctrl+M: step the paragraph indent (list level on list items).
    IncreaseIndent,
    /// Word's Ctrl+Shift+M.
    DecreaseIndent,
    /// Word's Ctrl+Q: strip direct paragraph formatting back to the style.
    ResetParagraphFormatting,
    /// Word's Ctrl+T: step the hanging indent (the first line stays put).
    IncreaseHangingIndent,
    /// Word's Ctrl+Shift+T.
    DecreaseHangingIndent,
    /// Word's Ctrl+1: single line spacing.
    SetLineSpacingSingle,
    /// Word's Ctrl+5: 1.5 line spacing.
    SetLineSpacingOneAndHalf,
    /// Word's Ctrl+2: double line spacing.
    SetLineSpacingDouble,
    /// Word's Ctrl+0: add or remove 12pt of space before the paragraph.
    ToggleSpaceBefore,
    /// Word's Ctrl+]: grow every run by exactly one point.
    IncreaseFontSizeByPoint,
    /// Word's Ctrl+[.
    DecreaseFontSizeByPoint,
    /// Word's Ctrl+Enter: insert a manual page break.
    InsertPageBreak,
    /// Word's Ctrl+Shift+L: toggle a bulleted list.
    ToggleBulletList,
    /// Move the caret to the very first byte offset of the focus paragraph.
    /// JS layer reads `event.shiftKey` to decide whether to extend the
    /// selection — bind both `Ctrl+A` and `Ctrl+Shift+A` to expose both modes.
    MoveParagraphStart,
    /// Move the caret to the end-of-text byte offset of the focus paragraph.
    /// See `MoveParagraphStart` regarding shift-extend handling.
    MoveParagraphEnd,
    /// Word's Shift+F3: cycle the selection (or the word at the caret)
    /// through Capitalize Each Word, UPPERCASE and lowercase.
    ChangeCase,
    /// Word's Ctrl+Shift+N: apply the document's default paragraph style.
    ApplyNormalStyle,
    /// Word's Ctrl+Alt+1 (⌘⌥1 on Mac): apply Heading 1.
    ApplyHeading1,
    /// Word's Ctrl+Alt+2 (⌘⌥2 on Mac): apply Heading 2.
    ApplyHeading2,
    /// Word's Ctrl+Alt+3 (⌘⌥3 on Mac): apply Heading 3.
    ApplyHeading3,
    /// Word's Ctrl+Shift+Space: insert U+00A0. (⌥Space already types it on Mac.)
    InsertNonBreakingSpace,
    /// Word's Ctrl+Shift+- (⌘⇧- on Mac): insert a nonbreaking hyphen (`w:noBreakHyphen`).
    InsertNonBreakingHyphen,
    /// Word's Ctrl+Alt+C AutoCorrect symbol: ©.
    InsertCopyright,
    /// Word's Ctrl+Alt+R: ®.
    InsertRegistered,
    /// Word's Ctrl+Alt+T: ™.
    InsertTrademark,
    /// Word's Ctrl+Alt+.: the ellipsis character.
    InsertEllipsis,
}

impl ShortcutAction {
    /// Actions `Options::set_shortcuts` reads through `with_bindings`
    /// rather than a dedicated builder.
    pub const WORD_EXTRAS: [ShortcutAction; 11] = [
        ShortcutAction::ChangeCase,
        ShortcutAction::ApplyNormalStyle,
        ShortcutAction::ApplyHeading1,
        ShortcutAction::ApplyHeading2,
        ShortcutAction::ApplyHeading3,
        ShortcutAction::InsertNonBreakingSpace,
        ShortcutAction::InsertNonBreakingHyphen,
        ShortcutAction::InsertCopyright,
        ShortcutAction::InsertRegistered,
        ShortcutAction::InsertTrademark,
        ShortcutAction::InsertEllipsis,
    ];

    /// The camelCase name used by `Options::set_shortcuts`, the worker
    /// protocol and the TypeScript `EditorShortcutMap`.
    pub fn name(self) -> &'static str {
        match self {
            ShortcutAction::DeleteBackward => "deleteBackward",
            ShortcutAction::DeleteForward => "deleteForward",
            ShortcutAction::Undo => "undo",
            ShortcutAction::Redo => "redo",
            ShortcutAction::Copy => "copy",
            ShortcutAction::Paste => "paste",
            ShortcutAction::Cut => "cut",
            ShortcutAction::ToggleBold => "toggleBold",
            ShortcutAction::ToggleItalic => "toggleItalic",
            ShortcutAction::ToggleUnderline => "toggleUnderline",
            ShortcutAction::ToggleWordUnderline => "toggleWordUnderline",
            ShortcutAction::ToggleDoubleUnderline => "toggleDoubleUnderline",
            ShortcutAction::ToggleSubscript => "toggleSubscript",
            ShortcutAction::ToggleSuperscript => "toggleSuperscript",
            ShortcutAction::ClearFormatting => "clearFormatting",
            ShortcutAction::IncreaseFontSize => "increaseFontSize",
            ShortcutAction::DecreaseFontSize => "decreaseFontSize",
            ShortcutAction::AlignLeft => "alignLeft",
            ShortcutAction::AlignCenter => "alignCenter",
            ShortcutAction::AlignRight => "alignRight",
            ShortcutAction::AlignJustify => "alignJustify",
            ShortcutAction::IncreaseIndent => "increaseIndent",
            ShortcutAction::DecreaseIndent => "decreaseIndent",
            ShortcutAction::ResetParagraphFormatting => "resetParagraphFormatting",
            ShortcutAction::IncreaseHangingIndent => "increaseHangingIndent",
            ShortcutAction::DecreaseHangingIndent => "decreaseHangingIndent",
            ShortcutAction::SetLineSpacingSingle => "setLineSpacingSingle",
            ShortcutAction::SetLineSpacingOneAndHalf => "setLineSpacingOneAndHalf",
            ShortcutAction::SetLineSpacingDouble => "setLineSpacingDouble",
            ShortcutAction::ToggleSpaceBefore => "toggleSpaceBefore",
            ShortcutAction::IncreaseFontSizeByPoint => "increaseFontSizeByPoint",
            ShortcutAction::DecreaseFontSizeByPoint => "decreaseFontSizeByPoint",
            ShortcutAction::InsertPageBreak => "insertPageBreak",
            ShortcutAction::ToggleBulletList => "toggleBulletList",
            ShortcutAction::MoveParagraphStart => "moveParagraphStart",
            ShortcutAction::MoveParagraphEnd => "moveParagraphEnd",
            ShortcutAction::ChangeCase => "changeCase",
            ShortcutAction::ApplyNormalStyle => "applyNormalStyle",
            ShortcutAction::ApplyHeading1 => "applyHeading1",
            ShortcutAction::ApplyHeading2 => "applyHeading2",
            ShortcutAction::ApplyHeading3 => "applyHeading3",
            ShortcutAction::InsertNonBreakingSpace => "insertNonBreakingSpace",
            ShortcutAction::InsertNonBreakingHyphen => "insertNonBreakingHyphen",
            ShortcutAction::InsertCopyright => "insertCopyright",
            ShortcutAction::InsertRegistered => "insertRegistered",
            ShortcutAction::InsertTrademark => "insertTrademark",
            ShortcutAction::InsertEllipsis => "insertEllipsis",
        }
    }
}

/// Normalised representation of a single key binding (key + modifier state).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ShortcutBinding {
    key: String,
    ctrl: bool,
    alt: bool,
    shift: bool,
    meta: bool,
}

impl ShortcutBinding {
    /// Parses a human readable accelerator string such as `"Ctrl+H"` or
    /// `"Backspace"`. Parsing is intentionally forgiving wrt casing and uses a
    /// lowered string for matching.
    pub fn parse(binding: &str) -> Result<Self, String> {
        let mut ctrl = false;
        let mut alt = false;
        let mut shift = false;
        let mut meta = false;
        let mut key: Option<String> = None;

        for token in binding.split('+') {
            let trimmed = token.trim();
            if trimmed.is_empty() {
                continue;
            }

            match trimmed.to_ascii_lowercase().as_str() {
                "ctrl" | "control" => ctrl = true,
                "shift" => shift = true,
                "alt" | "option" => alt = true,
                "meta" | "cmd" | "command" => meta = true,
                other => {
                    if key.is_some() {
                        return Err(format!(
                            "multiple keys detected while parsing shortcut: `{binding}`"
                        ));
                    }
                    key = Some(normalize_key(other));
                }
            }
        }

        let key = key.ok_or_else(|| {
            format!("shortcut `{binding}` does not specify a primary key (e.g. `Backspace`)")
        })?;

        Ok(Self {
            key,
            ctrl,
            alt,
            shift,
            meta,
        })
    }

    /// The normalised primary key (`"space"`, `"="`, lower-case letters).
    pub fn key(&self) -> &str {
        &self.key
    }

    pub fn ctrl(&self) -> bool {
        self.ctrl
    }

    pub fn alt(&self) -> bool {
        self.alt
    }

    pub fn shift(&self) -> bool {
        self.shift
    }

    pub fn meta(&self) -> bool {
        self.meta
    }

    /// Returns `true` when the provided key and modifier state match the binding.
    pub fn matches(&self, key: &str, ctrl: bool, alt: bool, shift: bool, meta: bool) -> bool {
        self.key == normalize_key(key)
            && self.ctrl == ctrl
            && self.alt == alt
            && self.shift == shift
            && self.meta == meta
    }
}

impl fmt::Display for ShortcutBinding {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut parts = Vec::new();
        if self.ctrl {
            parts.push("Ctrl");
        }
        if self.alt {
            parts.push("Alt");
        }
        if self.shift {
            parts.push("Shift");
        }
        if self.meta {
            parts.push("Meta");
        }
        parts.push(self.key.as_str());
        write!(f, "{}", parts.join("+"))
    }
}

#[derive(Clone, Debug)]
pub struct ShortcutConfig {
    delete_backward: Vec<ShortcutBinding>,
    delete_forward: Vec<ShortcutBinding>,
    undo: Vec<ShortcutBinding>,
    redo: Vec<ShortcutBinding>,
    copy: Vec<ShortcutBinding>,
    paste: Vec<ShortcutBinding>,
    cut: Vec<ShortcutBinding>,
    toggle_bold: Vec<ShortcutBinding>,
    toggle_italic: Vec<ShortcutBinding>,
    toggle_underline: Vec<ShortcutBinding>,
    toggle_word_underline: Vec<ShortcutBinding>,
    toggle_double_underline: Vec<ShortcutBinding>,
    toggle_subscript: Vec<ShortcutBinding>,
    toggle_superscript: Vec<ShortcutBinding>,
    clear_formatting: Vec<ShortcutBinding>,
    increase_font_size: Vec<ShortcutBinding>,
    decrease_font_size: Vec<ShortcutBinding>,
    align_left: Vec<ShortcutBinding>,
    align_center: Vec<ShortcutBinding>,
    align_right: Vec<ShortcutBinding>,
    align_justify: Vec<ShortcutBinding>,
    increase_indent: Vec<ShortcutBinding>,
    decrease_indent: Vec<ShortcutBinding>,
    reset_paragraph_formatting: Vec<ShortcutBinding>,
    increase_hanging_indent: Vec<ShortcutBinding>,
    decrease_hanging_indent: Vec<ShortcutBinding>,
    set_line_spacing_single: Vec<ShortcutBinding>,
    set_line_spacing_one_and_half: Vec<ShortcutBinding>,
    set_line_spacing_double: Vec<ShortcutBinding>,
    toggle_space_before: Vec<ShortcutBinding>,
    increase_font_size_by_point: Vec<ShortcutBinding>,
    decrease_font_size_by_point: Vec<ShortcutBinding>,
    insert_page_break: Vec<ShortcutBinding>,
    toggle_bullet_list: Vec<ShortcutBinding>,
    move_paragraph_start: Vec<ShortcutBinding>,
    move_paragraph_end: Vec<ShortcutBinding>,
    change_case: Vec<ShortcutBinding>,
    apply_normal_style: Vec<ShortcutBinding>,
    apply_heading1: Vec<ShortcutBinding>,
    apply_heading2: Vec<ShortcutBinding>,
    apply_heading3: Vec<ShortcutBinding>,
    insert_non_breaking_space: Vec<ShortcutBinding>,
    insert_non_breaking_hyphen: Vec<ShortcutBinding>,
    insert_copyright: Vec<ShortcutBinding>,
    insert_registered: Vec<ShortcutBinding>,
    insert_trademark: Vec<ShortcutBinding>,
    insert_ellipsis: Vec<ShortcutBinding>,
}

impl Default for ShortcutConfig {
    fn default() -> Self {
        Self {
            delete_backward: vec![
                ShortcutBinding::parse("Backspace").expect("valid default shortcut"),
                ShortcutBinding::parse("Ctrl+H").expect("valid default shortcut"),
            ],
            delete_forward: vec![
                ShortcutBinding::parse("Delete").expect("valid default shortcut"),
                ShortcutBinding::parse("Ctrl+D").expect("valid default shortcut"),
            ],
            undo: vec![
                ShortcutBinding::parse("Ctrl+Z").expect("valid default shortcut"),
                ShortcutBinding::parse("Meta+Z").expect("valid default shortcut"),
            ],
            redo: vec![
                ShortcutBinding::parse("Ctrl+Shift+Z").expect("valid default shortcut"),
                ShortcutBinding::parse("Meta+Shift+Z").expect("valid default shortcut"),
                ShortcutBinding::parse("Ctrl+Y").expect("valid default shortcut"),
                // Word for Mac redoes with ⌘Y.
                ShortcutBinding::parse("Meta+Y").expect("valid default shortcut"),
            ],
            copy: vec![
                ShortcutBinding::parse("Ctrl+C").expect("valid default shortcut"),
                ShortcutBinding::parse("Meta+C").expect("valid default shortcut"),
            ],
            paste: vec![
                ShortcutBinding::parse("Ctrl+V").expect("valid default shortcut"),
                ShortcutBinding::parse("Meta+V").expect("valid default shortcut"),
            ],
            cut: vec![
                ShortcutBinding::parse("Ctrl+X").expect("valid default shortcut"),
                ShortcutBinding::parse("Meta+X").expect("valid default shortcut"),
            ],
            toggle_bold: vec![
                ShortcutBinding::parse("Ctrl+B").expect("valid default shortcut"),
                ShortcutBinding::parse("Meta+B").expect("valid default shortcut"),
            ],
            toggle_italic: vec![
                ShortcutBinding::parse("Ctrl+I").expect("valid default shortcut"),
                ShortcutBinding::parse("Meta+I").expect("valid default shortcut"),
            ],
            toggle_underline: vec![
                ShortcutBinding::parse("Ctrl+U").expect("valid default shortcut"),
                ShortcutBinding::parse("Meta+U").expect("valid default shortcut"),
            ],
            toggle_word_underline: vec![
                ShortcutBinding::parse("Ctrl+Shift+W").expect("valid default shortcut"),
                ShortcutBinding::parse("Meta+Shift+W").expect("valid default shortcut"),
            ],
            toggle_double_underline: vec![
                ShortcutBinding::parse("Ctrl+Shift+D").expect("valid default shortcut"),
                ShortcutBinding::parse("Meta+Shift+D").expect("valid default shortcut"),
            ],
            toggle_subscript: vec![
                ShortcutBinding::parse("Ctrl+=").expect("valid default shortcut"),
                ShortcutBinding::parse("Meta+=").expect("valid default shortcut"),
            ],
            toggle_superscript: vec![
                ShortcutBinding::parse("Ctrl+Shift+=").expect("valid default shortcut"),
                ShortcutBinding::parse("Meta+Shift+=").expect("valid default shortcut"),
            ],
            clear_formatting: vec![
                ShortcutBinding::parse("Ctrl+Space").expect("valid default shortcut")
            ],
            increase_font_size: vec![
                ShortcutBinding::parse("Ctrl+Shift+>").expect("valid default shortcut"),
                ShortcutBinding::parse("Meta+Shift+>").expect("valid default shortcut"),
                ShortcutBinding::parse("Ctrl+Shift+.").expect("valid default shortcut"),
                ShortcutBinding::parse("Meta+Shift+.").expect("valid default shortcut"),
            ],
            decrease_font_size: vec![
                ShortcutBinding::parse("Ctrl+Shift+<").expect("valid default shortcut"),
                ShortcutBinding::parse("Meta+Shift+<").expect("valid default shortcut"),
                ShortcutBinding::parse("Ctrl+Shift+,").expect("valid default shortcut"),
                ShortcutBinding::parse("Meta+Shift+,").expect("valid default shortcut"),
            ],
            align_left: vec![
                ShortcutBinding::parse("Ctrl+L").expect("valid default shortcut"),
                ShortcutBinding::parse("Meta+L").expect("valid default shortcut"),
            ],
            align_center: vec![
                ShortcutBinding::parse("Ctrl+E").expect("valid default shortcut"),
                ShortcutBinding::parse("Meta+E").expect("valid default shortcut"),
            ],
            align_right: vec![
                ShortcutBinding::parse("Ctrl+R").expect("valid default shortcut"),
                ShortcutBinding::parse("Meta+R").expect("valid default shortcut"),
            ],
            align_justify: vec![
                ShortcutBinding::parse("Ctrl+J").expect("valid default shortcut"),
                ShortcutBinding::parse("Meta+J").expect("valid default shortcut"),
            ],
            // Meta variants are deliberately absent: ⌘M minimizes the window
            // and ⌘Q quits the browser, so capturing them would fight the OS.
            increase_indent: vec![ShortcutBinding::parse("Ctrl+M").expect("valid default shortcut")],
            decrease_indent: vec![
                ShortcutBinding::parse("Ctrl+Shift+M").expect("valid default shortcut")
            ],
            reset_paragraph_formatting: vec![
                ShortcutBinding::parse("Ctrl+Q").expect("valid default shortcut")
            ],
            // Word's remaining paragraph accelerators. Meta variants stay
            // absent (⌘T opens a tab, ⌘0 resets zoom, ⌘1/2/5 switch tabs).
            // On Windows/Linux Chrome, Ctrl+T and Ctrl+1..8 are reserved and
            // never reach the page, so these bindings are simply inert there
            // rather than fighting the browser; hosts can rebind them.
            increase_hanging_indent: vec![
                ShortcutBinding::parse("Ctrl+T").expect("valid default shortcut")
            ],
            decrease_hanging_indent: vec![
                ShortcutBinding::parse("Ctrl+Shift+T").expect("valid default shortcut")
            ],
            set_line_spacing_single: vec![
                ShortcutBinding::parse("Ctrl+1").expect("valid default shortcut")
            ],
            set_line_spacing_one_and_half: vec![
                ShortcutBinding::parse("Ctrl+5").expect("valid default shortcut")
            ],
            set_line_spacing_double: vec![
                ShortcutBinding::parse("Ctrl+2").expect("valid default shortcut")
            ],
            toggle_space_before: vec![
                ShortcutBinding::parse("Ctrl+0").expect("valid default shortcut")
            ],
            // ⌘[ / ⌘] are browser history navigation, so Meta variants stay
            // absent here too.
            increase_font_size_by_point: vec![
                ShortcutBinding::parse("Ctrl+]").expect("valid default shortcut")
            ],
            decrease_font_size_by_point: vec![
                ShortcutBinding::parse("Ctrl+[").expect("valid default shortcut")
            ],
            insert_page_break: vec![
                ShortcutBinding::parse("Ctrl+Enter").expect("valid default shortcut"),
                ShortcutBinding::parse("Meta+Enter").expect("valid default shortcut"),
            ],
            toggle_bullet_list: vec![
                ShortcutBinding::parse("Ctrl+Shift+L").expect("valid default shortcut"),
                ShortcutBinding::parse("Meta+Shift+L").expect("valid default shortcut"),
            ],
            // Defaults are intentionally left empty here so shipping consumers
            // do not unexpectedly steal the browser's `Ctrl+A` (Select All).
            // Application shells (e.g. jade-ide) can opt into Emacs-style
            // bindings by passing them through `Options::set_shortcuts` —
            // see `packages/jade-ide/src/lib/keybindings.ts`.
            move_paragraph_start: Vec::new(),
            move_paragraph_end: Vec::new(),
            // Word accelerators below that a browser reserves (Ctrl+Shift+N
            // opens an incognito window on Windows Chrome) never reach the
            // page there and are simply inert; the Mac/desktop variants work.
            change_case: vec![ShortcutBinding::parse("Shift+F3").expect("valid default shortcut")],
            apply_normal_style: vec![
                ShortcutBinding::parse("Ctrl+Shift+N").expect("valid default shortcut"),
                ShortcutBinding::parse("Meta+Shift+N").expect("valid default shortcut"),
            ],
            apply_heading1: vec![
                ShortcutBinding::parse("Ctrl+Alt+1").expect("valid default shortcut"),
                ShortcutBinding::parse("Meta+Alt+1").expect("valid default shortcut"),
            ],
            apply_heading2: vec![
                ShortcutBinding::parse("Ctrl+Alt+2").expect("valid default shortcut"),
                ShortcutBinding::parse("Meta+Alt+2").expect("valid default shortcut"),
            ],
            apply_heading3: vec![
                ShortcutBinding::parse("Ctrl+Alt+3").expect("valid default shortcut"),
                ShortcutBinding::parse("Meta+Alt+3").expect("valid default shortcut"),
            ],
            insert_non_breaking_space: vec![
                ShortcutBinding::parse("Ctrl+Shift+Space").expect("valid default shortcut")
            ],
            insert_non_breaking_hyphen: vec![
                ShortcutBinding::parse("Ctrl+Shift+_").expect("valid default shortcut"),
                ShortcutBinding::parse("Meta+Shift+_").expect("valid default shortcut"),
            ],
            insert_copyright: vec![
                ShortcutBinding::parse("Ctrl+Alt+C").expect("valid default shortcut")
            ],
            insert_registered: vec![
                ShortcutBinding::parse("Ctrl+Alt+R").expect("valid default shortcut")
            ],
            insert_trademark: vec![
                ShortcutBinding::parse("Ctrl+Alt+T").expect("valid default shortcut")
            ],
            insert_ellipsis: vec![
                ShortcutBinding::parse("Ctrl+Alt+.").expect("valid default shortcut")
            ],
        }
    }
}

impl ShortcutConfig {
    pub fn with_delete_backward(mut self, bindings: Vec<ShortcutBinding>) -> Self {
        if !bindings.is_empty() {
            self.delete_backward = bindings;
        }
        self
    }

    pub fn with_delete_forward(mut self, bindings: Vec<ShortcutBinding>) -> Self {
        if !bindings.is_empty() {
            self.delete_forward = bindings;
        }
        self
    }

    pub fn with_undo(mut self, bindings: Vec<ShortcutBinding>) -> Self {
        if !bindings.is_empty() {
            self.undo = bindings;
        }
        self
    }

    pub fn with_redo(mut self, bindings: Vec<ShortcutBinding>) -> Self {
        if !bindings.is_empty() {
            self.redo = bindings;
        }
        self
    }

    pub fn with_copy(mut self, bindings: Vec<ShortcutBinding>) -> Self {
        if !bindings.is_empty() {
            self.copy = bindings;
        }
        self
    }

    pub fn with_paste(mut self, bindings: Vec<ShortcutBinding>) -> Self {
        if !bindings.is_empty() {
            self.paste = bindings;
        }
        self
    }

    pub fn with_cut(mut self, bindings: Vec<ShortcutBinding>) -> Self {
        if !bindings.is_empty() {
            self.cut = bindings;
        }
        self
    }

    pub fn with_toggle_bold(mut self, bindings: Vec<ShortcutBinding>) -> Self {
        if !bindings.is_empty() {
            self.toggle_bold = bindings;
        }
        self
    }

    pub fn with_toggle_italic(mut self, bindings: Vec<ShortcutBinding>) -> Self {
        if !bindings.is_empty() {
            self.toggle_italic = bindings;
        }
        self
    }

    pub fn with_toggle_underline(mut self, bindings: Vec<ShortcutBinding>) -> Self {
        if !bindings.is_empty() {
            self.toggle_underline = bindings;
        }
        self
    }

    pub fn with_toggle_word_underline(mut self, bindings: Vec<ShortcutBinding>) -> Self {
        if !bindings.is_empty() {
            self.toggle_word_underline = bindings;
        }
        self
    }

    pub fn with_toggle_double_underline(mut self, bindings: Vec<ShortcutBinding>) -> Self {
        if !bindings.is_empty() {
            self.toggle_double_underline = bindings;
        }
        self
    }

    pub fn with_toggle_subscript(mut self, bindings: Vec<ShortcutBinding>) -> Self {
        if !bindings.is_empty() {
            self.toggle_subscript = bindings;
        }
        self
    }

    pub fn with_toggle_superscript(mut self, bindings: Vec<ShortcutBinding>) -> Self {
        if !bindings.is_empty() {
            self.toggle_superscript = bindings;
        }
        self
    }

    pub fn with_clear_formatting(mut self, bindings: Vec<ShortcutBinding>) -> Self {
        if !bindings.is_empty() {
            self.clear_formatting = bindings;
        }
        self
    }

    pub fn with_increase_font_size(mut self, bindings: Vec<ShortcutBinding>) -> Self {
        if !bindings.is_empty() {
            self.increase_font_size = bindings;
        }
        self
    }

    pub fn with_decrease_font_size(mut self, bindings: Vec<ShortcutBinding>) -> Self {
        if !bindings.is_empty() {
            self.decrease_font_size = bindings;
        }
        self
    }

    pub fn with_align_left(mut self, bindings: Vec<ShortcutBinding>) -> Self {
        if !bindings.is_empty() {
            self.align_left = bindings;
        }
        self
    }

    pub fn with_align_center(mut self, bindings: Vec<ShortcutBinding>) -> Self {
        if !bindings.is_empty() {
            self.align_center = bindings;
        }
        self
    }

    pub fn with_align_right(mut self, bindings: Vec<ShortcutBinding>) -> Self {
        if !bindings.is_empty() {
            self.align_right = bindings;
        }
        self
    }

    pub fn with_align_justify(mut self, bindings: Vec<ShortcutBinding>) -> Self {
        if !bindings.is_empty() {
            self.align_justify = bindings;
        }
        self
    }

    pub fn with_increase_indent(mut self, bindings: Vec<ShortcutBinding>) -> Self {
        if !bindings.is_empty() {
            self.increase_indent = bindings;
        }
        self
    }

    pub fn with_decrease_indent(mut self, bindings: Vec<ShortcutBinding>) -> Self {
        if !bindings.is_empty() {
            self.decrease_indent = bindings;
        }
        self
    }

    pub fn with_reset_paragraph_formatting(mut self, bindings: Vec<ShortcutBinding>) -> Self {
        if !bindings.is_empty() {
            self.reset_paragraph_formatting = bindings;
        }
        self
    }

    pub fn with_increase_hanging_indent(mut self, bindings: Vec<ShortcutBinding>) -> Self {
        if !bindings.is_empty() {
            self.increase_hanging_indent = bindings;
        }
        self
    }

    pub fn with_decrease_hanging_indent(mut self, bindings: Vec<ShortcutBinding>) -> Self {
        if !bindings.is_empty() {
            self.decrease_hanging_indent = bindings;
        }
        self
    }

    pub fn with_set_line_spacing_single(mut self, bindings: Vec<ShortcutBinding>) -> Self {
        if !bindings.is_empty() {
            self.set_line_spacing_single = bindings;
        }
        self
    }

    pub fn with_set_line_spacing_one_and_half(mut self, bindings: Vec<ShortcutBinding>) -> Self {
        if !bindings.is_empty() {
            self.set_line_spacing_one_and_half = bindings;
        }
        self
    }

    pub fn with_set_line_spacing_double(mut self, bindings: Vec<ShortcutBinding>) -> Self {
        if !bindings.is_empty() {
            self.set_line_spacing_double = bindings;
        }
        self
    }

    pub fn with_toggle_space_before(mut self, bindings: Vec<ShortcutBinding>) -> Self {
        if !bindings.is_empty() {
            self.toggle_space_before = bindings;
        }
        self
    }

    pub fn with_increase_font_size_by_point(mut self, bindings: Vec<ShortcutBinding>) -> Self {
        if !bindings.is_empty() {
            self.increase_font_size_by_point = bindings;
        }
        self
    }

    pub fn with_decrease_font_size_by_point(mut self, bindings: Vec<ShortcutBinding>) -> Self {
        if !bindings.is_empty() {
            self.decrease_font_size_by_point = bindings;
        }
        self
    }

    pub fn with_insert_page_break(mut self, bindings: Vec<ShortcutBinding>) -> Self {
        if !bindings.is_empty() {
            self.insert_page_break = bindings;
        }
        self
    }

    pub fn with_toggle_bullet_list(mut self, bindings: Vec<ShortcutBinding>) -> Self {
        if !bindings.is_empty() {
            self.toggle_bullet_list = bindings;
        }
        self
    }

    /// Override the bindings that move the caret to the paragraph start
    /// (e.g. Emacs-style `Ctrl+A`). Pass an empty Vec to clear.
    pub fn with_move_paragraph_start(mut self, bindings: Vec<ShortcutBinding>) -> Self {
        self.move_paragraph_start = bindings;
        self
    }

    /// Override the bindings that move the caret to the paragraph end
    /// (e.g. Emacs-style `Ctrl+E`). Pass an empty Vec to clear.
    pub fn with_move_paragraph_end(mut self, bindings: Vec<ShortcutBinding>) -> Self {
        self.move_paragraph_end = bindings;
        self
    }

    pub fn delete_backward(&self) -> &[ShortcutBinding] {
        &self.delete_backward
    }

    pub fn delete_forward(&self) -> &[ShortcutBinding] {
        &self.delete_forward
    }

    /// Replaces one action's bindings, keeping the default when `bindings`
    /// is empty (the same rule as the `with_*` builders). Lets
    /// `Options::set_shortcuts` read every action by its `name()`.
    pub fn with_bindings(mut self, action: ShortcutAction, bindings: Vec<ShortcutBinding>) -> Self {
        if !bindings.is_empty() {
            *self.bindings_mut(action) = bindings;
        }
        self
    }

    fn bindings_mut(&mut self, action: ShortcutAction) -> &mut Vec<ShortcutBinding> {
        match action {
            ShortcutAction::DeleteBackward => &mut self.delete_backward,
            ShortcutAction::DeleteForward => &mut self.delete_forward,
            ShortcutAction::Undo => &mut self.undo,
            ShortcutAction::Redo => &mut self.redo,
            ShortcutAction::Copy => &mut self.copy,
            ShortcutAction::Paste => &mut self.paste,
            ShortcutAction::Cut => &mut self.cut,
            ShortcutAction::ToggleBold => &mut self.toggle_bold,
            ShortcutAction::ToggleItalic => &mut self.toggle_italic,
            ShortcutAction::ToggleUnderline => &mut self.toggle_underline,
            ShortcutAction::ToggleWordUnderline => &mut self.toggle_word_underline,
            ShortcutAction::ToggleDoubleUnderline => &mut self.toggle_double_underline,
            ShortcutAction::ToggleSubscript => &mut self.toggle_subscript,
            ShortcutAction::ToggleSuperscript => &mut self.toggle_superscript,
            ShortcutAction::ClearFormatting => &mut self.clear_formatting,
            ShortcutAction::IncreaseIndent => &mut self.increase_indent,
            ShortcutAction::DecreaseIndent => &mut self.decrease_indent,
            ShortcutAction::ResetParagraphFormatting => &mut self.reset_paragraph_formatting,
            ShortcutAction::IncreaseHangingIndent => &mut self.increase_hanging_indent,
            ShortcutAction::DecreaseHangingIndent => &mut self.decrease_hanging_indent,
            ShortcutAction::SetLineSpacingSingle => &mut self.set_line_spacing_single,
            ShortcutAction::SetLineSpacingOneAndHalf => &mut self.set_line_spacing_one_and_half,
            ShortcutAction::SetLineSpacingDouble => &mut self.set_line_spacing_double,
            ShortcutAction::ToggleSpaceBefore => &mut self.toggle_space_before,
            ShortcutAction::IncreaseFontSizeByPoint => &mut self.increase_font_size_by_point,
            ShortcutAction::DecreaseFontSizeByPoint => &mut self.decrease_font_size_by_point,
            ShortcutAction::InsertPageBreak => &mut self.insert_page_break,
            ShortcutAction::ToggleBulletList => &mut self.toggle_bullet_list,
            ShortcutAction::IncreaseFontSize => &mut self.increase_font_size,
            ShortcutAction::DecreaseFontSize => &mut self.decrease_font_size,
            ShortcutAction::AlignLeft => &mut self.align_left,
            ShortcutAction::AlignCenter => &mut self.align_center,
            ShortcutAction::AlignRight => &mut self.align_right,
            ShortcutAction::AlignJustify => &mut self.align_justify,
            ShortcutAction::MoveParagraphStart => &mut self.move_paragraph_start,
            ShortcutAction::MoveParagraphEnd => &mut self.move_paragraph_end,
            ShortcutAction::ChangeCase => &mut self.change_case,
            ShortcutAction::ApplyNormalStyle => &mut self.apply_normal_style,
            ShortcutAction::ApplyHeading1 => &mut self.apply_heading1,
            ShortcutAction::ApplyHeading2 => &mut self.apply_heading2,
            ShortcutAction::ApplyHeading3 => &mut self.apply_heading3,
            ShortcutAction::InsertNonBreakingSpace => &mut self.insert_non_breaking_space,
            ShortcutAction::InsertNonBreakingHyphen => &mut self.insert_non_breaking_hyphen,
            ShortcutAction::InsertCopyright => &mut self.insert_copyright,
            ShortcutAction::InsertRegistered => &mut self.insert_registered,
            ShortcutAction::InsertTrademark => &mut self.insert_trademark,
            ShortcutAction::InsertEllipsis => &mut self.insert_ellipsis,
        }
    }

    /// Every action paired with its effective bindings, in match priority
    /// order. `resolve_action` walks this table, and the browser runtime
    /// receives a copy of it so it can cancel the browser's own accelerator
    /// while the keydown is still being dispatched (see `shortcut_table`).
    pub fn binding_table(&self) -> Vec<(ShortcutAction, &[ShortcutBinding])> {
        vec![
            (
                ShortcutAction::DeleteBackward,
                self.delete_backward.as_slice(),
            ),
            (
                ShortcutAction::DeleteForward,
                self.delete_forward.as_slice(),
            ),
            (ShortcutAction::Undo, self.undo.as_slice()),
            (ShortcutAction::Redo, self.redo.as_slice()),
            (ShortcutAction::Copy, self.copy.as_slice()),
            (ShortcutAction::Paste, self.paste.as_slice()),
            (ShortcutAction::Cut, self.cut.as_slice()),
            (ShortcutAction::ToggleBold, self.toggle_bold.as_slice()),
            (ShortcutAction::ToggleItalic, self.toggle_italic.as_slice()),
            (
                ShortcutAction::ToggleUnderline,
                self.toggle_underline.as_slice(),
            ),
            (
                ShortcutAction::ToggleWordUnderline,
                self.toggle_word_underline.as_slice(),
            ),
            (
                ShortcutAction::ToggleDoubleUnderline,
                self.toggle_double_underline.as_slice(),
            ),
            (
                ShortcutAction::ToggleSubscript,
                self.toggle_subscript.as_slice(),
            ),
            (
                ShortcutAction::ToggleSuperscript,
                self.toggle_superscript.as_slice(),
            ),
            (
                ShortcutAction::ClearFormatting,
                self.clear_formatting.as_slice(),
            ),
            (
                ShortcutAction::IncreaseIndent,
                self.increase_indent.as_slice(),
            ),
            (
                ShortcutAction::DecreaseIndent,
                self.decrease_indent.as_slice(),
            ),
            (
                ShortcutAction::ResetParagraphFormatting,
                self.reset_paragraph_formatting.as_slice(),
            ),
            (
                ShortcutAction::IncreaseHangingIndent,
                self.increase_hanging_indent.as_slice(),
            ),
            (
                ShortcutAction::DecreaseHangingIndent,
                self.decrease_hanging_indent.as_slice(),
            ),
            (
                ShortcutAction::SetLineSpacingSingle,
                self.set_line_spacing_single.as_slice(),
            ),
            (
                ShortcutAction::SetLineSpacingOneAndHalf,
                self.set_line_spacing_one_and_half.as_slice(),
            ),
            (
                ShortcutAction::SetLineSpacingDouble,
                self.set_line_spacing_double.as_slice(),
            ),
            (
                ShortcutAction::ToggleSpaceBefore,
                self.toggle_space_before.as_slice(),
            ),
            (
                ShortcutAction::IncreaseFontSizeByPoint,
                self.increase_font_size_by_point.as_slice(),
            ),
            (
                ShortcutAction::DecreaseFontSizeByPoint,
                self.decrease_font_size_by_point.as_slice(),
            ),
            (
                ShortcutAction::InsertPageBreak,
                self.insert_page_break.as_slice(),
            ),
            (
                ShortcutAction::ToggleBulletList,
                self.toggle_bullet_list.as_slice(),
            ),
            (
                ShortcutAction::IncreaseFontSize,
                self.increase_font_size.as_slice(),
            ),
            (
                ShortcutAction::DecreaseFontSize,
                self.decrease_font_size.as_slice(),
            ),
            (ShortcutAction::AlignLeft, self.align_left.as_slice()),
            (ShortcutAction::AlignCenter, self.align_center.as_slice()),
            (ShortcutAction::AlignRight, self.align_right.as_slice()),
            (ShortcutAction::AlignJustify, self.align_justify.as_slice()),
            (
                ShortcutAction::MoveParagraphStart,
                self.move_paragraph_start.as_slice(),
            ),
            (
                ShortcutAction::MoveParagraphEnd,
                self.move_paragraph_end.as_slice(),
            ),
            (ShortcutAction::ChangeCase, self.change_case.as_slice()),
            (
                ShortcutAction::ApplyNormalStyle,
                self.apply_normal_style.as_slice(),
            ),
            (
                ShortcutAction::ApplyHeading1,
                self.apply_heading1.as_slice(),
            ),
            (
                ShortcutAction::ApplyHeading2,
                self.apply_heading2.as_slice(),
            ),
            (
                ShortcutAction::ApplyHeading3,
                self.apply_heading3.as_slice(),
            ),
            (
                ShortcutAction::InsertNonBreakingSpace,
                self.insert_non_breaking_space.as_slice(),
            ),
            (
                ShortcutAction::InsertNonBreakingHyphen,
                self.insert_non_breaking_hyphen.as_slice(),
            ),
            (
                ShortcutAction::InsertCopyright,
                self.insert_copyright.as_slice(),
            ),
            (
                ShortcutAction::InsertRegistered,
                self.insert_registered.as_slice(),
            ),
            (
                ShortcutAction::InsertTrademark,
                self.insert_trademark.as_slice(),
            ),
            (
                ShortcutAction::InsertEllipsis,
                self.insert_ellipsis.as_slice(),
            ),
        ]
    }

    pub fn resolve_action(
        &self,
        key: &str,
        ctrl: bool,
        alt: bool,
        shift: bool,
        meta: bool,
    ) -> Option<ShortcutAction> {
        self.binding_table()
            .into_iter()
            .find(|(_, bindings)| {
                bindings
                    .iter()
                    .any(|binding| binding.matches(key, ctrl, alt, shift, meta))
            })
            .map(|(action, _)| action)
    }
}

fn normalize_key(key: &str) -> String {
    if key == " " || key.eq_ignore_ascii_case("spacebar") {
        return "space".to_string();
    }
    match key.trim() {
        // Browser KeyboardEvent.key reports Shift+= as `+`, while Word's
        // accelerator notation and our public shortcut API spell it as `=`.
        "+" => "=".to_string(),
        key => key.to_ascii_lowercase(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn binding_table_lists_every_action_once_under_a_unique_name() {
        // The browser runtime cancels bound chords from a main-thread copy of
        // this table, so an action missing here would let the browser's own
        // accelerator run alongside the edit again.
        let cfg = ShortcutConfig::default();
        let table = cfg.binding_table();
        let names: std::collections::HashSet<_> =
            table.iter().map(|(action, _)| action.name()).collect();
        assert_eq!(names.len(), table.len());
        for (action, bindings) in &table {
            for binding in *bindings {
                assert_eq!(
                    cfg.resolve_action(
                        binding.key(),
                        binding.ctrl(),
                        binding.alt(),
                        binding.shift(),
                        binding.meta()
                    ),
                    Some(*action),
                    "{binding} must resolve to {}",
                    action.name()
                );
            }
        }
    }

    #[test]
    fn resolves_word_style_symbol_and_case_shortcuts() {
        let cfg = ShortcutConfig::default();
        let cases = [
            ("F3", false, false, true, false, ShortcutAction::ChangeCase),
            (
                "n",
                true,
                false,
                true,
                false,
                ShortcutAction::ApplyNormalStyle,
            ),
            ("1", true, true, false, false, ShortcutAction::ApplyHeading1),
            ("2", true, true, false, false, ShortcutAction::ApplyHeading2),
            ("3", true, true, false, false, ShortcutAction::ApplyHeading3),
            // Word for Mac: ⌘⌥1-3.
            ("1", false, true, false, true, ShortcutAction::ApplyHeading1),
            (
                " ",
                true,
                false,
                true,
                false,
                ShortcutAction::InsertNonBreakingSpace,
            ),
            // Shift+- reports `_` on US layouts.
            (
                "_",
                true,
                false,
                true,
                false,
                ShortcutAction::InsertNonBreakingHyphen,
            ),
            (
                "c",
                true,
                true,
                false,
                false,
                ShortcutAction::InsertCopyright,
            ),
            (
                "r",
                true,
                true,
                false,
                false,
                ShortcutAction::InsertRegistered,
            ),
            (
                "t",
                true,
                true,
                false,
                false,
                ShortcutAction::InsertTrademark,
            ),
            (
                ".",
                true,
                true,
                false,
                false,
                ShortcutAction::InsertEllipsis,
            ),
            // Word for Mac redoes with ⌘Y.
            ("y", false, false, false, true, ShortcutAction::Redo),
        ];
        for (key, ctrl, alt, shift, meta, expected) in cases {
            assert_eq!(
                cfg.resolve_action(key, ctrl, alt, shift, meta),
                Some(expected),
                "{key} ctrl={ctrl} alt={alt} shift={shift} meta={meta}"
            );
        }
        // The new chords must not shadow the existing plain ones.
        assert_eq!(
            cfg.resolve_action("1", true, false, false, false),
            Some(ShortcutAction::SetLineSpacingSingle)
        );
        assert_eq!(
            cfg.resolve_action("r", true, false, false, false),
            Some(ShortcutAction::AlignRight)
        );
        assert_eq!(cfg.resolve_action("F3", false, false, false, false), None);
    }

    #[test]
    fn word_extras_accept_overrides_by_name() {
        let cfg = ShortcutConfig::default().with_bindings(
            ShortcutAction::ApplyHeading1,
            vec![ShortcutBinding::parse("Ctrl+Shift+1").unwrap()],
        );
        assert_eq!(cfg.resolve_action("1", true, true, false, false), None);
        assert_eq!(
            cfg.resolve_action("1", true, false, true, false),
            Some(ShortcutAction::ApplyHeading1)
        );
        // An empty override keeps the default, like the per-field builders.
        let cfg = ShortcutConfig::default().with_bindings(ShortcutAction::ChangeCase, Vec::new());
        assert_eq!(
            cfg.resolve_action("F3", false, false, true, false),
            Some(ShortcutAction::ChangeCase)
        );
    }

    #[test]
    fn parses_default_bindings() {
        let cfg = ShortcutConfig::default();
        assert!(cfg
            .resolve_action("Backspace", false, false, false, false)
            .is_some());
        assert!(cfg
            .resolve_action("Delete", false, false, false, false)
            .is_some());
        assert_eq!(
            cfg.resolve_action("h", true, false, false, false),
            Some(ShortcutAction::DeleteBackward)
        );
        assert_eq!(
            cfg.resolve_action("d", true, false, false, false),
            Some(ShortcutAction::DeleteForward)
        );
        assert_eq!(
            cfg.resolve_action("z", true, false, false, false),
            Some(ShortcutAction::Undo)
        );
        assert_eq!(
            cfg.resolve_action("z", false, false, false, true),
            Some(ShortcutAction::Undo)
        );
        assert_eq!(
            cfg.resolve_action("z", true, false, true, false),
            Some(ShortcutAction::Redo)
        );
        assert_eq!(
            cfg.resolve_action("z", false, false, true, true),
            Some(ShortcutAction::Redo)
        );
        assert_eq!(
            cfg.resolve_action("y", true, false, false, false),
            Some(ShortcutAction::Redo)
        );
        assert_eq!(
            cfg.resolve_action("c", true, false, false, false),
            Some(ShortcutAction::Copy)
        );
        assert_eq!(
            cfg.resolve_action("c", false, false, false, true),
            Some(ShortcutAction::Copy)
        );
        assert_eq!(
            cfg.resolve_action("v", true, false, false, false),
            Some(ShortcutAction::Paste)
        );
        assert_eq!(
            cfg.resolve_action("v", false, false, false, true),
            Some(ShortcutAction::Paste)
        );
        assert_eq!(
            cfg.resolve_action("x", true, false, false, false),
            Some(ShortcutAction::Cut)
        );
        assert_eq!(
            cfg.resolve_action("x", false, false, false, true),
            Some(ShortcutAction::Cut)
        );
    }

    #[test]
    fn resolves_word_paragraph_shortcuts() {
        let cfg = ShortcutConfig::default();
        // Word: Ctrl+M steps the indent (list level on list items) and
        // Ctrl+Shift+M steps it back; Ctrl+Q strips direct paragraph
        // formatting. Meta variants are deliberately absent: ⌘M minimizes
        // the window and ⌘Q quits the browser.
        assert_eq!(
            cfg.resolve_action("m", true, false, false, false),
            Some(ShortcutAction::IncreaseIndent)
        );
        assert_eq!(
            cfg.resolve_action("m", true, false, true, false),
            Some(ShortcutAction::DecreaseIndent)
        );
        assert_eq!(
            cfg.resolve_action("q", true, false, false, false),
            Some(ShortcutAction::ResetParagraphFormatting)
        );
        assert_eq!(cfg.resolve_action("m", false, false, false, true), None);
        assert_eq!(cfg.resolve_action("q", false, false, false, true), None);

        // Word: Ctrl+T / Ctrl+Shift+T step the hanging indent, Ctrl+1/2/5
        // set single/double/1.5 line spacing, Ctrl+0 toggles 12pt before.
        // Meta variants stay absent: ⌘T opens a tab, ⌘0 resets zoom and
        // ⌘1/2/5 switch tabs.
        assert_eq!(
            cfg.resolve_action("t", true, false, false, false),
            Some(ShortcutAction::IncreaseHangingIndent)
        );
        assert_eq!(
            cfg.resolve_action("t", true, false, true, false),
            Some(ShortcutAction::DecreaseHangingIndent)
        );
        assert_eq!(
            cfg.resolve_action("1", true, false, false, false),
            Some(ShortcutAction::SetLineSpacingSingle)
        );
        assert_eq!(
            cfg.resolve_action("2", true, false, false, false),
            Some(ShortcutAction::SetLineSpacingDouble)
        );
        assert_eq!(
            cfg.resolve_action("5", true, false, false, false),
            Some(ShortcutAction::SetLineSpacingOneAndHalf)
        );
        assert_eq!(
            cfg.resolve_action("0", true, false, false, false),
            Some(ShortcutAction::ToggleSpaceBefore)
        );
        for key in ["t", "0", "1", "2", "5"] {
            assert_eq!(cfg.resolve_action(key, false, false, false, true), None);
        }
    }

    #[test]
    fn resolves_word_point_step_break_and_bullet_shortcuts() {
        let cfg = ShortcutConfig::default();
        // Word: Ctrl+] / Ctrl+[ grow and shrink by exactly one point (unlike
        // the ribbon ladder on Ctrl+Shift+> / <), Ctrl+Enter inserts a page
        // break, Ctrl+Shift+L toggles bullets. ⌘[ / ⌘] are browser history
        // navigation, so no Meta variants.
        assert_eq!(
            cfg.resolve_action("]", true, false, false, false),
            Some(ShortcutAction::IncreaseFontSizeByPoint)
        );
        assert_eq!(
            cfg.resolve_action("[", true, false, false, false),
            Some(ShortcutAction::DecreaseFontSizeByPoint)
        );
        assert_eq!(
            cfg.resolve_action("Enter", true, false, false, false),
            Some(ShortcutAction::InsertPageBreak)
        );
        assert_eq!(
            cfg.resolve_action("Enter", false, false, false, false),
            None
        );
        assert_eq!(cfg.resolve_action("Enter", false, false, true, false), None);
        assert_eq!(
            cfg.resolve_action("l", true, false, true, false),
            Some(ShortcutAction::ToggleBulletList)
        );
        for key in ["]", "["] {
            assert_eq!(cfg.resolve_action(key, false, false, false, true), None);
        }
    }

    #[test]
    fn resolves_word_character_formatting_shortcuts() {
        let cfg = ShortcutConfig::default();
        for (key, action) in [
            ("b", ShortcutAction::ToggleBold),
            ("i", ShortcutAction::ToggleItalic),
            ("u", ShortcutAction::ToggleUnderline),
        ] {
            assert_eq!(
                cfg.resolve_action(key, true, false, false, false),
                Some(action)
            );
            assert_eq!(
                cfg.resolve_action(key, false, false, false, true),
                Some(action)
            );
        }

        for (key, ctrl, shift, meta, action) in [
            ("=", true, false, false, ShortcutAction::ToggleSubscript),
            ("=", false, false, true, ShortcutAction::ToggleSubscript),
            ("+", true, true, false, ShortcutAction::ToggleSuperscript),
            ("+", false, true, true, ShortcutAction::ToggleSuperscript),
        ] {
            assert_eq!(
                cfg.resolve_action(key, ctrl, false, shift, meta),
                Some(action)
            );
        }

        for (key, ctrl, meta, action) in [
            ("w", true, false, ShortcutAction::ToggleWordUnderline),
            ("w", false, true, ShortcutAction::ToggleWordUnderline),
            ("d", true, false, ShortcutAction::ToggleDoubleUnderline),
            ("d", false, true, ShortcutAction::ToggleDoubleUnderline),
        ] {
            assert_eq!(
                cfg.resolve_action(key, ctrl, false, true, meta),
                Some(action)
            );
        }

        assert_eq!(
            cfg.resolve_action(" ", true, false, false, false),
            Some(ShortcutAction::ClearFormatting)
        );
        assert_eq!(cfg.resolve_action(" ", false, false, false, true), None);
        for (key, ctrl, meta, action) in [
            (">", true, false, ShortcutAction::IncreaseFontSize),
            (">", false, true, ShortcutAction::IncreaseFontSize),
            ("<", true, false, ShortcutAction::DecreaseFontSize),
            ("<", false, true, ShortcutAction::DecreaseFontSize),
            (".", true, false, ShortcutAction::IncreaseFontSize),
            (",", true, false, ShortcutAction::DecreaseFontSize),
        ] {
            assert_eq!(
                cfg.resolve_action(key, ctrl, false, true, meta),
                Some(action)
            );
        }
        for (key, ctrl, meta, action) in [
            ("l", true, false, ShortcutAction::AlignLeft),
            ("l", false, true, ShortcutAction::AlignLeft),
            ("e", true, false, ShortcutAction::AlignCenter),
            ("e", false, true, ShortcutAction::AlignCenter),
            ("r", true, false, ShortcutAction::AlignRight),
            ("r", false, true, ShortcutAction::AlignRight),
            ("j", true, false, ShortcutAction::AlignJustify),
            ("j", false, true, ShortcutAction::AlignJustify),
        ] {
            assert_eq!(
                cfg.resolve_action(key, ctrl, false, false, meta),
                Some(action)
            );
        }
    }

    #[test]
    fn custom_copy_and_paste_bindings_override_defaults() {
        let cfg = ShortcutConfig::default()
            .with_copy(vec![ShortcutBinding::parse("Alt+C").unwrap()])
            .with_paste(vec![ShortcutBinding::parse("Alt+V").unwrap()]);

        assert_eq!(
            cfg.resolve_action("c", false, true, false, false),
            Some(ShortcutAction::Copy)
        );
        assert_eq!(
            cfg.resolve_action("v", false, true, false, false),
            Some(ShortcutAction::Paste)
        );
        assert_eq!(cfg.resolve_action("c", true, false, false, false), None);
        assert_eq!(cfg.resolve_action("v", true, false, false, false), None);
    }

    #[test]
    fn parse_rejects_missing_key() {
        assert!(ShortcutBinding::parse("Ctrl+").is_err());
        assert!(ShortcutBinding::parse("Shift+ ").is_err());
    }

    #[test]
    fn parse_rejects_multiple_primary_keys() {
        assert!(ShortcutBinding::parse("Ctrl+H+J").is_err());
    }

    #[test]
    fn matching_requires_exact_modifiers() {
        let binding = ShortcutBinding::parse("Ctrl+H").unwrap();
        assert!(binding.matches("H", true, false, false, false));
        assert!(!binding.matches("H", false, false, false, false));
        assert!(!binding.matches("H", true, false, true, false));
    }

    #[test]
    fn move_paragraph_bindings_default_empty_but_resolve_when_overridden() {
        let cfg = ShortcutConfig::default();
        // Defaults are empty so apps don't accidentally hijack browser Ctrl+A.
        assert_eq!(cfg.resolve_action("a", true, false, false, false), None);
        assert_eq!(cfg.resolve_action("k", true, false, false, false), None);
        assert_eq!(
            cfg.resolve_action("e", true, false, false, false),
            Some(ShortcutAction::AlignCenter)
        );

        let cfg = cfg
            .with_move_paragraph_start(vec![
                ShortcutBinding::parse("Ctrl+A").unwrap(),
                ShortcutBinding::parse("Ctrl+Shift+A").unwrap(),
            ])
            .with_move_paragraph_end(vec![
                ShortcutBinding::parse("Ctrl+K").unwrap(),
                ShortcutBinding::parse("Ctrl+Shift+K").unwrap(),
            ]);

        assert_eq!(
            cfg.resolve_action("a", true, false, false, false),
            Some(ShortcutAction::MoveParagraphStart)
        );
        assert_eq!(
            cfg.resolve_action("a", true, false, true, false),
            Some(ShortcutAction::MoveParagraphStart)
        );
        assert_eq!(
            cfg.resolve_action("k", true, false, false, false),
            Some(ShortcutAction::MoveParagraphEnd)
        );
        assert_eq!(
            cfg.resolve_action("k", true, false, true, false),
            Some(ShortcutAction::MoveParagraphEnd)
        );
    }

    #[test]
    fn respects_custom_undo_redo_bindings() {
        let undo = ShortcutBinding::parse("Alt+U").unwrap();
        let redo = ShortcutBinding::parse("Alt+Shift+U").unwrap();

        let cfg = ShortcutConfig::default()
            .with_undo(vec![undo.clone()])
            .with_redo(vec![redo.clone()]);

        assert_eq!(
            cfg.resolve_action("u", false, true, false, false),
            Some(ShortcutAction::Undo)
        );
        assert_eq!(
            cfg.resolve_action("u", false, true, true, false),
            Some(ShortcutAction::Redo)
        );

        // Default bindings should no longer match when custom overrides are supplied.
        assert_eq!(cfg.resolve_action("z", true, false, false, false), None);
        assert_eq!(cfg.resolve_action("z", true, false, true, false), None);
    }
}
