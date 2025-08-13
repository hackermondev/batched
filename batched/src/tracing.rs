pub use tracing::*;

pub trait TracingSpan {
    fn link_span(&mut self, span: &Span);
}

impl<N> TracingSpan for N {
    default fn link_span(&mut self, _span: &Span) {}
}

#[cfg(feature = "tracing_opentelemetry")]
impl<T: tracing_opentelemetry::OpenTelemetrySpanExt> TracingSpan for T {
    fn link_span(&mut self, span: &Span) {
        self.add_link(
            opentelemetry::trace::TraceContextExt::span(
                &tracing_opentelemetry::OpenTelemetrySpanExt::context(span),
            )
            .span_context()
            .clone(),
        );
    }
}
