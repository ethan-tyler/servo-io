# Phase 3A: Circuit Breakers + Rate Limiting

## Summary

Implements production-ready resilience patterns to protect Servo against cascading failures and abuse:

- **Circuit Breakers** for all external dependencies (PostgreSQL, Cloud Tasks, Secret Manager, OAuth2)
- **Rate Limiting** for worker endpoints (per-tenant and IP-based)
- Comprehensive configuration validation and documentation

## Changes

### Circuit Breakers

Async-native circuit breaker implementation protecting all external dependencies:

**New Files:**
- `servo-storage/src/circuit_breaker.rs` - Core circuit breaker implementation
- `servo-storage/src/metrics.rs` - Prometheus metrics for circuit breakers
- `servo-cloud-gcp/src/circuit_breaker.rs` - Error classification for GCP APIs

**Protected Dependencies:**
1. **PostgreSQL** (`servo-storage/src/postgres.rs`) - Connection pool protection
2. **Cloud Tasks API** (`servo-cloud-gcp/src/queue.rs`) - Task enqueueing protection
3. **Secret Manager API** (`servo-cloud-gcp/src/secrets.rs`) - Secret fetching protection
4. **OAuth2 Token Endpoint** (`servo-cloud-gcp/src/auth.rs`) - Token acquisition protection

**Features:**
- Consecutive failures policy (default: 5 failures trip breaker)
- Half-open state with guard to prevent concurrent recovery probes
- Error classification: trip on 5xx/429/network errors, don't trip on 4xx config errors
- Prometheus metrics with low-cardinality labels (dependency only)
- Per-dependency configuration via environment variables

**Configuration:**
```bash
# PostgreSQL circuit breaker
export SERVO_CB_POSTGRES_FAILURE_THRESHOLD=5        # Default: 5
export SERVO_CB_POSTGRES_HALF_OPEN_TIMEOUT_SECS=30  # Default: 30

# Cloud Tasks circuit breaker
export SERVO_CB_CLOUD_TASKS_FAILURE_THRESHOLD=5
export SERVO_CB_CLOUD_TASKS_HALF_OPEN_TIMEOUT_SECS=30

# Secret Manager circuit breaker
export SERVO_CB_SECRET_MANAGER_FAILURE_THRESHOLD=5
export SERVO_CB_SECRET_MANAGER_HALF_OPEN_TIMEOUT_SECS=30

# OAuth2 circuit breaker
export SERVO_CB_OAUTH2_FAILURE_THRESHOLD=5
export SERVO_CB_OAUTH2_HALF_OPEN_TIMEOUT_SECS=30
```

**Metrics:**
```promql
# Circuit breaker state (0=closed, 1=open)
servo_circuit_breaker_state{dependency="postgres|cloud_tasks|secret_manager|oauth2"}

# Total circuit opens
servo_circuit_breaker_opens_total{dependency="..."}

# Half-open probe attempts
servo_circuit_breaker_half_open_attempts_total{dependency="...", result="success|failure"}
```

### Rate Limiting

Dual-layer rate limiting to protect worker endpoints:

**New Files:**
- `servo-worker/src/rate_limiter.rs` - Per-tenant and IP-based rate limiters
- `servo-worker/README.md` - Comprehensive worker documentation

**Modified Files:**
- `servo-worker/src/handler.rs` - Integrated rate limiting into execute and metrics handlers
- `servo-worker/src/main.rs` - Rate limiter initialization
- `servo-worker/Cargo.toml` - Added dependencies (tower_governor, governor, dashmap)

**Features:**

1. **Per-Tenant Rate Limiting** (on `/execute` endpoint):
   - Applied after OIDC/HMAC validation, before expensive work
   - Default: 10 requests/second per tenant
   - Returns 429 with `Retry-After: 1` header
   - DashMap-based per-instance state (documented caveat)

2. **IP-Based Rate Limiting** (on `/metrics` endpoint):
   - Early check to prevent scraper abuse
   - Default: 60 requests/minute per IP with burst allowance
   - Returns 429 with `Retry-After: 60` header

**Configuration:**
```bash
# Per-tenant rate limiting
export SERVO_RATE_LIMIT_TENANT_RPS=10   # Default: 10 requests/second per tenant

# IP-based rate limiting for metrics
export SERVO_RATE_LIMIT_METRICS_RPM=60  # Default: 60 requests/minute per IP
```

**Config Validation:**
- Rejects zero values with clear panic messages
- Logs effective configuration at startup
- Falls back to sensible defaults

**Multi-Instance Caveat:**
DashMap state is per-worker-instance. For true global rate limits across multiple Cloud Run instances, a centralized store (Redis/Memorystore) would be required. This is documented prominently in the README.

### Documentation

**New Documentation:**
- `servo-worker/README.md` - Complete worker documentation covering:
  - Architecture and overview
  - Configuration (rate limiting, OIDC, circuit breakers)
  - All endpoints (/execute, /health, /ready, /metrics)
  - Security considerations (metrics protection, HMAC management)
  - Deployment guide for Cloud Run
  - Monitoring and alerting recommendations

**Updated Documentation:**
- `servo-cloud-gcp/README.md` - Added circuit breaker configuration section with:
  - Configuration examples for all dependencies
  - Error classification explanation
  - Metrics reference
  - Recommendations for production deployments

## Testing

**New Tests:**
- Circuit breaker tests in `servo-storage/src/circuit_breaker.rs`
- Rate limiter tests in `servo-worker/src/rate_limiter.rs` (9 tests)
  - Tenant isolation tests
  - Over-limit blocking tests
  - IP-based limiting tests
  - Config validation tests (zero rejection, positive value acceptance)

**Updated Tests:**
- `servo-worker/tests/oidc_validation_test.rs` - Added rate limiter initialization to test fixtures

**Test Results:**
```
✅ 38 tests passing in servo-worker
✅ 9 rate limiter tests (including 3 new validation tests)
✅ Circuit breaker tests across all modules
```

## Security Considerations

1. **Metrics Endpoint Protection:**
   - Documented three protection approaches (Bearer token, internal-only, IP allowlisting)
   - IP-based rate limiting prevents scraper abuse
   - Optional Bearer token authentication via `SERVO_METRICS_TOKEN`

2. **Circuit Breaker Error Classification:**
   - 5xx, 429, and network errors trip the breaker (transient failures)
   - 4xx errors don't trip the breaker (config/auth issues should fail fast)
   - Prevents cascading failures while maintaining fast failure for misconfigurations

3. **Rate Limiting Integration Order:**
   - Applied after authentication (OIDC + HMAC validation)
   - Before expensive work (database queries, workflow execution)
   - Proper 429 responses with RFC-compliant Retry-After headers

4. **Metrics Cardinality Control:**
   - Circuit breaker metrics use only `dependency` label (4 values max)
   - No per-tenant labels on rate limit counters (avoids cardinality explosion)
   - Structured logging with tenant_id/IP for debugging

## Breaking Changes

None. All changes are additive with sensible defaults.

## Migration Guide

No migration required. All features are opt-in via environment variables with safe defaults:

- Circuit breakers: Enabled by default with conservative thresholds
- Rate limiting: Enabled by default with permissive limits (10 req/s tenant, 60 req/min IP)

For production deployments, consider:
- Adjusting rate limits based on expected load (`SERVO_RATE_LIMIT_TENANT_RPS`)
- Enabling metrics endpoint authentication (`SERVO_METRICS_TOKEN`)
- Monitoring circuit breaker metrics to detect dependency issues

## Commits

```
95dbd80 feat(worker): add config validation for rate limiters
8b3f60f docs(worker): add comprehensive README with rate limiting documentation
34f49df feat(worker): implement production-ready rate limiting
ca8242f docs(cloud-gcp): add circuit breaker configuration documentation
b8d7184 feat(cloud-gcp): add circuit breaker for OAuth2 token endpoint
c90aec3 feat(cloud-gcp): add circuit breaker for Secret Manager API
f7b0d5d feat(cloud-gcp): add circuit breaker for Cloud Tasks API
ee0d086 refactor(storage): enhance circuit breaker with production safeguards
53df58a feat(storage): add async-native circuit breaker implementation
```

## Files Changed

```
M  Cargo.lock
M  servo-cloud-gcp/README.md
M  servo-cloud-gcp/src/auth.rs
A  servo-cloud-gcp/src/circuit_breaker.rs
M  servo-cloud-gcp/src/lib.rs
M  servo-cloud-gcp/src/queue.rs
M  servo-cloud-gcp/src/secrets.rs
M  servo-storage/Cargo.toml
A  servo-storage/src/circuit_breaker.rs
M  servo-storage/src/lib.rs
A  servo-storage/src/metrics.rs
M  servo-worker/Cargo.toml
A  servo-worker/README.md
M  servo-worker/src/handler.rs
M  servo-worker/src/lib.rs
M  servo-worker/src/main.rs
A  servo-worker/src/rate_limiter.rs
M  servo-worker/tests/oidc_validation_test.rs
```

## Checklist

- [x] Code follows project style guidelines
- [x] Self-review completed
- [x] Comments added for complex logic
- [x] Documentation updated (READMEs, inline docs)
- [x] Tests added/updated
- [x] All tests passing locally
- [x] No breaking changes (or documented in migration guide)
- [x] Security considerations addressed

## Related Issues

Part of Phase 2 production-ready features (ROADMAP.md):
- [x] Circuit breakers
- [x] Rate limiting

## Next Steps

After this PR merges, Phase 3B will add:
- **Distributed Tracing** (OpenTelemetry with W3C tracecontext)
- Trace propagation through Cloud Tasks and worker
- Cloud Trace exporter configuration
- PII/secret filtering in spans
