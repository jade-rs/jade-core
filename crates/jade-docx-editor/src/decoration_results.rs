use crate::plugins::DecoratedRange;

/// Decode the whole batch before the caller installs any cache entries.
/// Legacy decoration updates tolerate unknown range formats; a prepaint
/// provider must instead reject the batch and request an ordinary repaint.
pub(crate) fn parse_results(
    json: &str,
    strict: bool,
) -> Result<Vec<(String, Vec<DecoratedRange>)>, String> {
    #[derive(serde::Deserialize)]
    struct Entry {
        text: String,
        #[serde(rename = "rangesJson")]
        ranges_json: String,
    }
    let entries: Vec<Entry> =
        serde_json::from_str(json).map_err(|err| format!("invalid results JSON: {err}"))?;
    entries
        .into_iter()
        .map(|entry| {
            let ranges = if entry.ranges_json.is_empty() || entry.ranges_json == "[]" {
                Vec::new()
            } else {
                match serde_json::from_str(&entry.ranges_json) {
                    Ok(ranges) => ranges,
                    Err(err) if strict => return Err(format!("invalid ranges JSON: {err}")),
                    Err(_) => Vec::new(),
                }
            };
            Ok((entry.text, ranges))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::parse_results;
    use crate::plugins::{DecoratedRange, DecorationCache, DecorationStyle};
    use proptest::prelude::*;

    #[test]
    fn invalid_provider_batch_cannot_install_its_valid_prefix() {
        let mut cache = DecorationCache::new();
        let existing = vec![DecoratedRange {
            start: 0,
            end: 3,
            style: DecorationStyle::Squiggly {
                color: "red".into(),
            },
        }];
        cache.put("日".into(), existing.clone());
        let json = serde_json::json!([
            { "text": "日", "rangesJson": "[]" },
            { "text": "本", "rangesJson": "{" }
        ])
        .to_string();
        let result = parse_results(&json, true);
        if let Ok(entries) = &result {
            for (text, ranges) in entries {
                cache.put(text.clone(), ranges.clone());
            }
        }
        assert!(result.is_err());
        assert_eq!(cache.get("日"), Some(existing.as_slice()));
        assert!(cache.get("本").is_none());
        assert_eq!(
            parse_results(&json, false).unwrap(),
            vec![("日".into(), Vec::new()), ("本".into(), Vec::new())]
        );
    }

    #[test]
    fn malformed_outer_protocol_is_always_rejected() {
        for json in [
            "{",
            "null",
            "{}",
            r#"[{"text":3,"rangesJson":"[]"}]"#,
            r#"[{"text":"x","rangesJson":[]}]"#,
        ] {
            for strict in [false, true] {
                assert!(parse_results(json, strict).is_err(), "{json}");
            }
        }
        for strict in [false, true] {
            assert!(parse_results("[]", strict).unwrap().is_empty());
        }
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(128))]
        #[test]
        fn provider_batches_preserve_utf8_offsets_order_and_styles(
            texts in prop::collection::vec("[a-z日本語😀]{1,20}", 0..20),
            empty_ranges in any::<bool>(),
        ) {
            let expected: Vec<_> = texts.iter().map(|text| {
                let ranges = if empty_ranges { Vec::new() } else { vec![DecoratedRange {
                    start: 0, end: text.len(),
                    style: DecorationStyle::Squiggly { color: "#ff0000".into() },
                }] };
                (text.clone(), ranges)
            }).collect();
            let entries: Vec<_> = expected.iter().map(|(text, ranges)| serde_json::json!({
                "text": text, "rangesJson": serde_json::to_string(ranges).unwrap(),
            })).collect();
            let json = serde_json::to_string(&entries).unwrap();
            prop_assert_eq!(parse_results(&json, true).unwrap(), expected.clone());
            prop_assert_eq!(parse_results(&json, false).unwrap(), expected);
        }
    }
}
