//! Calculation preferences and cache freshness in the workbook part.

use crate::patch::{apply, Splice};
use crate::Result;
use quick_xml::{events::Event, name::ResolveResult, NsReader};

fn spreadsheet(namespace: &ResolveResult<'_>) -> bool {
    match namespace {
        ResolveResult::Unbound => true,
        ResolveResult::Bound(namespace) => matches!(
            namespace.as_ref(),
            b"http://schemas.openxmlformats.org/spreadsheetml/2006/main"
                | b"http://purl.oclc.org/ooxml/spreadsheetml/main"
        ),
        ResolveResult::Unknown(_) => false,
    }
}

fn update_properties(xml: &str, updates: &[(&str, &str)], insert: bool) -> Result<String> {
    let mut reader = NsReader::from_str(xml);
    reader.config_mut().trim_text(false);
    let mut depth = 0usize;
    let mut root_prefix = String::new();
    let mut root_name = String::new();
    let mut root_namespace = Vec::new();
    let mut root_empty = None;
    let mut insertion = None;
    let mut splices = Vec::new();
    loop {
        let start = reader.buffer_position() as usize;
        let event = reader.read_event()?;
        match event {
            Event::Start(ref element) | Event::Empty(ref element) => {
                let empty = matches!(event, Event::Empty(_));
                let end = reader.buffer_position() as usize;
                let name = element.name();
                let (namespace, local) = reader.resolver().resolve_element(name);
                if depth == 0 && local.as_ref() == b"workbook" && spreadsheet(&namespace) {
                    root_name = String::from_utf8_lossy(name.as_ref()).into_owned();
                    if let ResolveResult::Bound(namespace) = &namespace {
                        root_namespace = namespace.as_ref().to_vec();
                    }
                    root_prefix = crate::patch::namespace_prefix(name.as_ref());
                    if empty {
                        root_empty = Some((start, end));
                    }
                }
                if depth == 1
                    && !root_name.is_empty()
                    && match &namespace {
                        ResolveResult::Unbound => root_namespace.is_empty(),
                        ResolveResult::Bound(namespace) => {
                            namespace.as_ref() == root_namespace.as_slice()
                        }
                        _ => false,
                    }
                {
                    if local.as_ref() == b"calcPr" {
                        for (key, value) in updates {
                            let mut found = false;
                            for attribute in element.attributes() {
                                let attribute = attribute?;
                                if attribute.key.as_ref() == key.as_bytes() {
                                    let raw: &[u8] = element.as_ref();
                                    let offset =
                                        attribute.value.as_ptr() as usize - raw.as_ptr() as usize;
                                    splices.push(Splice {
                                        start: start + 1 + offset,
                                        end: start + 1 + offset + attribute.value.len(),
                                        text: (*value).into(),
                                    });
                                    found = true;
                                    break;
                                }
                            }
                            if !found {
                                let at = end - if empty { 2 } else { 1 };
                                splices.push(Splice {
                                    start: at,
                                    end: at,
                                    text: format!(" {key}=\"{value}\""),
                                });
                            }
                        }
                        return Ok(apply(xml, splices));
                    }
                    // calcPr precedes these workbook children in SpreadsheetML.
                    if insertion.is_none()
                        && matches!(
                            local.as_ref(),
                            b"oleSize"
                                | b"customWorkbookViews"
                                | b"pivotCaches"
                                | b"smartTagPr"
                                | b"smartTagTypes"
                                | b"webPublishing"
                                | b"fileRecoveryPr"
                                | b"webPublishObjects"
                                | b"extLst"
                        )
                    {
                        insertion = Some(start);
                    }
                }
                if !empty {
                    depth += 1;
                }
            }
            Event::End(ref element) => {
                depth = depth.saturating_sub(1);
                if depth == 0
                    && element.name().as_ref() == root_name.as_bytes()
                    && insertion.is_none()
                {
                    insertion = Some(start);
                }
            }
            Event::Eof => break,
            _ => {}
        }
    }
    if !insert || root_name.is_empty() {
        return Ok(xml.to_string());
    }
    let attributes = updates
        .iter()
        .map(|(key, value)| format!(" {key}=\"{value}\""))
        .collect::<String>();
    let child = format!("<{root_prefix}calcPr calcId=\"0\"{attributes}/>");
    if let Some((start, end)) = root_empty {
        let head = &xml[start..end - 2];
        splices.push(Splice {
            start,
            end,
            text: format!("{head}>{child}</{root_name}>"),
        });
    } else if let Some(at) = insertion {
        splices.push(Splice {
            start: at,
            end: at,
            text: child,
        });
    }
    Ok(apply(xml, splices))
}

pub fn request_full_recalculation(xml: &str) -> String {
    update_properties(xml, &[("fullCalcOnLoad", "1")], true).unwrap_or_else(|_| xml.to_string())
}

pub fn withdraw_full_recalculation(xml: &str) -> String {
    update_properties(xml, &[("fullCalcOnLoad", "0")], false).unwrap_or_else(|_| xml.to_string())
}

pub fn set_properties(
    xml: &str,
    mode: jade_xlsx_reader::CalculationMode,
    calculate_on_save: bool,
) -> Result<String> {
    update_properties(
        xml,
        &[
            ("calcMode", mode.as_str()),
            ("calcOnSave", if calculate_on_save { "1" } else { "0" }),
        ],
        true,
    )
}
