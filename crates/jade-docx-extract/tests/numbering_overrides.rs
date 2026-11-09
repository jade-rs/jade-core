use jade_docx_extract::extract_from_xml_parts;

fn labels(numbering: &str, paragraphs: &[(u32, u32)]) -> Vec<String> {
    let body: String = paragraphs.iter().map(|(id, level)| format!(
        r#"<w:p><w:pPr><w:numPr><w:ilvl w:val="{level}"/><w:numId w:val="{id}"/></w:numPr></w:pPr><w:r><w:t>Item</w:t></w:r></w:p>"#
    )).collect();
    let xml = format!(
        r#"<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:body>{body}</w:body></w:document>"#
    );
    let paragraphs = extract_from_xml_parts(&xml, Some(numbering), None).unwrap();
    paragraphs
        .into_iter()
        .map(|paragraph| {
            assert_eq!(paragraph.accepted_numbering_text, paragraph.numbering_text);
            paragraph.numbering_text
        })
        .collect()
}

#[test]
fn concrete_level_replaces_format_and_start_only_for_its_instance() {
    let xml = r#"<w:numbering xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
      <w:abstractNum w:abstractNumId="1"><w:lvl w:ilvl="0"><w:start w:val="1"/><w:numFmt w:val="decimal"/><w:lvlText w:val="%1."/></w:lvl></w:abstractNum>
      <w:num w:numId="1"><w:abstractNumId w:val="1"/></w:num>
      <w:num w:numId="2"><w:abstractNumId w:val="1"/><w:lvlOverride w:ilvl="0"><w:startOverride w:val="4"/><w:lvl w:ilvl="0"><w:start w:val="4"/><w:numFmt w:val="upperRoman"/><w:lvlText w:val="Article %1)"/><w:suff w:val="space"/></w:lvl></w:lvlOverride></w:num>
      <w:num w:numId="3"><w:abstractNumId w:val="1"/><w:lvlOverride w:ilvl="0"><w:startOverride w:val="8"/><w:lvl w:ilvl="0"><w:start w:val="4"/><w:numFmt w:val="lowerLetter"/><w:lvlText w:val="%1)"/></w:lvl></w:lvlOverride></w:num>
    </w:numbering>"#;
    assert_eq!(
        labels(xml, &[(1, 0), (2, 0), (3, 0), (2, 0), (1, 0)]),
        ["1.", "Article IV) ", "h)", "Article V) ", "6."]
    );
}

#[test]
fn placeholders_use_replaced_parent_format_and_other_levels_stay_available() {
    let xml = r#"<w:numbering xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
      <w:abstractNum w:abstractNumId="1">
        <w:lvl w:ilvl="0"><w:start w:val="1"/><w:numFmt w:val="decimal"/><w:lvlText w:val="%1."/></w:lvl>
        <w:lvl w:ilvl="1"><w:start w:val="1"/><w:numFmt w:val="lowerLetter"/><w:lvlText w:val="%1.%2)"/></w:lvl>
      </w:abstractNum>
      <w:num w:numId="2"><w:abstractNumId w:val="1"/><w:lvlOverride w:ilvl="0"><w:startOverride w:val="4"/><w:lvl w:ilvl="0"><w:start w:val="4"></w:start><w:numFmt w:val="upperRoman"></w:numFmt><w:lvlText w:val="Article %1)"></w:lvlText></w:lvl></w:lvlOverride></w:num>
      <w:num w:numId="3"><w:abstractNumId w:val="1"/><w:lvlOverride w:ilvl="0"><w:lvl w:ilvl="0"><w:numFmt w:val="bullet"/><w:lvlText w:val="•"/></w:lvl></w:lvlOverride></w:num>
    </w:numbering>"#;
    assert_eq!(
        labels(xml, &[(2, 0), (2, 1), (3, 0)]),
        ["Article IV)", "IV.a)", "•"]
    );
}

#[test]
fn empty_replacement_does_not_inherit_abstract_format_or_text() {
    for level in [r#"<w:lvl w:ilvl="0"/>"#, r#"<w:lvl w:ilvl="0"></w:lvl>"#] {
        let xml = format!(
            r#"<w:numbering xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:abstractNum w:abstractNumId="1"><w:lvl w:ilvl="0"><w:numFmt w:val="upperRoman"/><w:lvlText w:val="Article %1)"/></w:lvl></w:abstractNum><w:num w:numId="2"><w:abstractNumId w:val="1"/><w:lvlOverride w:ilvl="0">{level}</w:lvlOverride></w:num></w:numbering>"#
        );
        assert_eq!(labels(&xml, &[(2, 0)]), ["1."]);
    }
}

#[test]
fn restart_controls_which_ancestors_reset_a_level() {
    for (restart, expected) in [
        ("", ["3.", "1.", "3.", "1.", "3."]),
        (
            r#"<w:lvlRestart w:val="0"/>"#,
            ["3.", "1.", "4.", "1.", "5."],
        ),
        (
            r#"<w:lvlRestart w:val="1"></w:lvlRestart>"#,
            ["3.", "1.", "4.", "1.", "3."],
        ),
        (
            r#"<w:lvlRestart w:val="2"/>"#,
            ["3.", "1.", "3.", "1.", "3."],
        ),
        // Invalid forward references and values outside Word's range are ignored.
        (
            r#"<w:lvlRestart w:val="3"/>"#,
            ["3.", "1.", "3.", "1.", "3."],
        ),
        (
            r#"<w:lvlRestart w:val="99"/>"#,
            ["3.", "1.", "3.", "1.", "3."],
        ),
    ] {
        let xml = format!(
            r#"<w:numbering xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:abstractNum w:abstractNumId="1"><w:lvl w:ilvl="0"><w:lvlText w:val="%1."/></w:lvl><w:lvl w:ilvl="1"><w:lvlText w:val="%2."/></w:lvl><w:lvl w:ilvl="2"><w:start w:val="3"/><w:lvlText w:val="%3."/>{restart}</w:lvl></w:abstractNum><w:num w:numId="1"><w:abstractNumId w:val="1"/></w:num></w:numbering>"#
        );
        assert_eq!(
            labels(&xml, &[(1, 2), (1, 1), (1, 2), (1, 0), (1, 2)]),
            expected,
            "{restart}"
        );
    }
}

#[test]
fn word_ignores_start_and_restart_inside_concrete_level() {
    let xml = r#"<w:numbering xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:abstractNum w:abstractNumId="1"><w:lvl w:ilvl="0"><w:lvlText w:val="%1."/></w:lvl><w:lvl w:ilvl="1"><w:start w:val="3"/><w:lvlRestart w:val="0"/><w:lvlText w:val="%2."/></w:lvl></w:abstractNum><w:num w:numId="1"><w:abstractNumId w:val="1"/><w:lvlOverride w:ilvl="1"><w:lvl w:ilvl="1"><w:start w:val="9"/><w:lvlRestart w:val="1"/><w:numFmt w:val="upperRoman"/><w:lvlText w:val="%2)"/></w:lvl></w:lvlOverride></w:num></w:numbering>"#;
    assert_eq!(
        labels(xml, &[(1, 1), (1, 0), (1, 1)]),
        ["III)", "1.", "IV)"]
    );
}

#[test]
fn zero_start_advances_and_shared_num_ids_follow_latest_counter() {
    let xml = r#"<w:numbering xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:abstractNum w:abstractNumId="1"><w:lvl w:ilvl="0"><w:start w:val="0"/><w:lvlText w:val="%1."/></w:lvl></w:abstractNum><w:num w:numId="1"><w:abstractNumId w:val="1"/></w:num><w:num w:numId="2"><w:abstractNumId w:val="1"/></w:num></w:numbering>"#;
    assert_eq!(
        labels(xml, &[(1, 0), (1, 0), (2, 0), (1, 0)]),
        ["0.", "1.", "2.", "3."]
    );
}

#[test]
fn legal_numbering_uses_decimal_for_ancestors_and_current_level() {
    for (flag, expected) in [
        ("", "III.01"),
        ("<w:isLgl/>", "3.1"),
        (r#"<w:isLgl w:val="true"></w:isLgl>"#, "3.1"),
        (r#"<w:isLgl w:val="1"/>"#, "3.1"),
        (r#"<w:isLgl w:val="on"/>"#, "3.1"),
        (r#"<w:isLgl w:val="0"/>"#, "III.01"),
        (r#"<w:isLgl w:val="false"></w:isLgl>"#, "III.01"),
        (r#"<w:isLgl w:val="off"/>"#, "III.01"),
    ] {
        let xml = format!(
            r#"<w:numbering xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:abstractNum w:abstractNumId="1"><w:lvl w:ilvl="0"><w:start w:val="3"/><w:numFmt w:val="upperRoman"/><w:lvlText w:val="%1"/></w:lvl><w:lvl w:ilvl="1"><w:numFmt w:val="decimalZero"/><w:lvlText w:val="%1.%2"/>{flag}</w:lvl></w:abstractNum><w:num w:numId="1"><w:abstractNumId w:val="1"/></w:num></w:numbering>"#
        );
        assert_eq!(labels(&xml, &[(1, 0), (1, 1)]), ["III", expected], "{flag}");
        // Implicit ancestors use their initial value; legal decimal keeps zero.
        assert_eq!(labels(&xml, &[(1, 1)]), [expected], "{flag}");
    }
}

#[test]
fn legal_numbering_honors_concrete_replacement_and_none() {
    for (abstract_flag, replacement_flag, expected) in [
        ("", "<w:isLgl/>", "3.1"),
        ("<w:isLgl/>", "", "III.a"),
        ("<w:isLgl/>", r#"<w:isLgl w:val="0"/>"#, "III.a"),
    ] {
        let xml = format!(
            r#"<w:numbering xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:abstractNum w:abstractNumId="1"><w:lvl w:ilvl="0"><w:start w:val="3"/><w:numFmt w:val="upperRoman"/><w:lvlText w:val="%1"/></w:lvl><w:lvl w:ilvl="1"><w:numFmt w:val="decimal"/><w:lvlText w:val="%2"/>{abstract_flag}</w:lvl></w:abstractNum><w:num w:numId="1"><w:abstractNumId w:val="1"/><w:lvlOverride w:ilvl="1"><w:lvl w:ilvl="1"><w:numFmt w:val="lowerLetter"/><w:lvlText w:val="%1.%2"/>{replacement_flag}</w:lvl></w:lvlOverride></w:num></w:numbering>"#
        );
        assert_eq!(labels(&xml, &[(1, 0), (1, 1)]), ["III", expected]);
    }
    let xml = r#"<w:numbering xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:abstractNum w:abstractNumId="1"><w:lvl w:ilvl="0"><w:numFmt w:val="none"/><w:lvlText w:val="%1"/><w:isLgl/></w:lvl><w:lvl w:ilvl="1"><w:numFmt w:val="decimalZero"/><w:lvlText w:val="%1.%2"/><w:isLgl/></w:lvl></w:abstractNum><w:num w:numId="1"><w:abstractNumId w:val="1"/></w:num></w:numbering>"#;
    assert_eq!(labels(xml, &[(1, 0), (1, 1)]), ["", ".1"]);
    let xml = xml.replace(
        r#"<w:numFmt w:val="none"/>"#,
        r#"<w:start w:val="0"/><w:numFmt w:val="upperRoman"/>"#,
    );
    assert_eq!(labels(&xml, &[(1, 0), (1, 1)]), ["0", "0.1"]);
}

#[test]
fn multilingual_number_formats_are_not_replaced_with_decimal() {
    for (format, start, expected) in [
        ("japaneseCounting", 21, "二十一"),
        ("japaneseLegal", 11, "拾壱"),
        ("decimalFullWidth", 12, "１２"),
        ("decimalFullWidth2", 0, "０"),
        ("decimalEnclosedCircle", 3, "③"),
        ("ideographDigital", 12, "一二"),
        ("koreanDigital2", 12, "一二"),
        ("vietnameseCounting", 21, "hai mươi mốt"),
        ("hindiVowels", 2, "आ"),
        ("hindiCounting", 3, "तीन"),
        ("thaiCounting", 11, "สิบเอ็ด"),
        ("dollarText", 5, "five and 00/100"),
        ("hex", 255, "FF"),
        ("numberInDash", 12, "- 12 -"),
        ("unrecognized", 7, "7"),
    ] {
        let xml = format!(
            r#"<w:numbering xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:abstractNum w:abstractNumId="1"><w:lvl w:ilvl="0"><w:start w:val="{start}"/><w:numFmt w:val="{format}"/><w:lvlText w:val="%1"/></w:lvl></w:abstractNum><w:num w:numId="1"><w:abstractNumId w:val="1"/></w:num></w:numbering>"#
        );
        assert_eq!(labels(&xml, &[(1, 0)]), [expected], "{format}");
    }
}

#[test]
fn language_sensitive_formats_use_numbering_level_language() {
    for (language, expected) in [
        ("en-US", "twenty-one"),
        ("en-GB", "twenty-one"),
        ("ja-JP", "21"),
        ("", "21"),
    ] {
        let xml = format!(
            r#"<w:numbering xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:abstractNum w:abstractNumId="1"><w:lvl w:ilvl="0"><w:start w:val="21"/><w:numFmt w:val="cardinalText"/><w:lvlText w:val="%1"/><w:rPr><w:lang w:val="{language}"></w:lang></w:rPr></w:lvl></w:abstractNum><w:num w:numId="1"><w:abstractNumId w:val="1"/></w:num></w:numbering>"#
        );
        assert_eq!(labels(&xml, &[(1, 0)]), [expected], "{language}");
    }
}

#[test]
fn multilingual_parent_and_custom_override_survive_fallback() {
    let xml = r#"<w:numbering xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main" xmlns:mc="http://schemas.openxmlformats.org/markup-compatibility/2006" xmlns:w14="http://schemas.microsoft.com/office/word/2010/wordml"><w:abstractNum w:abstractNumId="1"><w:lvl w:ilvl="0"><w:start w:val="2"/><w:numFmt w:val="japaneseCounting"/><w:lvlText w:val="第%1章"/></w:lvl><w:lvl w:ilvl="1"><w:numFmt w:val="decimal"/><w:lvlText w:val="%1-%2"/></w:lvl></w:abstractNum><w:num w:numId="1"><w:abstractNumId w:val="1"/><w:lvlOverride w:ilvl="1"><w:startOverride w:val="4"/><w:lvl w:ilvl="1"><mc:AlternateContent><mc:Choice Requires="w14"><w:numFmt w:val="custom" w:format="&#48;01, 002, 003, ..."></w:numFmt></mc:Choice><mc:Fallback><w:numFmt w:val="decimal"/></mc:Fallback></mc:AlternateContent><w:lvlText w:val="%1-%2"/></w:lvl></w:lvlOverride></w:num></w:numbering>"#;
    assert_eq!(
        labels(xml, &[(1, 0), (1, 1), (1, 1)]),
        ["第二章", "二-004", "二-005"]
    );
}
