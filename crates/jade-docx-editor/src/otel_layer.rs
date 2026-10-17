//! OpenTelemetry bridge layer for tracing.
//!
//! This module provides a `tracing_subscriber::Layer` implementation that
//! forwards span lifecycle events to JavaScript's OpenTelemetry SDK via
//! wasm-bindgen bindings.

use std::fmt;
use tracing::span::Attributes;
use tracing::{Event, Id, Subscriber};
use tracing_subscriber::layer::Context;
use tracing_subscriber::registry::LookupSpan;
use tracing_subscriber::Layer;

use wasm_bindgen::prelude::*;

// JavaScript bindings for OpenTelemetry API
#[wasm_bindgen]
extern "C" {
    #[wasm_bindgen(js_name = otelStartSpan)]
    fn otel_start_span(name: &str, parent_id: Option<u32>) -> u32;

    #[wasm_bindgen(js_name = otelEndSpan)]
    fn otel_end_span(id: u32);

    #[wasm_bindgen(js_name = otelSetSpanAttr)]
    fn otel_set_span_attr(id: u32, key: &str, value: &str);

    #[wasm_bindgen(js_name = otelGetActiveSpanId)]
    fn otel_get_active_span_id() -> Option<u32>;

    #[wasm_bindgen(js_name = otelPushActiveSpan)]
    fn otel_push_active_span(id: u32);

    #[wasm_bindgen(js_name = otelPopActiveSpan)]
    fn otel_pop_active_span();
}

/// Stores the JavaScript-side span handle.
/// When dropped, automatically ends the span on the JS side.
#[derive(Debug)]
struct JsSpanHandle(u32);

impl Drop for JsSpanHandle {
    fn drop(&mut self) {
        otel_end_span(self.0);
    }
}

/// A `tracing_subscriber::Layer` that bridges Rust's `tracing` to JavaScript
/// OpenTelemetry via wasm-bindgen.
#[derive(Debug, Default)]
pub struct JsOtelLayer;

impl<S> Layer<S> for JsOtelLayer
where
    S: Subscriber + for<'a> LookupSpan<'a>,
{
    fn enabled(&self, _metadata: &tracing::Metadata<'_>, _ctx: Context<'_, S>) -> bool {
        true
    }

    fn on_new_span(&self, attrs: &Attributes<'_>, id: &Id, ctx: Context<'_, S>) {
        let span = ctx.span(id).expect("span not found");
        let name = span.metadata().name();

        // Get parent span ID from JS context for context propagation
        let parent_id = otel_get_active_span_id();
        let span_id = otel_start_span(name, parent_id);
        let handle = JsSpanHandle(span_id);

        // Store JS span handle in the span's extensions
        span.extensions_mut().insert(handle);

        // Record initial attributes using the span_id we already have
        let mut visitor = SpanAttrVisitor { span_id };
        attrs.record(&mut visitor);
    }

    fn on_enter(&self, id: &Id, ctx: Context<'_, S>) {
        // When entering a span, push it onto the active span stack
        if let Some(span) = ctx.span(id) {
            if let Some(handle) = span.extensions().get::<JsSpanHandle>() {
                otel_push_active_span(handle.0);
            }
        }
    }

    fn on_exit(&self, id: &Id, ctx: Context<'_, S>) {
        // When exiting a span, pop it from the active span stack
        if let Some(span) = ctx.span(id) {
            if span.extensions().get::<JsSpanHandle>().is_some() {
                otel_pop_active_span();
            }
        }
    }

    fn on_record(&self, id: &Id, values: &tracing::span::Record<'_>, ctx: Context<'_, S>) {
        if let Some(span) = ctx.span(id) {
            if let Some(handle) = span.extensions().get::<JsSpanHandle>() {
                let mut visitor = SpanAttrVisitor { span_id: handle.0 };
                values.record(&mut visitor);
            }
        }
    }

    fn on_close(&self, _id: Id, _ctx: Context<'_, S>) {
        // Span ending is handled by JsSpanHandle::drop
        // The handle will be dropped when the span's extensions are cleaned up
    }

    fn on_event(&self, event: &Event<'_>, ctx: Context<'_, S>) {
        // Log events as attributes on the current span
        if let Some(scope) = ctx.lookup_current() {
            if let Some(handle) = scope.extensions().get::<JsSpanHandle>() {
                let mut visitor = SpanAttrVisitor { span_id: handle.0 };
                event.record(&mut visitor);
            }
        }
    }
}

/// Visitor that forwards span fields to JavaScript as span attributes.
struct SpanAttrVisitor {
    span_id: u32,
}

impl tracing::field::Visit for SpanAttrVisitor {
    fn record_str(&mut self, field: &tracing::field::Field, value: &str) {
        otel_set_span_attr(self.span_id, field.name(), value);
    }

    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn fmt::Debug) {
        otel_set_span_attr(self.span_id, field.name(), &format!("{value:?}"));
    }

    fn record_i64(&mut self, field: &tracing::field::Field, value: i64) {
        otel_set_span_attr(self.span_id, field.name(), &value.to_string());
    }

    fn record_u64(&mut self, field: &tracing::field::Field, value: u64) {
        otel_set_span_attr(self.span_id, field.name(), &value.to_string());
    }

    fn record_bool(&mut self, field: &tracing::field::Field, value: bool) {
        otel_set_span_attr(self.span_id, field.name(), &value.to_string());
    }
}

use std::sync::atomic::{AtomicBool, Ordering};

static TRACING_INITIALIZED: AtomicBool = AtomicBool::new(false);

/// Initializes the tracing subscriber with the JsOtelLayer.
/// Call this once at application startup.
pub fn init_tracing() {
    // Ensure we only initialize once
    if TRACING_INITIALIZED.swap(true, Ordering::SeqCst) {
        return;
    }

    use tracing_subscriber::prelude::*;

    tracing_subscriber::registry().with(JsOtelLayer).init();
}
