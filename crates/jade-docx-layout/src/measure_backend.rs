//! Runtime selection between the browser measurement path and the native one.
//!
//! Layout is measurement-driven, so "which renderer am I running under" reduces
//! to "which [`TextMeasurer`] / [`LineWrapContext`] is in play":
//!
//! - [`MeasureBackend::Host`] / [`WrapBackend::Host`] carry whatever the host
//!   supplies: the canvas-backed measurer inside `jade-docx-editor` in the browser
//!   (the existing rendering path), or `FontdueMeasurer` in headless tooling.
//! - The `Shaping` arms measure in-process with [`ShapingMeasurer`] and need no
//!   browser at all.
//!
//! The `Shaping` arms only exist with the `native-shaping` feature enabled, so
//! wasm builds never link `rustybuzz`.

use crate::font_resolver::ResolvedRunStyle;
use crate::layout_cache::TextMeasurer;
use crate::line_wrap::LineWrapContext;
use jade_docx_reader::RunProperties;
use std::fmt;
use std::rc::Rc;

#[cfg(feature = "native-shaping")]
use crate::shaping_measurer::{ShapingLineWrapContext, ShapingMeasurer};

/// Error from whichever backend is selected. Kept as a sum type so the host
/// error (`JsValue` in the browser) never has to be convertible into the
/// native one, or vice versa.
#[derive(Debug)]
pub enum MeasureBackendError<E> {
    Host(E),
    #[cfg(feature = "native-shaping")]
    Shaping(Box<dyn std::error::Error>),
}

impl<E: fmt::Display> fmt::Display for MeasureBackendError<E> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Host(error) => write!(formatter, "host measurement failed: {error}"),
            #[cfg(feature = "native-shaping")]
            Self::Shaping(error) => write!(formatter, "native shaping failed: {error}"),
        }
    }
}

impl<E: fmt::Debug + fmt::Display> std::error::Error for MeasureBackendError<E> {}

/// Selectable [`TextMeasurer`].
pub enum MeasureBackend<H> {
    /// Measurement supplied by the host — canvas `measureText` in the browser.
    Host(H),
    /// In-process OpenType shaping.
    #[cfg(feature = "native-shaping")]
    Shaping(ShapingMeasurer),
}

impl<H> MeasureBackend<H> {
    /// Name of the active backend, for logging and CLI output.
    pub fn name(&self) -> &'static str {
        match self {
            Self::Host(_) => "host",
            #[cfg(feature = "native-shaping")]
            Self::Shaping(_) => "shaping",
        }
    }
}

impl<H: TextMeasurer> TextMeasurer for MeasureBackend<H> {
    type Error = MeasureBackendError<H::Error>;

    fn measure_text(&self, text: &str, font_descriptor: &str) -> Result<f64, Self::Error> {
        match self {
            Self::Host(host) => host
                .measure_text(text, font_descriptor)
                .map_err(MeasureBackendError::Host),
            #[cfg(feature = "native-shaping")]
            Self::Shaping(measurer) => measurer
                .measure_text(text, font_descriptor)
                .map_err(MeasureBackendError::Shaping),
        }
    }

    fn measure_text_with_hash(
        &self,
        text: &str,
        font_descriptor: &str,
        font_descriptor_hash: u64,
    ) -> Result<f64, Self::Error> {
        match self {
            Self::Host(host) => host
                .measure_text_with_hash(text, font_descriptor, font_descriptor_hash)
                .map_err(MeasureBackendError::Host),
            #[cfg(feature = "native-shaping")]
            Self::Shaping(measurer) => measurer
                .measure_text_with_hash(text, font_descriptor, font_descriptor_hash)
                .map_err(MeasureBackendError::Shaping),
        }
    }
}

/// Selectable [`LineWrapContext`], the seam the wrap engine actually consumes.
pub enum WrapBackend<H> {
    /// Wrap context supplied by the host — the canvas renderer in the browser.
    Host(H),
    /// In-process OpenType shaping. Boxed to keep the enum small; the host
    /// arm is often a zero-sized handle.
    #[cfg(feature = "native-shaping")]
    Shaping(Box<ShapingLineWrapContext>),
}

impl<H> WrapBackend<H> {
    pub fn name(&self) -> &'static str {
        match self {
            Self::Host(_) => "host",
            #[cfg(feature = "native-shaping")]
            Self::Shaping(_) => "shaping",
        }
    }
}

impl<H: LineWrapContext> LineWrapContext for WrapBackend<H> {
    type Error = MeasureBackendError<H::Error>;

    fn resolve_run_style(
        &self,
        properties: Option<&RunProperties>,
    ) -> Result<Rc<ResolvedRunStyle>, Self::Error> {
        match self {
            Self::Host(host) => host
                .resolve_run_style(properties)
                .map_err(MeasureBackendError::Host),
            #[cfg(feature = "native-shaping")]
            Self::Shaping(context) => context
                .resolve_run_style(properties)
                .map_err(MeasureBackendError::Shaping),
        }
    }

    fn measure_text(&self, text: &str, font_descriptor: &str) -> Result<f64, Self::Error> {
        match self {
            Self::Host(host) => host
                .measure_text(text, font_descriptor)
                .map_err(MeasureBackendError::Host),
            #[cfg(feature = "native-shaping")]
            Self::Shaping(context) => context
                .measure_text(text, font_descriptor)
                .map_err(MeasureBackendError::Shaping),
        }
    }

    fn measure_text_with_hash(
        &self,
        text: &str,
        font_descriptor: &str,
        font_descriptor_hash: u64,
    ) -> Result<f64, Self::Error> {
        match self {
            Self::Host(host) => host
                .measure_text_with_hash(text, font_descriptor, font_descriptor_hash)
                .map_err(MeasureBackendError::Host),
            #[cfg(feature = "native-shaping")]
            Self::Shaping(context) => context
                .measure_text_with_hash(text, font_descriptor, font_descriptor_hash)
                .map_err(MeasureBackendError::Shaping),
        }
    }

    // Every `LineWrapContext` method is forwarded explicitly rather than left to
    // its default. The defaults delegate to `measure_text`, which for the `Host`
    // arm would silently discard the host's own specialisation — its vertical
    // stacking metric, or its interned-font-key fast path.

    fn measure_text_for_vertical_stack(
        &self,
        text: &str,
        font_descriptor: &str,
    ) -> Result<f64, Self::Error> {
        match self {
            Self::Host(host) => host
                .measure_text_for_vertical_stack(text, font_descriptor)
                .map_err(MeasureBackendError::Host),
            #[cfg(feature = "native-shaping")]
            Self::Shaping(context) => context
                .measure_text_for_vertical_stack(text, font_descriptor)
                .map_err(MeasureBackendError::Shaping),
        }
    }

    fn measure_text_with_font_key(
        &self,
        text: &str,
        font_descriptor: &str,
        font_descriptor_hash: u64,
        font_descriptor_id: Option<u32>,
    ) -> Result<f64, Self::Error> {
        match self {
            Self::Host(host) => host
                .measure_text_with_font_key(
                    text,
                    font_descriptor,
                    font_descriptor_hash,
                    font_descriptor_id,
                )
                .map_err(MeasureBackendError::Host),
            #[cfg(feature = "native-shaping")]
            Self::Shaping(context) => context
                .measure_text_with_font_key(
                    text,
                    font_descriptor,
                    font_descriptor_hash,
                    font_descriptor_id,
                )
                .map_err(MeasureBackendError::Shaping),
        }
    }

    fn measure_text_for_vertical_stack_with_hash(
        &self,
        text: &str,
        font_descriptor: &str,
        font_descriptor_hash: u64,
    ) -> Result<f64, Self::Error> {
        match self {
            Self::Host(host) => host
                .measure_text_for_vertical_stack_with_hash(
                    text,
                    font_descriptor,
                    font_descriptor_hash,
                )
                .map_err(MeasureBackendError::Host),
            #[cfg(feature = "native-shaping")]
            Self::Shaping(context) => context
                .measure_text_for_vertical_stack_with_hash(
                    text,
                    font_descriptor,
                    font_descriptor_hash,
                )
                .map_err(MeasureBackendError::Shaping),
        }
    }

    fn measure_text_for_vertical_stack_with_font_key(
        &self,
        text: &str,
        font_descriptor: &str,
        font_descriptor_hash: u64,
        font_descriptor_id: Option<u32>,
    ) -> Result<f64, Self::Error> {
        match self {
            Self::Host(host) => host
                .measure_text_for_vertical_stack_with_font_key(
                    text,
                    font_descriptor,
                    font_descriptor_hash,
                    font_descriptor_id,
                )
                .map_err(MeasureBackendError::Host),
            #[cfg(feature = "native-shaping")]
            Self::Shaping(context) => context
                .measure_text_for_vertical_stack_with_font_key(
                    text,
                    font_descriptor,
                    font_descriptor_hash,
                    font_descriptor_id,
                )
                .map_err(MeasureBackendError::Shaping),
        }
    }

    fn measure_trailing_overhang(
        &self,
        text: &str,
        font_descriptor: &str,
    ) -> Result<f64, Self::Error> {
        match self {
            Self::Host(host) => host
                .measure_trailing_overhang(text, font_descriptor)
                .map_err(MeasureBackendError::Host),
            #[cfg(feature = "native-shaping")]
            Self::Shaping(context) => context
                .measure_trailing_overhang(text, font_descriptor)
                .map_err(MeasureBackendError::Shaping),
        }
    }

    fn measure_line_box_height(&self, font_descriptor: &str) -> Result<Option<f64>, Self::Error> {
        match self {
            Self::Host(host) => host
                .measure_line_box_height(font_descriptor)
                .map_err(MeasureBackendError::Host),
            #[cfg(feature = "native-shaping")]
            Self::Shaping(context) => context
                .measure_line_box_height(font_descriptor)
                .map_err(MeasureBackendError::Shaping),
        }
    }

    fn measure_font_vertical_metrics(
        &self,
        font_descriptor: &str,
    ) -> Result<Option<(f64, f64)>, Self::Error> {
        match self {
            Self::Host(host) => host
                .measure_font_vertical_metrics(font_descriptor)
                .map_err(MeasureBackendError::Host),
            #[cfg(feature = "native-shaping")]
            Self::Shaping(context) => context
                .measure_font_vertical_metrics(font_descriptor)
                .map_err(MeasureBackendError::Shaping),
        }
    }
}
