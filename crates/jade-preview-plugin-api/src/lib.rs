//! Guest ABI for jade decoration plugins.
//!
//! This crate is shared between the host (the jade editor, running
//! as a WASM module in the browser) and guests (third-party decoration
//! plugins, compiled to standalone `wasm32-unknown-unknown` modules and
//! loaded dynamically from JavaScript at runtime).
//!
//! ## Design choice: hand-rolled C ABI + JSON payloads
//!
//! The MVP deliberately avoids the Component Model / WIT toolchain so that:
//!
//! * Plugins are plain `cargo build --target wasm32-unknown-unknown` crates
//!   with no `wit-bindgen`, no `jco`, and no runtime adapter layer.
//! * The host can call into a plugin using only the JavaScript
//!   `WebAssembly.Instance.exports` surface — no component-model runtime is
//!   required in the browser.
//! * The wire format is `serde_json` (`Vec<DecoratedRange>`), so adding a
//!   new `DecorationStyle` variant is a non-breaking schema change as long
//!   as it uses `#[serde(other)]`-tolerant deserialization on the host.
//!
//! The cost is a JSON encode/decode per segment. For the decoration volumes
//! we care about (a handful of ranges per visible paragraph) this is
//! negligible. A faster binary encoding can be layered on later without
//! breaking the exported symbol set.
//!
//! ## Exported symbols
//!
//! A conforming guest module MUST export exactly these symbols (generated
//! by [`export_plugin!`]):
//!
//! | name              | signature                                | purpose |
//! |-------------------|------------------------------------------|---------|
//! | `plugin_alloc`    | `(len: u32) -> u32`                      | The host calls this to reserve a buffer inside guest linear memory, writes the input text bytes into `[ptr, ptr+len)`, then calls `plugin_decorate`. The guest retains ownership of the buffer until `plugin_free`. |
//! | `plugin_free`     | `(ptr: u32, len: u32)`                   | The host calls this to release an input buffer (after `plugin_decorate` returned) and to release result buffers (after the host has copied the bytes out). |
//! | `plugin_decorate` | `(text_ptr: u32, text_len: u32) -> u64`  | Runs the plugin's matcher over the UTF-8 slice at `[text_ptr, text_ptr+text_len)` and returns a **packed pointer**: the high 32 bits are the result buffer pointer, the low 32 bits are the result buffer length. The result buffer holds a JSON-encoded `Vec<DecoratedRange>`. |
//! | `plugin_configure` | `(ptr: u32, len: u32) -> u32`           | Hands the guest a UTF-8 configuration blob. Returns 1 when the guest accepted it, 0 otherwise. The host calls this once at registration, before the first `plugin_decorate`. Plugins that need no configuration ignore it. |
//! | `plugin_abi_version` | `() -> u32`                           | Returns [`ABI_VERSION`]. The host rejects plugins whose version it does not support. |
//!
//! ## Configuration
//!
//! A matcher that can be written as a pure function of the text — "underline
//! every ◯" — needs nothing from the host. Others have to be *told* what to
//! look for: a diff highlighter knows which ranges changed only because
//! something outside the document computed them.
//!
//! `plugin_configure` is that channel. The host passes a UTF-8 string
//! (JSON in practice, but the ABI does not care) which the guest keeps for
//! the lifetime of the registration.
//!
//! Note what this does *not* buy you. The host caches decoration results
//! **keyed on the paragraph text**, and two paragraphs with the same body
//! share one cache entry. So a configuration has to be expressed in terms
//! of the text itself, not the paragraph's identity — keyed by text, a
//! diff highlighter will decorate both of two identical paragraphs even
//! when only one of them changed. Carrying paragraph identity through the
//! renderer would mean re-keying that cache, which is a bigger change than
//! this hook.
//!
//! ## Result encoding
//!
//! ```text
//!   high 32 bits        low 32 bits
//! ┌─────────────────┬─────────────────┐
//! │ result_ptr : u32│ result_len : u32│
//! └─────────────────┴─────────────────┘
//! ```
//!
//! Packed via [`pack_result`] / unpacked via [`unpack_result`]. A result of
//! `0` (both halves zero) indicates "no decorations" and no buffer to free.
//!
//! ## Native-width helpers
//!
//! The ABI is defined in terms of `u32` because the only conforming guest
//! target is `wasm32-unknown-unknown`, where pointers fit in 32 bits. To
//! keep the helpers unit-testable on 64-bit hosts, the raw implementations
//! ([`alloc_buffer`], [`free_buffer`], [`run_plugin`]) operate on native
//! pointer widths; the [`export_plugin!`] macro performs the lossless
//! `*mut u8 <-> u32` cast at the WASM boundary.
//!
//! ## Authoring a plugin
//!
//! ```ignore
//! use jade_preview_plugin_api::{export_plugin, DecoratedRange, DecorationStyle};
//!
//! fn decorate(text: &str) -> Vec<DecoratedRange> {
//!     let mut out = Vec::new();
//!     for (byte_index, ch) in text.char_indices() {
//!         if ch == '◯' {
//!             out.push(DecoratedRange {
//!                 start: byte_index,
//!                 end: byte_index + ch.len_utf8(),
//!                 style: DecorationStyle::Underline {
//!                     color: "#FF0000".into(),
//!                     thickness_px: None,
//!                 },
//!             });
//!         }
//!     }
//!     out
//! }
//!
//! export_plugin!(decorate);
//! ```
//!
//! The crate must be built with `crate-type = ["cdylib"]` and the
//! `wasm32-unknown-unknown` target.

use serde::{Deserialize, Serialize};

/// Current ABI version. Incremented on breaking changes to the exported
/// symbol set, the packed result format, or the `DecoratedRange` /
/// `DecorationStyle` schema in a way that is not tolerated by older
/// consumers.
///
/// Guests report this via the `plugin_abi_version` export; the host rejects
/// plugins whose reported version it does not understand.
pub const ABI_VERSION: u32 = 2;

/// Versions of the ABI a host built against this crate can drive.
///
/// Version 1 guests lack `plugin_configure`; they are still perfectly
/// usable as long as the host skips the call when the export is absent.
pub const SUPPORTED_ABI_VERSIONS: &[u32] = &[1, 2];

/// A single decorated byte range reported by the plugin.
///
/// Byte offsets refer to the UTF-8 encoding of the input text passed to the
/// plugin. Both `start` and `end` must fall on UTF-8 character boundaries;
/// the host drops any range that does not.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DecoratedRange {
    pub start: usize,
    pub end: usize,
    pub style: DecorationStyle,
}

/// Visual style applied to a decorated range.
///
/// `#[serde(tag = "kind", rename_all = "kebab-case")]` makes the on-the-wire
/// form self-describing, which keeps future variants forward-compatible as
/// long as the host is tolerant of unknown tags (it will drop them).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum DecorationStyle {
    /// Draw a straight colored line along the baseline of the matched range.
    Underline {
        /// CSS color string ("#RRGGBB", "red", "rgb(...)", ...).
        color: String,
        /// Optional stroke width in CSS pixels. When `None` the host uses
        /// its built-in font-size-proportional thickness.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        thickness_px: Option<f64>,
    },
    /// Fill a rectangle behind the matched text. Useful for lint-style
    /// backgrounds (e.g. highlighting a `TODO` keyword).
    ///
    /// The host draws the fill BELOW the text layer, so fully opaque
    /// colors are fine — the glyphs still render on top. Use a
    /// translucent color (`"rgba(...)"`) if the intended effect is a
    /// "marker-pen" overlay.
    Highlight {
        /// CSS color string.
        color: String,
    },
    /// Draw a wavy underline — the classic spellchecker / lint
    /// decoration. Rendered as a zig-zag of short strokes along the
    /// baseline + a small offset, at a wavelength and amplitude
    /// proportional to the font size.
    Squiggly {
        /// CSS color string.
        color: String,
    },
    /// Draw a double straight line under the matched range — how Word
    /// marks a grammar or style issue.
    DoubleUnderline {
        /// CSS color string.
        color: String,
    },
}

/// Reads a configuration blob out of guest memory and hands it to
/// `configure` as a `&str`.
///
/// Returns `true` when the bytes were valid UTF-8 and `configure` accepted
/// them. Invalid UTF-8 is rejected without calling `configure` — a guest
/// should keep whatever configuration it already had rather than act on a
/// half-read blob.
///
/// # Safety
/// `ptr` / `len` must describe a readable byte slice in this module's
/// linear memory.
pub unsafe fn run_configure<F>(ptr: *const u8, len: usize, configure: F) -> bool
where
    F: FnOnce(&str) -> bool,
{
    if ptr.is_null() {
        return false;
    }
    let bytes = unsafe { std::slice::from_raw_parts(ptr, len) };
    match std::str::from_utf8(bytes) {
        Ok(text) => configure(text),
        Err(_) => false,
    }
}

/// Packs a `(ptr, len)` pair into the 64-bit return value expected by the
/// `plugin_decorate` contract.
#[inline]
pub const fn pack_result(ptr: u32, len: u32) -> u64 {
    ((ptr as u64) << 32) | (len as u64)
}

/// Inverse of [`pack_result`].
#[inline]
pub const fn unpack_result(packed: u64) -> (u32, u32) {
    ((packed >> 32) as u32, (packed & 0xFFFF_FFFF) as u32)
}

/// Allocates a byte buffer and returns its starting pointer.
///
/// The buffer is leaked so the caller retains a stable pointer across
/// intervening calls; [`free_buffer`] must be invoked to release it.
/// Returns a null pointer when `len == 0`.
pub fn alloc_buffer(len: usize) -> *mut u8 {
    if len == 0 {
        return std::ptr::null_mut();
    }
    // Zero-initialize so readers never observe UB if the host copies fewer
    // bytes than requested (or never writes the buffer at all). The cost is
    // a memset of `len` bytes, which is negligible next to the subsequent
    // JSON encode in `run_plugin`.
    let mut buf: Vec<u8> = vec![0u8; len];
    let ptr = buf.as_mut_ptr();
    std::mem::forget(buf);
    ptr
}

/// Releases a buffer previously returned from [`alloc_buffer`] or from a
/// `plugin_decorate` result.
///
/// # Safety
/// `ptr` and `len` must match a previous allocation performed by this
/// crate. Passing a null pointer or `len == 0` is a no-op.
pub unsafe fn free_buffer(ptr: *mut u8, len: usize) {
    if ptr.is_null() || len == 0 {
        return;
    }
    // SAFETY: `alloc_buffer` and `run_plugin` both shrink capacity to equal
    // length before leaking (via `Vec::with_capacity(len) + set_len(len)` or
    // `into_boxed_slice`), so reconstructing with `cap == len` matches the
    // original layout.
    let _ = unsafe { Vec::from_raw_parts(ptr, len, len) };
}

/// Runs the user-supplied decorator and, if it produced any decorations,
/// serializes the result as JSON into a freshly-leaked buffer.
///
/// Returns `(ptr, len)` pointing at the JSON result. Returns
/// `(null, 0)` when the input was empty, not valid UTF-8, or the
/// decorator produced zero ranges — the host treats this as "no
/// decorations" and skips the free step.
///
/// # Safety
/// `text_ptr` and `text_len` must describe a valid readable byte slice in
/// the current address space. The bytes do not need to be UTF-8 — invalid
/// input is simply reported as "no decorations".
pub unsafe fn run_plugin<F>(text_ptr: *const u8, text_len: usize, decorate: F) -> (*mut u8, usize)
where
    F: FnOnce(&str) -> Vec<DecoratedRange>,
{
    if text_len == 0 || text_ptr.is_null() {
        return (std::ptr::null_mut(), 0);
    }
    // SAFETY: Per contract, `[text_ptr, text_len)` is a readable slice. The
    // reference is only borrowed for the duration of `decorate`.
    let bytes = unsafe { std::slice::from_raw_parts(text_ptr, text_len) };
    let Ok(text) = std::str::from_utf8(bytes) else {
        return (std::ptr::null_mut(), 0);
    };

    let ranges = decorate(text);
    if ranges.is_empty() {
        return (std::ptr::null_mut(), 0);
    }
    let Ok(json) = serde_json::to_vec(&ranges) else {
        return (std::ptr::null_mut(), 0);
    };
    // `Vec<u8>` with capacity equal to length matches the layout that
    // `free_buffer` reconstructs below. Calling `shrink_to_fit` is the
    // documented way to guarantee `capacity == len` for this purpose.
    let mut buf = json;
    buf.shrink_to_fit();
    debug_assert_eq!(buf.capacity(), buf.len());
    let len = buf.len();
    let ptr = buf.as_mut_ptr();
    std::mem::forget(buf);
    (ptr, len)
}

/// Generates the complete guest export set from a single user-supplied
/// decorator function `fn(&str) -> Vec<DecoratedRange>`.
///
/// Invoke this macro exactly once at the crate root of a `cdylib` guest.
/// The macro expands to `#[no_mangle] pub extern "C"` symbols named
/// `plugin_alloc`, `plugin_free`, `plugin_decorate`, `plugin_configure`,
/// and `plugin_abi_version`.
///
/// Pass a second argument — `fn(&str) -> bool` — to receive the host's
/// configuration blob. With one argument the generated `plugin_configure`
/// accepts anything and does nothing, so a matcher that needs no
/// configuration stays a one-liner.
///
/// The `*mut u8 <-> u32` cast at the ABI boundary is lossless on
/// `wasm32-unknown-unknown` (the only supported guest target) where
/// pointers are exactly 32 bits.
///
/// ```ignore
/// use jade_preview_plugin_api::{export_plugin, DecoratedRange};
///
/// fn my_decorator(text: &str) -> Vec<DecoratedRange> {
///     // ... matcher logic ...
///     Vec::new()
/// }
///
/// export_plugin!(my_decorator);
/// ```
#[macro_export]
macro_rules! export_plugin {
    ($decorator:path) => {
        $crate::export_plugin!($decorator, |_| true);
    };
    ($decorator:path, $configure:expr) => {
        #[unsafe(no_mangle)]
        pub extern "C" fn plugin_abi_version() -> u32 {
            $crate::ABI_VERSION
        }

        #[unsafe(no_mangle)]
        pub extern "C" fn plugin_alloc(len: u32) -> u32 {
            $crate::alloc_buffer(len as usize) as u32
        }

        /// # Safety
        /// `ptr` / `len` must describe a buffer previously returned by
        /// `plugin_alloc` or from a `plugin_decorate` result.
        #[unsafe(no_mangle)]
        pub unsafe extern "C" fn plugin_free(ptr: u32, len: u32) {
            unsafe { $crate::free_buffer(ptr as *mut u8, len as usize) }
        }

        /// # Safety
        /// `text_ptr` / `text_len` must describe a readable byte slice in
        /// this module's linear memory.
        #[unsafe(no_mangle)]
        pub unsafe extern "C" fn plugin_decorate(text_ptr: u32, text_len: u32) -> u64 {
            let (ptr, len) =
                unsafe { $crate::run_plugin(text_ptr as *const u8, text_len as usize, $decorator) };
            $crate::pack_result(ptr as u32, len as u32)
        }

        /// # Safety
        /// `ptr` / `len` must describe a readable byte slice in this
        /// module's linear memory.
        #[unsafe(no_mangle)]
        pub unsafe extern "C" fn plugin_configure(ptr: u32, len: u32) -> u32 {
            let accepted =
                unsafe { $crate::run_configure(ptr as *const u8, len as usize, $configure) };
            if accepted { 1 } else { 0 }
        }
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pack_unpack_round_trip() {
        for (ptr, len) in [
            (0u32, 0u32),
            (1, 1),
            (0xDEAD_BEEF, 0x1234_5678),
            (u32::MAX, u32::MAX),
        ] {
            let (p, l) = unpack_result(pack_result(ptr, len));
            assert_eq!((p, l), (ptr, len));
        }
    }

    #[test]
    fn decoration_style_round_trips_through_json() {
        let ranges = vec![
            DecoratedRange {
                start: 0,
                end: 3,
                style: DecorationStyle::Underline {
                    color: "#FF0000".into(),
                    thickness_px: None,
                },
            },
            DecoratedRange {
                start: 10,
                end: 15,
                style: DecorationStyle::Underline {
                    color: "rgb(0, 128, 0)".into(),
                    thickness_px: Some(1.5),
                },
            },
        ];
        let json = serde_json::to_string(&ranges).unwrap();
        // Tag-based enum encoding: verify `kind` is present and
        // thickness_px is omitted when None.
        assert!(json.contains("\"kind\":\"underline\""));
        assert!(!json.contains("\"thickness_px\":null"));
        let decoded: Vec<DecoratedRange> = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded, ranges);
    }

    #[test]
    fn highlight_variant_round_trips() {
        let ranges = vec![DecoratedRange {
            start: 0,
            end: 5,
            style: DecorationStyle::Highlight {
                color: "yellow".into(),
            },
        }];
        let json = serde_json::to_string(&ranges).unwrap();
        assert!(json.contains("\"kind\":\"highlight\""));
        let decoded: Vec<DecoratedRange> = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded, ranges);
    }

    #[test]
    fn squiggly_variant_round_trips() {
        let ranges = vec![DecoratedRange {
            start: 0,
            end: 5,
            style: DecorationStyle::Squiggly {
                color: "#FF4040".into(),
            },
        }];
        let json = serde_json::to_string(&ranges).unwrap();
        assert!(json.contains("\"kind\":\"squiggly\""));
        let decoded: Vec<DecoratedRange> = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded, ranges);
    }

    #[test]
    fn mixed_variants_round_trip_in_one_payload() {
        let ranges = vec![
            DecoratedRange {
                start: 0,
                end: 3,
                style: DecorationStyle::Highlight {
                    color: "#FFFF00".into(),
                },
            },
            DecoratedRange {
                start: 5,
                end: 10,
                style: DecorationStyle::Underline {
                    color: "red".into(),
                    thickness_px: None,
                },
            },
            DecoratedRange {
                start: 12,
                end: 20,
                style: DecorationStyle::Squiggly {
                    color: "blue".into(),
                },
            },
        ];
        let json = serde_json::to_string(&ranges).unwrap();
        let decoded: Vec<DecoratedRange> = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded, ranges);
    }

    #[test]
    fn run_plugin_emits_json_for_matches() {
        fn circle_matcher(text: &str) -> Vec<DecoratedRange> {
            text.char_indices()
                .filter(|(_, ch)| *ch == '◯')
                .map(|(i, _)| DecoratedRange {
                    start: i,
                    end: i + '◯'.len_utf8(),
                    style: DecorationStyle::Underline {
                        color: "#FF0000".into(),
                        thickness_px: None,
                    },
                })
                .collect()
        }

        let input = "a◯b◯c".as_bytes();
        // Allocate, write, run, read, free — the host-side round trip.
        let ptr = alloc_buffer(input.len());
        assert!(!ptr.is_null());
        // SAFETY: We just allocated `input.len()` bytes at `ptr`.
        unsafe {
            std::ptr::copy_nonoverlapping(input.as_ptr(), ptr, input.len());
        }
        // SAFETY: Pointer and length match the allocation above.
        let (res_ptr, res_len) = unsafe { run_plugin(ptr, input.len(), circle_matcher) };
        assert!(!res_ptr.is_null());
        assert!(res_len > 0);

        // SAFETY: Result buffer lives until we call `free_buffer` on it.
        let result_bytes = unsafe { std::slice::from_raw_parts(res_ptr, res_len) };
        let decoded: Vec<DecoratedRange> = serde_json::from_slice(result_bytes).unwrap();
        assert_eq!(decoded.len(), 2);
        assert_eq!(decoded[0].start, 1);
        assert_eq!(decoded[0].end, 4);
        assert_eq!(decoded[1].start, 5);
        assert_eq!(decoded[1].end, 8);

        // SAFETY: Both buffers came from this crate's allocator.
        unsafe {
            free_buffer(res_ptr, res_len);
            free_buffer(ptr, input.len());
        }
    }

    #[test]
    fn run_plugin_returns_null_for_empty_or_no_match() {
        fn no_match(_text: &str) -> Vec<DecoratedRange> {
            Vec::new()
        }
        // Empty input.
        // SAFETY: null+0 is valid per the run_plugin contract.
        let (p, l) = unsafe { run_plugin(std::ptr::null(), 0, no_match) };
        assert!(p.is_null());
        assert_eq!(l, 0);

        // Non-empty input but no matches.
        let input = b"no match here";
        let ptr = alloc_buffer(input.len());
        // SAFETY: Freshly allocated buffer.
        unsafe {
            std::ptr::copy_nonoverlapping(input.as_ptr(), ptr, input.len());
        }
        // SAFETY: Buffer matches allocation.
        let (rp, rl) = unsafe { run_plugin(ptr, input.len(), no_match) };
        assert!(rp.is_null());
        assert_eq!(rl, 0);
        // SAFETY: Matches our allocation.
        unsafe { free_buffer(ptr, input.len()) };
    }

    #[test]
    fn alloc_zero_returns_null() {
        assert!(alloc_buffer(0).is_null());
        // SAFETY: null is explicitly allowed.
        unsafe { free_buffer(std::ptr::null_mut(), 0) };
        // SAFETY: len = 0 is explicitly allowed.
        unsafe { free_buffer(0x12345 as *mut u8, 0) };
    }

    #[test]
    fn run_plugin_round_trips_invalid_utf8_as_no_match() {
        fn any_match(text: &str) -> Vec<DecoratedRange> {
            vec![DecoratedRange {
                start: 0,
                end: text.len(),
                style: DecorationStyle::Underline {
                    color: "#000".into(),
                    thickness_px: None,
                },
            }]
        }
        // Lone continuation byte — not valid UTF-8.
        let input: &[u8] = &[0xC3];
        let ptr = alloc_buffer(input.len());
        // SAFETY: freshly allocated.
        unsafe {
            std::ptr::copy_nonoverlapping(input.as_ptr(), ptr, input.len());
        }
        // SAFETY: matches allocation.
        let (rp, rl) = unsafe { run_plugin(ptr, input.len(), any_match) };
        assert!(rp.is_null());
        assert_eq!(rl, 0);
        // SAFETY: matches allocation.
        unsafe { free_buffer(ptr, input.len()) };
    }
}
