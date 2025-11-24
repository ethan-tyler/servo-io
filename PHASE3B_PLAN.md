# Phase 3B: Distributed Tracing (OpenTelemetry)

## Goal

Add end-to-end distributed tracing with OpenTelemetry to enable request flow visibility across the Servo platform (orchestrator → Cloud Tasks → worker → execution).

## Scope

### 1. OpenTelemetry Integration with W3C Trace Context

**Components:**
- `servo-runtime` - Add tracing to orchestrator operations
- `servo-cloud-gcp` - Propagate trace context through Cloud Tasks
- `servo-worker` - Extract trace context and create worker spans
- `servo-storage` - Add database operation spans

**Key Requirements:**
- W3C trace context propagation via HTTP headers (`traceparent`, `tracestate`)
- Span hierarchy: orchestrator → queue enqueue → worker receive → execution
- Automatic instrumentation for HTTP requests, database queries

### 2. Trace Context Propagation

**Cloud Tasks Integration:**
```rust
// In servo-cloud-gcp/src/queue.rs
// When enqueueing task, inject trace context into:
// 1. HTTP headers (for worker to extract)
// 2. Task payload metadata (backup/logging)

use opentelemetry::propagation::Injector;
use opentelemetry::global;

// Create task with trace context
let mut headers = HashMap::new();
let propagator = global::get_text_map_propagator();
propagator.inject(&mut HeaderInjector(&mut headers));

// Add traceparent/tracestate to Cloud Tasks HTTP request
```

**Worker Context Extraction:**
```rust
// In servo-worker/src/handler.rs
// Extract trace context from incoming request

use opentelemetry::propagation::Extractor;
use opentelemetry::global;

// Extract parent context from headers
let propagator = global::get_text_map_propagator();
let parent_ctx = propagator.extract(&HeaderExtractor(headers));

// Create span with parent context
let span = tracer
    .span_builder("workflow_execution")
    .with_parent(parent_ctx)
    .start();
```

### 3. Sampling Strategy

**Configuration:**
```bash
# Sampling rates (0.0 to 1.0)
export SERVO_TRACE_SAMPLE_RATE=0.01        # Production: 1%
export SERVO_TRACE_SAMPLE_RATE=1.0         # Staging: 100%

# Environment-based defaults
# - production: 0.01 (1%)
# - staging: 1.0 (100%)
# - development: 1.0 (100%)
```

**Sampler Implementation:**
```rust
use opentelemetry::sdk::trace::Sampler;
use opentelemetry::sdk::trace::SamplerResult;

// Use parent-based sampler with fallback to probability
let sampler = Sampler::ParentBased(Box::new(
    Sampler::TraceIdRatioBased(sample_rate)
));
```

**Goals:**
- Keep production overhead low (1% sampling)
- Full traces in staging/development for debugging
- Respect parent sampling decision (consistent traces)

### 4. PII and Secret Filtering

**Span Attribute Filtering:**
```rust
// Custom span processor to scrub sensitive data
use opentelemetry::sdk::trace::SpanProcessor;

struct SensitiveDataFilter;

impl SpanProcessor for SensitiveDataFilter {
    fn on_start(&self, span: &mut Span, _cx: &Context) {
        // Remove sensitive attributes
        const SENSITIVE_KEYS: &[&str] = &[
            "hmac_secret",
            "oidc_token",
            "authorization",
            "password",
            "api_key",
            "bearer_token",
        ];

        // Scrub or redact
        for key in SENSITIVE_KEYS {
            if span.attributes.contains_key(key) {
                span.attributes.insert(key.to_string(), "[REDACTED]");
            }
        }
    }
}
```

**SQL Query Sanitization:**
- Use OpenTelemetry semantic conventions
- Avoid logging query parameters (may contain sensitive data)
- Log only sanitized query structure

### 5. Cloud Trace Exporter Configuration

**Dependencies:**
```toml
# servo-worker/Cargo.toml
[dependencies]
opentelemetry = "0.21"
opentelemetry_sdk = "0.21"
opentelemetry-otlp = "0.14"
opentelemetry-semantic-conventions = "0.13"
tonic = "0.10"  # For gRPC export to Cloud Trace
```

**Exporter Setup:**
```rust
use opentelemetry::global;
use opentelemetry_sdk::trace::TracerProvider;
use opentelemetry_otlp::WithExportConfig;

// Cloud Trace exporter via OTLP/gRPC
let exporter = opentelemetry_otlp::new_exporter()
    .tonic()
    .with_endpoint("https://cloudtrace.googleapis.com/v2/projects/{project_id}/traces:batchWrite")
    .with_timeout(Duration::from_secs(10));

let provider = TracerProvider::builder()
    .with_batch_exporter(exporter, runtime::Tokio)
    .with_sampler(sampler)
    .with_processor(SensitiveDataFilter)
    .build();

global::set_tracer_provider(provider);
```

**Configuration:**
```bash
# Cloud Trace exporter
export SERVO_TRACE_ENABLED=true                      # Enable tracing (default: false)
export SERVO_TRACE_EXPORTER=cloudtrace               # cloudtrace|jaeger|stdout
export SERVO_TRACE_ENDPOINT=https://cloudtrace...    # Auto-detected for Cloud Run
export GCP_PROJECT_ID=my-project                     # For Cloud Trace
```

### 6. Span Instrumentation

**Key Spans to Add:**

**Orchestrator (servo-runtime):**
```rust
// In orchestrator.rs
#[tracing::instrument(
    name = "orchestrator.enqueue_execution",
    skip(self),
    fields(
        execution_id = %execution_id,
        workflow_id = %workflow_id,
        tenant_id = %tenant_id,
    )
)]
async fn enqueue_execution(...) {
    // Existing logic
}
```

**Cloud Tasks Queue (servo-cloud-gcp):**
```rust
// In queue.rs
#[tracing::instrument(
    name = "cloud_tasks.enqueue",
    skip(self),
    fields(
        execution_id = %execution_id,
        task_name = Empty,
    )
)]
async fn enqueue(...) {
    // Inject trace context into headers
    // Existing logic

    // Record task name in span
    tracing::Span::current().record("task_name", &task_name);
}
```

**Worker Execution (servo-worker):**
```rust
// In handler.rs - execute_handler
let span = tracing::info_span!(
    "worker.execute_workflow",
    execution_id = %execution_id,
    workflow_id = %payload.workflow_id,
    tenant_id = %payload.tenant_id,
    asset_count = payload.execution_plan.len(),
);

async move {
    // Extract parent trace context
    // Existing execution logic
}.instrument(span).await
```

**Database Operations (servo-storage):**
```rust
// In postgres.rs
#[tracing::instrument(
    name = "db.query",
    skip(self, query),
    fields(
        db.system = "postgresql",
        db.operation = "SELECT",  // or INSERT, UPDATE
    )
)]
async fn execute_query(&self, query: &str) {
    // Existing logic
}
```

### 7. Documentation

**Add to servo-worker/README.md:**

```markdown
### Distributed Tracing

Servo supports distributed tracing with OpenTelemetry to track request flow across components.

**Configuration:**

\`\`\`bash
# Enable tracing
export SERVO_TRACE_ENABLED=true

# Sampling rate (0.0 to 1.0)
export SERVO_TRACE_SAMPLE_RATE=0.01  # 1% in production

# Exporter (cloudtrace, jaeger, stdout)
export SERVO_TRACE_EXPORTER=cloudtrace
export GCP_PROJECT_ID=my-project
\`\`\`

**Cloud Trace Setup:**

1. Enable Cloud Trace API:
   \`\`\`bash
   gcloud services enable cloudtrace.googleapis.com
   \`\`\`

2. Grant IAM permissions:
   \`\`\`bash
   gcloud projects add-iam-policy-binding my-project \
     --member="serviceAccount:servo-worker@my-project.iam.gserviceaccount.com" \
     --role="roles/cloudtrace.agent"
   \`\`\`

3. Deploy worker with tracing enabled:
   \`\`\`bash
   gcloud run deploy servo-worker \
     --set-env-vars="SERVO_TRACE_ENABLED=true,SERVO_TRACE_SAMPLE_RATE=0.01"
   \`\`\`

**Viewing Traces:**

View traces in Cloud Console:
\`\`\`
https://console.cloud.google.com/traces/list?project=my-project
\`\`\`

Filter by execution_id, workflow_id, or tenant_id attributes.

**PII Protection:**

Tracing automatically redacts sensitive attributes:
- HMAC secrets
- OIDC tokens
- Authorization headers
- Passwords and API keys
\`\`\`

### 8. Testing Strategy

**Unit Tests:**
- Trace context injection/extraction
- Span attribute filtering (PII redaction)
- Sampler configuration

**Integration Tests:**
- End-to-end trace propagation (mock Cloud Tasks)
- Verify trace IDs match across components
- Ensure sampling is respected

**Manual Testing:**
- Deploy to staging with 100% sampling
- Trigger workflow execution
- Verify traces appear in Cloud Trace console
- Validate span hierarchy and timing

## Implementation Plan

### Phase 3B.1: Core OpenTelemetry Integration
- [ ] Add OpenTelemetry dependencies to workspace Cargo.toml
- [ ] Implement trace provider initialization in servo-worker/src/main.rs
- [ ] Add tracing configuration from environment variables
- [ ] Implement Cloud Trace exporter with GCP authentication

### Phase 3B.2: Trace Context Propagation
- [ ] Add trace context injection to Cloud Tasks queue (servo-cloud-gcp/src/queue.rs)
- [ ] Add trace context extraction in worker handler (servo-worker/src/handler.rs)
- [ ] Implement W3C propagator for traceparent/tracestate headers
- [ ] Test trace ID propagation end-to-end

### Phase 3B.3: Span Instrumentation
- [ ] Add spans to orchestrator operations (servo-runtime/src/orchestrator.rs)
- [ ] Add spans to Cloud Tasks operations (servo-cloud-gcp/src/queue.rs)
- [ ] Add spans to worker execution (servo-worker/src/handler.rs, executor.rs)
- [ ] Add spans to database operations (servo-storage/src/postgres.rs)

### Phase 3B.4: Sampling and Filtering
- [ ] Implement configurable sampling strategy
- [ ] Add PII/secret filtering span processor
- [ ] Test sampling at different rates
- [ ] Verify sensitive data is redacted

### Phase 3B.5: Documentation and Testing
- [ ] Update servo-worker/README.md with tracing setup
- [ ] Add Cloud Trace IAM setup documentation
- [ ] Write unit tests for trace propagation
- [ ] Write integration tests for end-to-end tracing
- [ ] Manual testing in staging environment

## Success Criteria

- [ ] Trace context propagates from orchestrator → Cloud Tasks → worker
- [ ] Spans appear in Cloud Trace console with correct hierarchy
- [ ] Sampling works correctly (1% in prod, 100% in staging)
- [ ] No PII or secrets leak into span attributes
- [ ] Performance overhead <5% with 1% sampling
- [ ] Documentation enables users to set up Cloud Trace
- [ ] Tests verify trace propagation and filtering

## Configuration Summary

```bash
# Tracing
export SERVO_TRACE_ENABLED=true                    # Enable tracing (default: false)
export SERVO_TRACE_SAMPLE_RATE=0.01                # Sampling rate 0.0-1.0 (default: 0.01)
export SERVO_TRACE_EXPORTER=cloudtrace             # cloudtrace|jaeger|stdout (default: cloudtrace)
export GCP_PROJECT_ID=my-project                   # GCP project for Cloud Trace

# Environment-specific defaults
# Production: SERVO_TRACE_SAMPLE_RATE=0.01 (1%)
# Staging: SERVO_TRACE_SAMPLE_RATE=1.0 (100%)
# Development: SERVO_TRACE_SAMPLE_RATE=1.0 (100%)
```

## Dependencies

**New Cargo.toml dependencies:**
```toml
opentelemetry = { version = "0.21", features = ["trace"] }
opentelemetry_sdk = { version = "0.21", features = ["rt-tokio"] }
opentelemetry-otlp = { version = "0.14", features = ["grpc-tonic", "trace"] }
opentelemetry-semantic-conventions = "0.13"
tonic = "0.10"
```

## Timeline Estimate

- Phase 3B.1: Core integration - 4 hours
- Phase 3B.2: Propagation - 3 hours
- Phase 3B.3: Instrumentation - 4 hours
- Phase 3B.4: Sampling/filtering - 3 hours
- Phase 3B.5: Docs/testing - 3 hours

**Total: ~17 hours** (2-3 working days)

## Open Questions

1. Should we add custom trace exporters beyond Cloud Trace (Jaeger, Zipkin)?
   - **Recommendation**: Start with Cloud Trace only; add others if requested

2. Should we trace every database query or batch them?
   - **Recommendation**: Trace high-level operations (enqueue, execute) not individual queries

3. What span attributes should be mandatory vs optional?
   - **Recommendation**: Always include execution_id, workflow_id, tenant_id; others optional

4. Should we add trace ID to all log messages?
   - **Recommendation**: Yes, via tracing-opentelemetry to correlate logs with traces

## References

- [OpenTelemetry Rust Documentation](https://docs.rs/opentelemetry/latest/opentelemetry/)
- [Cloud Trace Documentation](https://cloud.google.com/trace/docs)
- [W3C Trace Context Specification](https://www.w3.org/TR/trace-context/)
- [OpenTelemetry Semantic Conventions](https://opentelemetry.io/docs/specs/semconv/)
