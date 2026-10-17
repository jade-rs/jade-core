//! JS-backed decoration plugin.
//!
//! [`JsCallbackPlugin`] wraps a `js_sys::Function` so that any decoration
//! matcher implemented on the JavaScript side can be registered with the
//! renderer. The callback receives the segment's text as a UTF-8 string
//! and must return either:
//!
//! * a JSON string matching the shape of `Vec<DecoratedRange>`
//!   (see [`jade_preview_plugin_api`] for the wire schema), or
//! * `null` / `undefined` / any non-string value — interpreted as
//!   "no decorations".
//!
//! ## Why JS-level, not WASM-level
//!
//! Guest plugins compiled to `wasm32-unknown-unknown` modules (see
//! `crates/jade-plugins/circle-mark-plugin` for a reference implementation)
//! cannot be called directly from the host WASM module: the two modules
//! have separate linear memories and the `plugin_decorate` return value
//! is an `i64` that JavaScript surfaces as a `BigInt`. Bridging those
//! details is straightforward in JS and awkward in Rust.
//!
//! We therefore draw the FFI boundary at the JS layer: the host-side
//! plugin trait receives a plain `(text: string) => string` function,
//! and JavaScript (see the jade-ide embedding) takes care of:
//!
//! 1. Allocating an input buffer in the guest's memory via
//!    `plugin_alloc`,
//! 2. Copying the UTF-8 bytes,
//! 3. Calling `plugin_decorate` and unpacking the `BigInt` result,
//! 4. Reading the JSON payload from guest memory,
//! 5. Freeing both buffers via `plugin_free`.
//!
//! The side effect of this split is that a [`JsCallbackPlugin`] can also
//! be backed by pure JavaScript logic (a regex matcher, a lint rule, an
//! API call) without ever touching WebAssembly. The guest WASM story is
//! just one flavor of this callback.

use jade_preview_plugin_api::DecoratedRange;
use js_sys::Function;
use wasm_bindgen::JsValue;

use super::DecorationPlugin;

/// A decoration plugin implemented as a JavaScript callback.
///
/// The callback is invoked synchronously for each segment; it must return
/// quickly or the render pass stalls. See
/// [`DecorationPlugin::decorate`] for the required semantics.
pub struct JsCallbackPlugin {
    name: String,
    decorate_fn: Function,
}

impl JsCallbackPlugin {
    /// Wraps the given JS function as a plugin with the supplied `name`.
    /// The name is visible to [`crate::plugins::DecorationRegistry::unregister`]
    /// and is reported as the plugin's identifier in diagnostics.
    pub fn new(name: String, decorate_fn: Function) -> Self {
        Self { name, decorate_fn }
    }
}

impl DecorationPlugin for JsCallbackPlugin {
    fn name(&self) -> &str {
        &self.name
    }

    fn decorate(&self, text: &str) -> Vec<DecoratedRange> {
        // Call the JS callback with the segment text as a single argument.
        // Any thrown exception (or a non-string return) is treated as "no
        // decorations" rather than being propagated: a misbehaving plugin
        // must never crash the render pass.
        let arg = JsValue::from_str(text);
        let Ok(result) = self.decorate_fn.call1(&JsValue::NULL, &arg) else {
            return Vec::new();
        };
        let Some(json) = result.as_string() else {
            return Vec::new();
        };
        parse_decorations_json(&json)
    }
}

/// Parses a JSON payload produced by a decoration plugin.
///
/// Extracted as a free function so it can be unit-tested on the host
/// toolchain without going through `js_sys`. Returns an empty `Vec` if
/// the payload is not valid JSON in the expected shape — a plugin should
/// not be able to crash the render pass by emitting garbage.
pub fn parse_decorations_json(json: &str) -> Vec<DecoratedRange> {
    if json.is_empty() {
        return Vec::new();
    }
    serde_json::from_str::<Vec<DecoratedRange>>(json).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use jade_preview_plugin_api::DecorationStyle;

    #[test]
    fn parses_empty_array() {
        assert!(parse_decorations_json("[]").is_empty());
    }

    #[test]
    fn parses_empty_string_as_empty() {
        assert!(parse_decorations_json("").is_empty());
    }

    #[test]
    fn parses_underline_range() {
        let json = r##"[
            {
                "start": 3,
                "end": 6,
                "style": {"kind": "underline", "color": "#FF0000"}
            }
        ]"##;
        let ranges = parse_decorations_json(json);
        assert_eq!(ranges.len(), 1);
        assert_eq!(ranges[0].start, 3);
        assert_eq!(ranges[0].end, 6);
        assert!(matches!(
            &ranges[0].style,
            DecorationStyle::Underline { color, thickness_px: None } if color == "#FF0000"
        ));
    }

    #[test]
    fn parses_underline_range_with_thickness() {
        let json = r#"[
            {
                "start": 0,
                "end": 2,
                "style": {"kind": "underline", "color": "red", "thickness_px": 2.5}
            }
        ]"#;
        let ranges = parse_decorations_json(json);
        assert!(matches!(
            &ranges[0].style,
            DecorationStyle::Underline { color, thickness_px: Some(t) }
                if color == "red" && (*t - 2.5).abs() < 1e-9
        ));
    }

    #[test]
    fn malformed_json_yields_empty_vec() {
        assert!(parse_decorations_json("{ not json").is_empty());
        assert!(parse_decorations_json("null").is_empty());
        assert!(parse_decorations_json("42").is_empty());
        assert!(parse_decorations_json("\"a string\"").is_empty());
    }

    #[test]
    fn unknown_style_kind_drops_whole_payload() {
        // Conservative choice: if ANY range in the payload has a kind we
        // do not understand, we drop the whole array rather than silently
        // truncating. The plugin can be re-registered with a compatible
        // schema after the host is upgraded.
        //
        // `"zigzag"` is a hypothetical future variant that this host
        // does not yet understand; the test is stable against new
        // `DecorationStyle` additions so long as we do not introduce
        // a variant named `zigzag`.
        let json = r#"[
            {"start": 0, "end": 1, "style": {"kind": "zigzag", "color": "blue"}}
        ]"#;
        assert!(parse_decorations_json(json).is_empty());
    }

    #[test]
    fn highlight_variant_parses() {
        let json = r#"[
            {"start": 0, "end": 5, "style": {"kind": "highlight", "color": "yellow"}}
        ]"#;
        let ranges = parse_decorations_json(json);
        assert_eq!(ranges.len(), 1);
        assert!(matches!(
            &ranges[0].style,
            DecorationStyle::Highlight { color } if color == "yellow"
        ));
    }

    #[test]
    fn squiggly_variant_parses() {
        let json = r##"[
            {"start": 0, "end": 5, "style": {"kind": "squiggly", "color": "#FF4040"}}
        ]"##;
        let ranges = parse_decorations_json(json);
        assert_eq!(ranges.len(), 1);
        assert!(matches!(
            &ranges[0].style,
            DecorationStyle::Squiggly { color } if color == "#FF4040"
        ));
    }
}
