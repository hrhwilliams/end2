use std::fmt::Write;
use std::sync::OnceLock;

use axum::{body::Body, extract::MatchedPath, http::Request, middleware::Next, response::Response};
use opentelemetry::{KeyValue, metrics::Histogram, propagation::TextMapPropagator};
use opentelemetry_http::HeaderExtractor;
use opentelemetry_sdk::propagation::TraceContextPropagator;
use tracing::Instrument;
use tracing_opentelemetry::OpenTelemetrySpanExt;

static DURATION_HISTOGRAM_MS: OnceLock<Histogram<f64>> = OnceLock::new();

pub async fn telemetry(request: Request<Body>, next: Next) -> Response {
    let method = request.method().to_string();
    let route = request.extensions().get::<MatchedPath>().map_or_else(
        || request.uri().path().to_owned(),
        |p| p.as_str().to_owned(),
    );

    let headers: String = request.headers().iter().fold(String::new(), |mut acc, s| {
        write!(acc, "{}:{};", s.0, s.1.to_str().expect("should be string"))
            .expect("writing to string should not fail");
        acc
    });

    let parent_ctx = TraceContextPropagator::new().extract(&HeaderExtractor(request.headers()));

    let span = tracing::info_span!(
        "http_request",
        "http.request.method" = %method,
        "http.request.headers" = %headers,
        "http.route" = %route,
    );

    let _ = span.set_parent(parent_ctx);

    let start = tokio::time::Instant::now();

    async move {
        tracing::info!("on_request");

        let response = next.run(request).await;
        let elapsed = start.elapsed();
        let duration_ms = elapsed.as_secs_f64() * 1000.0;

        tracing::info!(
            latency_us = elapsed.as_micros(),
            latency_ms = elapsed.as_millis(),
            "on_response",
        );

        let attributes = [
            KeyValue::new("http.request.method", method),
            KeyValue::new("http.route", route),
        ];

        DURATION_HISTOGRAM_MS
            .get_or_init(|| {
                opentelemetry::global::meter("end2")
                    .f64_histogram("http.request.duration.ms")
                    .with_unit("ms")
                    .with_boundaries(vec![
                        0.5, 1.0, 2.0, 3.0, 4.0, 5.0, 7.5, 10.0, 25.0, 50.0, 100.0, 250.0, 500.0,
                        1000.0,
                    ])
                    .build()
            })
            .record(duration_ms, &attributes);

        response
    }
    .instrument(span)
    .await
}
