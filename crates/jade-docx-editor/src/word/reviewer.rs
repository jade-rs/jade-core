//! Who is reviewing: Word's ユーザー情報 (環境設定 › ユーザー情報) — the name
//! and initials every comment, reply and tracked change is put down to. Both
//! shells keep one [`Reviewer`], read from and saved to their own settings,
//! and edit it in a [`ReviewerForm`].

use crate::text_edit::TextEdit;

/// The name and initials review work is recorded under.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Reviewer {
    pub name: String,
    pub initials: String,
}

impl Reviewer {
    /// `name`, with the initials Word would suggest for it.
    pub fn named(name: &str) -> Self {
        Self {
            name: name.trim().to_string(),
            initials: initials_of(name),
        }
    }

    /// `name` and `initials`; blank initials are the name's own.
    pub fn new(name: &str, initials: &str) -> Self {
        let mut reviewer = Self::named(name);
        if !initials.trim().is_empty() {
            reviewer.initials = initials.trim().to_string();
        }
        reviewer
    }

    /// Initials, or `None` when there are none to record.
    pub fn initials(&self) -> Option<String> {
        (!self.initials.is_empty()).then(|| self.initials.clone())
    }

    /// The settings file's text: `name=` and `initials=` lines.
    pub fn to_settings(&self) -> String {
        format!("name={}\ninitials={}\n", self.name, self.initials)
    }

    /// A reviewer from [`Self::to_settings`]'s text; `None` without a name.
    pub fn from_settings(text: &str) -> Option<Self> {
        let mut name = None;
        let mut initials = String::new();
        for line in text.lines() {
            if let Some(value) = line.strip_prefix("name=") {
                name = Some(value.trim().to_string());
            } else if let Some(value) = line.strip_prefix("initials=") {
                initials = value.trim().to_string();
            }
        }
        name.filter(|name| !name.is_empty())
            .map(|name| Self::new(&name, &initials))
    }
}

/// The initials Word suggests for `name`: the first letter of each word,
/// upper-cased — "Ann Lee" is "AL" — or, for a name with no spaces to
/// break on (山田太郎), its first character.
pub fn initials_of(name: &str) -> String {
    let words: Vec<&str> = name.split_whitespace().collect();
    match words.as_slice() {
        [] => String::new(),
        [only] if !only.is_ascii() => only.chars().take(1).collect(),
        words => words
            .iter()
            .filter_map(|word| word.chars().next())
            .flat_map(char::to_uppercase)
            .collect(),
    }
}

/// Which box of the ユーザー情報 dialog has the keyboard.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReviewerField {
    Name,
    Initials,
}

/// The ユーザー情報 dialog: 姓名 and イニシャル.
pub struct ReviewerForm {
    pub name: TextEdit,
    pub initials: TextEdit,
    pub focus: ReviewerField,
    /// Whether the initials were typed, rather than following the name.
    initials_typed: bool,
}

impl ReviewerForm {
    pub fn new(reviewer: &Reviewer) -> Self {
        Self {
            name: TextEdit::selected(reviewer.name.clone()),
            initials: TextEdit::new(reviewer.initials.clone()),
            focus: ReviewerField::Name,
            initials_typed: reviewer.initials != initials_of(&reviewer.name),
        }
    }

    pub fn focused(&self) -> &TextEdit {
        match self.focus {
            ReviewerField::Name => &self.name,
            ReviewerField::Initials => &self.initials,
        }
    }

    pub fn focused_mut(&mut self) -> &mut TextEdit {
        if self.focus == ReviewerField::Initials {
            self.initials_typed = true;
        }
        match self.focus {
            ReviewerField::Name => &mut self.name,
            ReviewerField::Initials => &mut self.initials,
        }
    }

    /// After the name changed: the initials follow it until they are typed.
    pub fn name_changed(&mut self) {
        if !self.initials_typed {
            self.initials = TextEdit::new(initials_of(&self.name.text));
        }
    }

    /// Tab: the other box.
    pub fn next_field(&mut self) {
        self.focus = match self.focus {
            ReviewerField::Name => ReviewerField::Initials,
            ReviewerField::Initials => ReviewerField::Name,
        };
    }

    /// OK: the reviewer the boxes say, or `None` while the name is blank.
    /// Initials never typed are the name's, whatever the box last showed.
    pub fn reviewer(&self) -> Option<Reviewer> {
        let name = self.name.text.trim();
        let initials = if self.initials_typed {
            self.initials.text.clone()
        } else {
            initials_of(name)
        };
        (!name.is_empty()).then(|| Reviewer::new(name, &initials))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn initials_follow_the_name_until_typed() {
        assert_eq!(initials_of("Ann lee"), "AL");
        assert_eq!(initials_of("山田太郎"), "山");
        assert_eq!(initials_of("  "), "");
        let mut form = ReviewerForm::new(&Reviewer::named("jade"));
        form.focused_mut().insert("Mary Major");
        form.name_changed();
        assert_eq!(form.initials.text, "MM");
        form.next_field();
        form.focused_mut().insert("X");
        form.next_field();
        form.focused_mut().insert("!");
        form.name_changed();
        assert_eq!(form.reviewer(), Some(Reviewer::new("Mary Major!", "MMX")));
    }

    #[test]
    fn settings_round_trip() {
        let reviewer = Reviewer::new("Ann Lee", "AJL");
        assert_eq!(
            Reviewer::from_settings(&reviewer.to_settings()),
            Some(reviewer)
        );
        assert_eq!(Reviewer::from_settings("initials=X\n"), None);
    }
}
