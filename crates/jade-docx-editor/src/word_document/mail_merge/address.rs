//! Word's address fields — the names a recipient list's columns are
//! matched to (フィールドの対応) — and the 住所ブロック and あいさつ文 that
//! are built from them (`ADDRESSBLOCK`, `GREETINGLINE`, §17.16.5.2 and
//! §17.16.5.28), with their `\f` formats of `<<…>>` groups.

use super::MergeData;

/// One of Word's address fields (the ODSO mapped names).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum AddressField {
    CourtesyTitle,
    FirstName,
    MiddleName,
    LastName,
    Suffix,
    Nickname,
    JobTitle,
    Company,
    Department,
    Address1,
    Address2,
    City,
    State,
    PostalCode,
    Country,
    Phone,
    Email,
}

impl AddressField {
    pub const ALL: [AddressField; 17] = [
        AddressField::CourtesyTitle,
        AddressField::FirstName,
        AddressField::MiddleName,
        AddressField::LastName,
        AddressField::Suffix,
        AddressField::Nickname,
        AddressField::JobTitle,
        AddressField::Company,
        AddressField::Department,
        AddressField::Address1,
        AddressField::Address2,
        AddressField::City,
        AddressField::State,
        AddressField::PostalCode,
        AddressField::Country,
        AddressField::Phone,
        AddressField::Email,
    ];

    /// The name Word keeps the mapping under (`w:mappedName`).
    pub fn odso_name(self) -> &'static str {
        match self {
            AddressField::CourtesyTitle => "Courtesy Title",
            AddressField::FirstName => "First Name",
            AddressField::MiddleName => "Middle Name",
            AddressField::LastName => "Last Name",
            AddressField::Suffix => "Suffix",
            AddressField::Nickname => "Nickname",
            AddressField::JobTitle => "Job Title",
            AddressField::Company => "Company",
            AddressField::Department => "Department",
            AddressField::Address1 => "Address 1",
            AddressField::Address2 => "Address 2",
            AddressField::City => "City",
            AddressField::State => "State",
            AddressField::PostalCode => "Postal Code",
            AddressField::Country => "Country or Region",
            AddressField::Phone => "Business Phone",
            AddressField::Email => "E-mail Address",
        }
    }

    pub fn from_odso(name: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|field| field.odso_name().eq_ignore_ascii_case(name.trim()))
    }

    /// The i18n key of its name in the フィールドの対応 dialog.
    pub fn label_key(self) -> &'static str {
        match self {
            AddressField::CourtesyTitle => "mail.field.title",
            AddressField::FirstName => "mail.field.first",
            AddressField::MiddleName => "mail.field.middle",
            AddressField::LastName => "mail.field.last",
            AddressField::Suffix => "mail.field.suffix",
            AddressField::Nickname => "mail.field.nickname",
            AddressField::JobTitle => "mail.field.job",
            AddressField::Company => "mail.field.company",
            AddressField::Department => "mail.field.department",
            AddressField::Address1 => "mail.field.address1",
            AddressField::Address2 => "mail.field.address2",
            AddressField::City => "mail.field.city",
            AddressField::State => "mail.field.state",
            AddressField::PostalCode => "mail.field.postal",
            AddressField::Country => "mail.field.country",
            AddressField::Phone => "mail.field.phone",
            AddressField::Email => "mail.field.email",
        }
    }

    /// The `<<…>>` token that stands for it in an address block's or a
    /// greeting line's format.
    pub fn token(self) -> Option<&'static str> {
        Some(match self {
            AddressField::CourtesyTitle => "_TITLE0_",
            AddressField::FirstName => "_FIRST0_",
            AddressField::MiddleName => "_MIDDLE0_",
            AddressField::LastName => "_LAST0_",
            AddressField::Suffix => "_SUFFIX0_",
            AddressField::Nickname => "_NICK0_",
            AddressField::JobTitle => "_JOBTITLE_",
            AddressField::Company => "_COMPANY_",
            AddressField::Department => "_DEPARTMENT_",
            AddressField::Address1 => "_STREET1_",
            AddressField::Address2 => "_STREET2_",
            AddressField::City => "_CITY_",
            AddressField::State => "_STATE_",
            AddressField::PostalCode => "_POSTAL_",
            AddressField::Country => "_COUNTRY_",
            AddressField::Phone | AddressField::Email => return None,
        })
    }

    /// Column names a list made in Word, Excel or by hand calls it, in
    /// Japanese and English: what 対応 matches by itself.
    fn synonyms(self) -> &'static [&'static str] {
        match self {
            AddressField::CourtesyTitle => &["敬称", "title", "courtesy title", "courtesytitle"],
            AddressField::FirstName => &["名", "名前", "first name", "firstname", "given name"],
            AddressField::MiddleName => &["ミドル ネーム", "middle name", "middlename"],
            AddressField::LastName => &[
                "姓",
                "名字",
                "苗字",
                "last name",
                "lastname",
                "surname",
                "family name",
            ],
            AddressField::Suffix => &["サフィックス", "suffix"],
            AddressField::Nickname => &["ニックネーム", "nickname"],
            AddressField::JobTitle => &["役職", "job title", "jobtitle"],
            AddressField::Company => &[
                "会社名",
                "会社",
                "勤務先",
                "company",
                "company name",
                "organization",
            ],
            AddressField::Department => &["部署", "部署名", "department"],
            AddressField::Address1 => &[
                "住所1",
                "住所 1",
                "住所",
                "番地",
                "address 1",
                "address1",
                "address",
                "street",
            ],
            AddressField::Address2 => {
                &["住所2", "住所 2", "建物名", "建物", "address 2", "address2"]
            }
            AddressField::City => &["市区町村", "市町村", "city"],
            AddressField::State => &["都道府県", "state", "province", "prefecture"],
            AddressField::PostalCode => &[
                "郵便番号",
                "〒",
                "postal code",
                "postalcode",
                "zip",
                "zip code",
                "postcode",
            ],
            AddressField::Country => &["国", "国/地域", "country", "country or region"],
            AddressField::Phone => &["電話", "電話番号", "phone", "business phone", "telephone"],
            AddressField::Email => &[
                "メール",
                "電子メール",
                "メール アドレス",
                "メールアドレス",
                "email",
                "e-mail",
                "e-mail address",
                "email address",
            ],
        }
    }
}

/// Which column each address field takes.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FieldMapping {
    columns: Vec<(AddressField, Option<usize>)>,
}

impl FieldMapping {
    /// Each field matched to the column whose name is one of its names.
    pub fn matched(data: &MergeData) -> Self {
        let normalize = |text: &str| text.trim().to_lowercase().replace(['_', '　'], " ");
        let columns = AddressField::ALL
            .into_iter()
            .map(|field| {
                let column = data.columns.iter().position(|column| {
                    let column = normalize(column);
                    field
                        .synonyms()
                        .iter()
                        .any(|synonym| normalize(synonym) == column)
                        || normalize(field.odso_name()) == column
                });
                (field, column)
            })
            .collect();
        Self { columns }
    }

    pub fn column(&self, field: AddressField) -> Option<usize> {
        self.columns
            .iter()
            .find(|(candidate, _)| *candidate == field)
            .and_then(|(_, column)| *column)
    }

    pub fn set(&mut self, field: AddressField, column: Option<usize>) {
        match self
            .columns
            .iter_mut()
            .find(|(candidate, _)| *candidate == field)
        {
            Some(entry) => entry.1 = column,
            None => self.columns.push((field, column)),
        }
    }

    /// The address field a format token stands for.
    fn field_of_token(token: &str) -> Option<AddressField> {
        AddressField::ALL
            .into_iter()
            .find(|field| field.token() == Some(token))
    }
}

/// How the recipient's name is written.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum NameFormat {
    /// 山田 太郎 (姓 名).
    #[default]
    LastFirst,
    /// 山田
    Last,
    /// Mr. Joshua Randall Jr.
    TitleFirstLastSuffix,
    /// Joshua Randall
    FirstLast,
    /// Joshua
    First,
    /// Mr. Randall
    TitleLast,
}

impl NameFormat {
    pub const ALL: [NameFormat; 6] = [
        NameFormat::LastFirst,
        NameFormat::Last,
        NameFormat::TitleFirstLastSuffix,
        NameFormat::FirstLast,
        NameFormat::First,
        NameFormat::TitleLast,
    ];

    /// How the format shows a sample name.
    pub fn sample(self, japanese: bool) -> &'static str {
        match (self, japanese) {
            (NameFormat::LastFirst, true) => "山田 太郎",
            (NameFormat::LastFirst, false) => "Randall Joshua",
            (NameFormat::Last, true) => "山田",
            (NameFormat::Last, false) => "Randall",
            (NameFormat::TitleFirstLastSuffix, _) => "Mr. Joshua Randall Jr.",
            (NameFormat::FirstLast, _) => "Joshua Randall",
            (NameFormat::First, _) => "Joshua",
            (NameFormat::TitleLast, _) => "Mr. Randall",
        }
    }

    fn groups(self) -> &'static str {
        match self {
            NameFormat::LastFirst => "<<_LAST0_>><< _FIRST0_>>",
            NameFormat::Last => "<<_LAST0_>>",
            NameFormat::TitleFirstLastSuffix => {
                "<<_TITLE0_ >><<_FIRST0_>><< _LAST0_>><< _SUFFIX0_>>"
            }
            NameFormat::FirstLast => "<<_FIRST0_>><< _LAST0_>>",
            NameFormat::First => "<<_FIRST0_>>",
            NameFormat::TitleLast => "<<_TITLE0_ >><<_LAST0_>>",
        }
    }
}

/// Whether an address block includes the country.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum CountryMode {
    /// 宛先の国/地域を常に含める.
    Always,
    /// 国/地域を含めない.
    #[default]
    Never,
    /// 以下の国/地域以外の場合のみ含める.
    ExceptFor(String),
}

/// 住所ブロックの挿入's choices.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AddressBlockOptions {
    /// 受信者名をこの形式で挿入する; `None` leaves the name out.
    pub name: Option<NameFormat>,
    /// The honorific after the name: 様, 殿, 御中, or nothing.
    pub honorific: String,
    /// 会社名を挿入する.
    pub company: bool,
    /// 住所を挿入する.
    pub postal_address: bool,
    pub country: CountryMode,
    /// 宛先の国/地域に従って住所の書式を設定する.
    pub by_country: bool,
    /// The Japanese order: postal code first, the name last.
    pub japanese: bool,
}

impl AddressBlockOptions {
    pub fn new(japanese: bool) -> Self {
        Self {
            name: Some(if japanese {
                NameFormat::LastFirst
            } else {
                NameFormat::TitleFirstLastSuffix
            }),
            honorific: if japanese {
                "様".to_string()
            } else {
                String::new()
            },
            company: true,
            postal_address: true,
            country: if japanese {
                CountryMode::ExceptFor("日本".to_string())
            } else {
                CountryMode::ExceptFor("United States".to_string())
            },
            by_country: false,
            japanese,
        }
    }

    /// The `\f` format of `<<…>>` groups, lines separated by `\n`.
    pub fn format(&self) -> String {
        let name = self
            .name
            .map(|name| {
                let mut groups = name.groups().to_string();
                if !self.honorific.is_empty() {
                    // The honorific goes with the last name group, so it
                    // drops out with an empty name.
                    let last = groups.rfind("<<").unwrap_or(0);
                    let close = groups[last..]
                        .find(">>")
                        .map(|at| last + at)
                        .unwrap_or(groups.len());
                    groups.insert_str(close, &format!(" {}", self.honorific));
                }
                groups
            })
            .unwrap_or_default();
        let mut lines: Vec<String> = Vec::new();
        if self.japanese {
            if self.postal_address {
                lines.push("<<〒_POSTAL_>>".to_string());
                lines.push("<<_STATE_>><<_CITY_>><<_STREET1_>>".to_string());
                lines.push("<<_STREET2_>>".to_string());
            }
            if self.company {
                lines.push("<<_COMPANY_>><< _DEPARTMENT_>>".to_string());
            }
            if !name.is_empty() {
                lines.push(name);
            }
            if self.postal_address && self.country != CountryMode::Never {
                lines.push("<<_COUNTRY_>>".to_string());
            }
        } else {
            if !name.is_empty() {
                lines.push(name);
            }
            if self.company {
                lines.push("<<_COMPANY_>>".to_string());
            }
            if self.postal_address {
                lines.push("<<_STREET1_>>".to_string());
                lines.push("<<_STREET2_>>".to_string());
                lines.push("<<_CITY_>><<, _STATE_>><< _POSTAL_>>".to_string());
                if self.country != CountryMode::Never {
                    lines.push("<<_COUNTRY_>>".to_string());
                }
            }
        }
        lines.join("\n")
    }

    /// The field's instruction: `ADDRESSBLOCK \f "…" \l … \c … \e "…" \d`.
    pub fn instruction(&self) -> String {
        let country = match &self.country {
            CountryMode::Always => "1".to_string(),
            CountryMode::Never => "0".to_string(),
            CountryMode::ExceptFor(_) => "2".to_string(),
        };
        let mut instruction = format!(
            "ADDRESSBLOCK \\f \"{}\" \\l {} \\c {country}",
            self.format().replace('"', "\\\""),
            if self.japanese { 1041 } else { 1033 }
        );
        if let CountryMode::ExceptFor(except) = &self.country {
            instruction.push_str(&format!(" \\e \"{}\"", except.replace('"', "\\\"")));
        }
        if self.by_country {
            instruction.push_str(" \\d");
        }
        instruction
    }
}

/// あいさつ文の挿入's choices.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GreetingLineOptions {
    /// What comes before the name: Dear, To, 拝啓, or nothing.
    pub before: String,
    pub name: NameFormat,
    /// What comes after: `,`, `:`, 様, or nothing.
    pub after: String,
    /// 無効な受信者名に使用するあいさつ文.
    pub invalid: String,
    pub japanese: bool,
}

impl GreetingLineOptions {
    pub fn new(japanese: bool) -> Self {
        if japanese {
            Self {
                before: String::new(),
                name: NameFormat::LastFirst,
                after: "様".to_string(),
                invalid: "お客様各位".to_string(),
                japanese,
            }
        } else {
            Self {
                before: "Dear".to_string(),
                name: NameFormat::TitleLast,
                after: ",".to_string(),
                invalid: "Dear Sir or Madam,".to_string(),
                japanese,
            }
        }
    }

    pub fn format(&self) -> String {
        let mut format = String::new();
        if !self.before.is_empty() {
            format.push_str(&format!("<<_BEFORE_ {} >>", self.before));
        }
        format.push_str(self.name.groups());
        if !self.after.is_empty() {
            let space = if self.japanese && !self.after.starts_with([',', ':']) {
                " "
            } else {
                ""
            };
            format.push_str(&format!("<<_AFTER_{space}{}>>", self.after));
        }
        format
    }

    pub fn instruction(&self) -> String {
        format!(
            "GREETINGLINE \\f \"{}\" \\l {} \\e \"{}\"",
            self.format().replace('"', "\\\""),
            if self.japanese { 1041 } else { 1033 },
            self.invalid.replace('"', "\\\"")
        )
    }
}

/// Renders a format of `<<…>>` groups: a group shows when one of its
/// address tokens has a value (or it is `_BEFORE_` / `_AFTER_` text), with
/// the tokens replaced; text outside groups is kept; blank lines go.
pub fn render_format(
    format: &str,
    value: &dyn Fn(AddressField) -> String,
    country_shown: &dyn Fn(&str) -> bool,
) -> String {
    let mut out = String::new();
    let mut rest = format;
    while let Some(open) = rest.find("<<") {
        out.push_str(&rest[..open]);
        let after_open = &rest[open + 2..];
        let Some(close) = after_open.find(">>") else {
            out.push_str(&rest[open..]);
            rest = "";
            break;
        };
        let group = &after_open[..close];
        rest = &after_open[close + 2..];
        if let Some(text) = group
            .strip_prefix("_BEFORE_")
            .or_else(|| group.strip_prefix("_AFTER_"))
        {
            out.push_str(text);
            continue;
        }
        let mut rendered = group.to_string();
        let mut has_value = false;
        let mut has_token = false;
        let mut cursor = 0;
        while let Some(start) = rendered[cursor..].find('_').map(|at| cursor + at) {
            let Some(end) = rendered[start + 1..].find('_').map(|at| start + 1 + at) else {
                break;
            };
            let token = &rendered[start..=end];
            match FieldMapping::field_of_token(token) {
                Some(field) => {
                    has_token = true;
                    let mut text = value(field);
                    if field == AddressField::Country && !country_shown(&text) {
                        text.clear();
                    }
                    if !text.trim().is_empty() {
                        has_value = true;
                    }
                    rendered.replace_range(start..=end, &text);
                    cursor = start + text.len();
                }
                None => cursor = end,
            }
        }
        if !has_token || has_value {
            out.push_str(&rendered);
        }
    }
    out.push_str(rest);
    out.replace("\r\n", "\n")
        .replace('\r', "\n")
        .split('\n')
        .map(str::trim_end)
        .filter(|line| !line.trim().is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}
