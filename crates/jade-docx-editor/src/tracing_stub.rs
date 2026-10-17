//! No-op tracing stubs used when the `otel` feature is disabled.
//!
//! These macros expand to nothing so that `#[instrument]`, `info_span!`, and
//! `.instrument(…)` calls compile away with zero runtime cost.

/// No-op replacement for `tracing::info_span!(…)`.
///
/// Returns a [`NoopSpan`] whose `.entered()` and `Instrument` impl are
/// costless.
macro_rules! info_span {
    ($($tt:tt)*) => {
        $crate::tracing_stub::NoopSpan
    };
}

pub(crate) use info_span;

/// A zero-sized span that does nothing.
pub struct NoopSpan;

impl NoopSpan {
    /// Mimics `tracing::Span::entered()`.
    #[inline(always)]
    pub fn entered(self) -> Self {
        self
    }
}

/// No-op equivalent of `tracing::Instrument`.
///
/// Allows `future.instrument(info_span!(…))` to compile when tracing is
/// disabled.
pub trait Instrument: Sized {
    fn instrument(self, _span: NoopSpan) -> Self {
        self
    }
}

// Blanket impl so any `Future` (or in fact any type) gets the method.
impl<T> Instrument for T {}
