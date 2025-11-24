# servo-worker

Cloud Run worker for Servo, providing secure workflow execution with OIDC validation and comprehensive rate limiting.

## Overview

The `servo-worker` crate implements a Cloud Run service that executes workflows triggered by Google Cloud Tasks:

- **OIDC Validation**: Verifies requests are from authenticated Google Cloud Tasks using JWT tokens
- **HMAC Payload Signing**: Ensures task payload integrity with signature verification
- **Rate Limiting**: Per-tenant and IP-based rate limiting to prevent abuse
- **Background Execution**: Immediate 200 OK response with async workflow execution
- **Observability**: Prometheus metrics, structured logging, health/readiness probes

## Architecture

```
Cloud Tasks (OIDC token) → [POST /execute] → OIDC Validation → HMAC Verification
                                             ↓
                                    Rate Limiting (per-tenant)
                                             ↓
                                    Background Task Spawn → Workflow Execution
                                             ↓
                                    200 OK (immediate)
```

## Configuration

### Required Environment Variables

```bash
# Database connection
export DATABASE_URL="postgresql://user:pass@localhost:5432/servo"

# HMAC secret for payload verification (min 32 bytes)
export SERVO_HMAC_SECRET="your-strong-random-secret-min-32-bytes"

# OIDC validation (for Cloud Tasks authentication)
export SERVO_OIDC_ENABLED=true
export SERVO_OIDC_AUDIENCE="https://your-worker.run.app"  # Your Cloud Run service URL
export SERVO_OIDC_ISSUER="https://accounts.google.com"
```

### Optional Environment Variables

```bash
# Server configuration
export PORT=8080                     # HTTP server port (default: 8080)
export EXECUTION_TIMEOUT=600         # Workflow execution timeout in seconds (default: 600)

# Rate limiting
export SERVO_RATE_LIMIT_TENANT_RPS=10   # Per-tenant requests/second (default: 10)
export SERVO_RATE_LIMIT_METRICS_RPM=60  # Metrics endpoint requests/minute per IP (default: 60)

# Metrics endpoint authentication (optional but recommended)
export SERVO_METRICS_TOKEN="your-bearer-token"

# Logging
export RUST_LOG=info                 # Log level (trace, debug, info, warn, error)
```

### Rate Limiting

The worker implements two layers of rate limiting:

#### Per-Tenant Rate Limiting

Applied to the `/execute` endpoint after authentication but before expensive work:

```bash
export SERVO_RATE_LIMIT_TENANT_RPS=10  # Default: 10 requests/second per tenant
```

**Behavior**:
- Enforced per `tenant_id` extracted from the task payload
- Applied after OIDC and HMAC validation
- Returns 429 with `Retry-After: 1` header when exceeded
- Structured log: `tenant_id`, `execution_id`, `error` fields

**Multi-Instance Caveat**: Rate limiting state is stored in-memory using DashMap, which is **per-worker-instance**.
If you run multiple Cloud Run instances, each maintains its own rate limit state. For true global rate limits across
all instances, a centralized store (Redis, Memorystore) would be required.

#### IP-Based Rate Limiting

Applied to the `/metrics` endpoint to prevent scraper abuse:

```bash
export SERVO_RATE_LIMIT_METRICS_RPM=60  # Default: 60 requests/minute per IP
```

**Behavior**:
- Enforced per client IP address
- Early check before any expensive operations
- Returns 429 with `Retry-After: 60` header when exceeded
- Includes burst allowance (30 requests by default)

**Why IP-based for metrics**: Metrics scraping doesn't have tenant context, so IP-based limiting prevents
individual IPs from overwhelming the endpoint.

### OIDC Validation

OIDC validation verifies that requests to `/execute` come from Google Cloud Tasks with a valid identity token.

**Configuration**:

```bash
# Enable OIDC validation (recommended for production)
export SERVO_OIDC_ENABLED=true

# Audience: Your Cloud Run service URL
export SERVO_OIDC_AUDIENCE="https://servo-worker-abc123-uc.a.run.app"

# Issuer: Google's OIDC issuer
export SERVO_OIDC_ISSUER="https://accounts.google.com"
```

**How it works**:

1. Cloud Tasks includes `Authorization: Bearer <jwt>` header with each request
2. Worker validates JWT signature using Google's public keys (JWKS)
3. Validates issuer, audience, and expiration
4. Caches JWKS keys with automatic refresh

**Disable for development** (NOT recommended for production):

```bash
export SERVO_OIDC_ENABLED=false
```

**Security**: When disabled, anyone with your Cloud Run URL can trigger workflow executions. Always enable in production.

### Circuit Breaker Configuration

The worker inherits circuit breaker protection for PostgreSQL connections from `servo-storage`:

```bash
# PostgreSQL circuit breaker
export SERVO_CB_POSTGRES_FAILURE_THRESHOLD=5        # Default: 5
export SERVO_CB_POSTGRES_HALF_OPEN_TIMEOUT_SECS=30  # Default: 30
```

See [servo-storage documentation](../servo-storage/README.md) for details on database circuit breaker behavior.

## Endpoints

### `POST /execute`

Execute a workflow task (called by Cloud Tasks).

**Authentication**: OIDC token + HMAC signature

**Headers**:
- `Authorization: Bearer <oidc-token>` (if OIDC enabled)
- `X-Servo-Signature: <hex-encoded-hmac-sha256>`

**Body**: Base64-encoded JSON payload

**Response**: 200 OK (immediate), execution happens in background

**Rate Limiting**: Per-tenant (10 req/s by default)

### `GET /health`

Liveness probe - returns 200 OK if the service is running.

**No authentication required** (liveness checks should always succeed)

### `GET /ready`

Readiness probe - returns 200 OK if the service can accept traffic.

**Checks**:
- Database connectivity
- Token acquisition capability (if configured)

**No authentication required**

### `GET /metrics`

Prometheus metrics endpoint.

**Authentication**: Optional Bearer token (set via `SERVO_METRICS_TOKEN`)

**Rate Limiting**: IP-based (60 req/min by default)

**Response**: Prometheus text format

## Security Considerations

### Metrics Endpoint Protection

**CRITICAL**: The `/metrics` endpoint exposes operational metrics and should be protected in production.

#### Option 1: Bearer Token Authentication (Recommended)

```bash
# Set metrics token
export SERVO_METRICS_TOKEN="$(openssl rand -base64 32)"

# Configure Prometheus to include the token
scrape_configs:
  - job_name: 'servo-worker'
    bearer_token: 'your-metrics-token-here'
    static_configs:
      - targets: ['servo-worker.run.app:443']
```

#### Option 2: Internal-Only Access

Restrict `/metrics` to internal traffic:

```bash
# Cloud Run: Use ingress settings
gcloud run services update servo-worker \
  --ingress=internal  # Only accessible within VPC

# Or use VPC ingress settings
gcloud run services update servo-worker \
  --ingress=internal-and-cloud-load-balancing
```

#### Option 3: IP Allowlisting

Use Cloud Armor or Load Balancer rules to restrict access to known monitoring IPs.

### HMAC Secret Management

**Recommended**: Use GCP Secret Manager for HMAC secrets (see [servo-cloud-gcp README](../servo-cloud-gcp/README.md)).

**Minimum Requirements**:
- At least 32 bytes long
- Cryptographically random
- Rotated regularly (Secret Manager supports zero-downtime rotation)

## Deployment

### Cloud Run Deployment

```bash
# Build container
docker build -t gcr.io/my-project/servo-worker:latest .

# Push to GCR
docker push gcr.io/my-project/servo-worker:latest

# Deploy to Cloud Run
gcloud run deploy servo-worker \
  --image=gcr.io/my-project/servo-worker:latest \
  --platform=managed \
  --region=us-central1 \
  --set-env-vars="DATABASE_URL=postgresql://...,SERVO_OIDC_ENABLED=true,SERVO_OIDC_AUDIENCE=https://servo-worker-abc.run.app" \
  --set-secrets="SERVO_HMAC_SECRET=servo-hmac-secret:latest" \
  --service-account=servo-worker@my-project.iam.gserviceaccount.com \
  --max-instances=100 \
  --concurrency=80 \
  --cpu=2 \
  --memory=2Gi \
  --timeout=610s  # Slightly longer than EXECUTION_TIMEOUT
```

### Recommended Production Settings

```bash
# Rate limiting for high-volume tenants
export SERVO_RATE_LIMIT_TENANT_RPS=50

# Rate limiting for metrics (allow frequent scraping)
export SERVO_RATE_LIMIT_METRICS_RPM=120

# Circuit breaker (conservative defaults)
export SERVO_CB_POSTGRES_FAILURE_THRESHOLD=5
export SERVO_CB_POSTGRES_HALF_OPEN_TIMEOUT_SECS=30

# Execution timeout (10 minutes)
export EXECUTION_TIMEOUT=600

# Structured JSON logging
export RUST_LOG=info
```

## Monitoring

### Key Metrics

```promql
# Execution requests
servo_execute_requests_total{status="success|failure"}

# Rate limiting
servo_rate_limit_exceeded_total{endpoint="/execute|/metrics"}

# Circuit breaker state
servo_circuit_breaker_state{dependency="postgres"}

# OIDC validation
servo_oidc_validation_total{result="success|failure"}
```

### Alerts

Recommended alerts:

```yaml
# High rate limit rejection rate
- alert: HighRateLimitRejections
  expr: rate(servo_rate_limit_exceeded_total[5m]) > 10
  annotations:
    summary: "High rate limit rejection rate"

# Circuit breaker open
- alert: CircuitBreakerOpen
  expr: servo_circuit_breaker_state{dependency="postgres"} == 1
  annotations:
    summary: "Database circuit breaker is open"

# OIDC validation failures
- alert: HighOidcFailures
  expr: rate(servo_oidc_validation_total{result="failure"}[5m]) > 1
  annotations:
    summary: "High OIDC validation failure rate"
```

## Testing

```bash
# Run unit tests
cargo test -p servo-worker

# Run integration tests (requires PostgreSQL)
export TEST_DATABASE_URL="postgresql://localhost:5432/servo_test"
cargo test -p servo-worker --test '*'
```

## Documentation

For more details, see:
- [GCP Deployment Guide](https://docs.servo.dev/guides/deployment-gcp)
- [Security Best Practices](../SECURITY.md)
- [Circuit Breaker Documentation](../servo-storage/README.md)
