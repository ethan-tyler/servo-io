window.BENCHMARK_DATA = {
  "lastUpdate": 1764466618426,
  "repoUrl": "https://github.com/ethan-tyler/servo-io",
  "entries": {
    "Rust Benchmark": [
      {
        "commit": {
          "author": {
            "email": "ethan@urbanskitech.com",
            "name": "Ethan Urbanski",
            "username": "ethan-tyler"
          },
          "committer": {
            "email": "noreply@github.com",
            "name": "GitHub",
            "username": "web-flow"
          },
          "distinct": true,
          "id": "6169214197fee210cda959d1cff6674274012d73",
          "message": "Feat/phase2 config secrets (#39)\n\n* feat(tracing): add span instrumentation for queue, retry, and storage (Phase 3B.3)\n\nAdds comprehensive distributed tracing spans across the Servo platform:\n\n**Cloud Tasks Queue (servo-cloud-gcp/src/queue.rs)**\n- Add spans for enqueue(), create_cloud_task(), and call_cloud_tasks_api()\n- Track task_name, attempts, and http.status_code in span fields\n- Enable end-to-end trace visibility through Cloud Tasks\n\n**Retry Logic (servo-runtime/src/retry.rs)**\n- Add span for execute_with_retry() with attempt tracking\n- Record max_attempts, strategy, attempts, and total_duration_ms\n- Enables visibility into retry behavior in production\n\n**Concurrency Control (servo-runtime/src/concurrency.rs)**\n- Add span for acquire() semaphore operations\n- Track max_concurrent, available_before, and wait_duration_ms\n- Helps identify concurrency bottlenecks\n\n**Database Operations (servo-storage/src/postgres.rs)**\n- Add OTel semantic conventions to existing instrumented methods\n- Include db.system, db.operation, db.sql.table attributes\n- Enhanced tracing for: create_asset, get_asset, create_workflow,\n  create_execution, create_execution_or_get_existing, update_workflow,\n  update_execution, get_asset_lineage\n\nAll spans follow OpenTelemetry semantic conventions for consistent\ntrace analysis in Cloud Trace and other backends.\n\n* feat(tracing): add PII/secret filtering for span attributes (Phase 3B.4)\n\nAdd sensitive data filtering to prevent secrets from being exported\nto Cloud Trace:\n\n- SensitiveDataMatcher: Pattern-based key/value detection with allowlist\n- SensitiveDataFilteringExporter: Wraps OTLP exporter to filter spans\n- Sample rate validation: Clamp SERVO_TRACE_SAMPLE_RATE to [0.0, 1.0]\n\nKey patterns redacted: password, secret, token, bearer, authorization,\napi_key, hmac, signature, oidc, jwt, credential, connection_string\n\nValue patterns redacted: Bearer tokens, JWTs (ey...), GitHub tokens,\nStripe keys, Google OAuth tokens, database connection strings\n\nSafe fields allowlisted: execution_id, tenant_id, db.operation,\nhttp.status_code, and other OTel semantic conventions\n\n* feat(metrics): add Prometheus metrics for workflow execution (Phase 3C)\n\nAdd comprehensive Prometheus metrics for observability:\n- servo_workflow_executions_total: execution counts by status/tenant\n- servo_workflow_duration_seconds: execution duration histogram\n- servo_workflow_asset_count: assets per workflow histogram\n- servo_rate_limit_rejections_total: rate limiter rejections\n- servo_active_tenant_limiters: active tenant limiter gauge\n- servo_http_requests_total: HTTP request counts\n- servo_http_request_duration_seconds: request latency histogram\n\nIntegration:\n- Metrics recorded in executor on success/failure/timeout\n- Rate limiter records rejections (skipped in tests)\n- init_metrics() called at startup to pre-register all metrics\n- /metrics endpoint already exists via prometheus::gather()\n\n* docs: add commit guidelines for branding\n\n* ci: run servo-worker tests with --test-threads=1\n\nTests that manipulate environment variables (tracing_config, rate_limiter)\nneed to run single-threaded to avoid race conditions between tests.\n\n* feat(python): add Python SDK foundation (Phase 0)\n\nImplement core Python SDK for Servo with asset-centric data orchestration:\n\n- @asset decorator with name, dependencies, metadata, groups\n- @workflow decorator with cron schedules, timeout, retries\n- ServoClient and AsyncServoClient for API interaction\n- Comprehensive validation:\n  - Duplicate name detection for assets/workflows\n  - Dependency validation (validate_dependencies, validate_dependencies_strict)\n  - Enhanced cron validation with field range checking\n- Type-safe with full annotations and py.typed marker\n- 58 tests with pytest, ruff linting, mypy strict mode\n- Python 3.9-3.12 support\n- CI workflow for automated testing\n\n* chore: rename config section comment\n\n* feat(testing): add comprehensive testing infrastructure (Phase 1)\n\n- Add servo-tests crate with shared test utilities:\n  - Fixtures module with factories for Workflow, Execution, Asset, TaskPayload\n  - Builders module with type-safe ExecutionRequestBuilder, WorkflowBuilder\n  - Mocks module with MockJwksServer for OIDC testing, HMAC helpers\n  - Assertions module with ResponseAssertions and schema validation\n\n- Add contract tests for API boundaries:\n  - servo-worker: 34 tests for task payload, signature, responses, headers\n  - servo-cloud-gcp: 26 tests for Cloud Tasks API format and signatures\n\n- Add chaos engineering tests (servo-storage):\n  - Circuit breaker tests: threshold, half-open, concurrent probes, recovery\n  - Timeout tests: cancellation, nesting, concurrency, cleanup safety\n\n- Add criterion benchmarks for lineage queries:\n  - Graph construction (chain, fan-out patterns)\n  - Upstream/downstream queries at various scales\n  - Cycle detection and impact analysis\n  - Basic graph operations\n\nTotal: 97 new tests + 42 benchmark groups\n\n* feat(config): add unified config and secrets management (Phase 2)\n\n- Add RuntimeEnvironment detection module:\n  - Automatic detection via K_SERVICE (Cloud Run) and SERVO_ENVIRONMENT\n  - Production, Staging, Development variants\n  - Determines secret source (Secret Manager vs env vars)\n\n- Add SecretsProvider abstraction:\n  - GCP Secret Manager integration for production/staging\n  - Environment variable fallback for development\n  - Support for secret rotation (multiple valid secrets)\n  - Automatic refresh and validation\n\n- Update servo-worker integration:\n  - Handler uses SecretsProvider for HMAC validation\n  - Main initializes secrets based on environment\n  - Added .env.example with all configuration options\n\n- Enhance servo-cloud-gcp auth module:\n  - Improved OIDC token validation\n  - Better error handling and logging\n\nEnables secure, environment-aware configuration without code changes.\n\n* feat(observability): add comprehensive observability infrastructure (Phase 3)\n\nSLO/Error Budget Foundation:\n- Add formal SLO specifications following Google SRE best practices\n- Define SLIs for HTTP availability, workflow availability, latency, data quality\n- Create multi-window burn rate alerts (14.4x fast burn, 3x slow burn)\n- Add SLO overview Grafana dashboard with error budget gauges\n\nDistributed Tracing:\n- Enhance executor with OpenTelemetry span instrumentation\n- Add otel.name attributes for better trace visualization\n- Support multiple trace exporters (OTLP, Jaeger, stdout)\n- Add Jaeger endpoint configuration for local development\n\nLog-Trace Correlation:\n- Add JSON logging layer with full span context\n- Enable trace_id propagation in structured logs\n- Add defensive config validation with clear error messages\n\nMTTR-Focused Metrics:\n- Add circuit breaker state and transition metrics\n- Add execution queue depth and wait time metrics\n- Add error detection time metrics\n- Add dependency health and latency metrics\n\nIncident Response:\n- Create 10 comprehensive runbooks for common alert scenarios\n- Include triage steps, resolution procedures, escalation paths\n- Document detection queries and recovery verification\n\nLocal Development:\n- Add docker-compose.dev.yml with Jaeger and PostgreSQL\n- Add optional Prometheus/Grafana with --profile metrics\n- Create prometheus.dev.yml for local metric scraping\n\n* feat(quality): add data quality checks infrastructure (Phase 4)\n\nCore Library:\n- Add quality module with Check, CheckDefinition, CheckResult types\n- Define check outcomes (Passed, Failed, Skipped, Error)\n- Support comparison operators and threshold-based validations\n\nStorage Layer:\n- Add asset_checks table migration with proper indexing\n- Implement CheckRepository with CRUD operations\n- Add batch insert for check results with conflict handling\n- Support querying checks by execution, asset, or tenant\n\nPython SDK:\n- Add quality module with Check and CheckResult classes\n- Extend ServoClient with check management methods\n- Add comprehensive type definitions for check operations\n- Include unit and integration tests for quality features\n\nWorker Components:\n- Add check_validator module for runtime validation\n- Add PII detection utilities for sensitive data handling\n- Export new modules from worker lib\n\nE2E Tests:\n- Add comprehensive end-to-end tests for check lifecycle\n- Test check creation, querying, and result recording\n\n* fix: resolve CI linting and formatting issues\n\n- Apply cargo fmt formatting to Rust files\n- Sort __all__ in Python servo/__init__.py\n- Remove unused inspect import from quality.py\n- Add type parameter to generic list in exceptions.py\n- Fix type annotations for _InlineCheckCallable\n- Prefix unused test parameters with underscore\n- Use ternary operator in deploy_checks\n\n* style: apply ruff formatting to Python files\n\n* fix: resolve additional clippy warnings\n\n- Use derive(Default) with #[default] attribute for CheckSeverity\n- Use Range::contains() for status code checks\n- Fix const assertion in contract tests",
          "timestamp": "2025-11-29T11:50:19-05:00",
          "tree_id": "c093f6d2729bb8021cfc38d5b0cb1bf5151b181f",
          "url": "https://github.com/ethan-tyler/servo-io/commit/6169214197fee210cda959d1cff6674274012d73"
        },
        "date": 1764435769322,
        "tool": "cargo",
        "benches": [
          {
            "name": "graph_construction/chain/10",
            "value": 6314,
            "range": "± 47",
            "unit": "ns/iter"
          },
          {
            "name": "graph_construction/fan_out/10",
            "value": 6957,
            "range": "± 99",
            "unit": "ns/iter"
          },
          {
            "name": "graph_construction/chain/100",
            "value": 67541,
            "range": "± 2195",
            "unit": "ns/iter"
          },
          {
            "name": "graph_construction/fan_out/100",
            "value": 67839,
            "range": "± 457",
            "unit": "ns/iter"
          },
          {
            "name": "graph_construction/chain/1000",
            "value": 676944,
            "range": "± 9156",
            "unit": "ns/iter"
          },
          {
            "name": "graph_construction/fan_out/1000",
            "value": 669715,
            "range": "± 7205",
            "unit": "ns/iter"
          },
          {
            "name": "upstream_queries/chain_last_node/10",
            "value": 43,
            "range": "± 0",
            "unit": "ns/iter"
          },
          {
            "name": "upstream_queries/chain_mid_node/10",
            "value": 43,
            "range": "± 0",
            "unit": "ns/iter"
          },
          {
            "name": "upstream_queries/chain_last_node/100",
            "value": 45,
            "range": "± 0",
            "unit": "ns/iter"
          },
          {
            "name": "upstream_queries/chain_mid_node/100",
            "value": 44,
            "range": "± 0",
            "unit": "ns/iter"
          },
          {
            "name": "upstream_queries/chain_last_node/500",
            "value": 43,
            "range": "± 0",
            "unit": "ns/iter"
          },
          {
            "name": "upstream_queries/chain_mid_node/500",
            "value": 43,
            "range": "± 2",
            "unit": "ns/iter"
          },
          {
            "name": "upstream_queries/single_upstream/10",
            "value": 43,
            "range": "± 0",
            "unit": "ns/iter"
          },
          {
            "name": "upstream_queries/single_upstream/100",
            "value": 43,
            "range": "± 0",
            "unit": "ns/iter"
          },
          {
            "name": "upstream_queries/single_upstream/500",
            "value": 43,
            "range": "± 0",
            "unit": "ns/iter"
          },
          {
            "name": "downstream_queries/fan_out/10",
            "value": 119,
            "range": "± 2",
            "unit": "ns/iter"
          },
          {
            "name": "downstream_queries/fan_out/100",
            "value": 421,
            "range": "± 5",
            "unit": "ns/iter"
          },
          {
            "name": "downstream_queries/fan_out/500",
            "value": 1760,
            "range": "± 11",
            "unit": "ns/iter"
          },
          {
            "name": "downstream_queries/chain_first/10",
            "value": 44,
            "range": "± 0",
            "unit": "ns/iter"
          },
          {
            "name": "downstream_queries/chain_first/100",
            "value": 44,
            "range": "± 0",
            "unit": "ns/iter"
          },
          {
            "name": "downstream_queries/chain_first/500",
            "value": 44,
            "range": "± 0",
            "unit": "ns/iter"
          },
          {
            "name": "cycle_detection/chain_no_cycle/10",
            "value": 144,
            "range": "± 3",
            "unit": "ns/iter"
          },
          {
            "name": "cycle_detection/chain_no_cycle/100",
            "value": 1269,
            "range": "± 34",
            "unit": "ns/iter"
          },
          {
            "name": "cycle_detection/chain_no_cycle/500",
            "value": 6538,
            "range": "± 36",
            "unit": "ns/iter"
          },
          {
            "name": "cycle_detection/chain_no_cycle/1000",
            "value": 13362,
            "range": "± 93",
            "unit": "ns/iter"
          },
          {
            "name": "cycle_detection/diamond_layers/3",
            "value": 442,
            "range": "± 8",
            "unit": "ns/iter"
          },
          {
            "name": "cycle_detection/diamond_layers/5",
            "value": 1005,
            "range": "± 9",
            "unit": "ns/iter"
          },
          {
            "name": "cycle_detection/diamond_layers/7",
            "value": 1590,
            "range": "± 46",
            "unit": "ns/iter"
          },
          {
            "name": "impact_analysis/fan_out_source/10",
            "value": 130,
            "range": "± 0",
            "unit": "ns/iter"
          },
          {
            "name": "impact_analysis/fan_out_source/100",
            "value": 515,
            "range": "± 3",
            "unit": "ns/iter"
          },
          {
            "name": "impact_analysis/fan_out_source/500",
            "value": 1837,
            "range": "± 14",
            "unit": "ns/iter"
          },
          {
            "name": "impact_analysis/chain_first/10",
            "value": 54,
            "range": "± 0",
            "unit": "ns/iter"
          },
          {
            "name": "impact_analysis/chain_first/100",
            "value": 54,
            "range": "± 0",
            "unit": "ns/iter"
          },
          {
            "name": "impact_analysis/chain_first/500",
            "value": 54,
            "range": "± 0",
            "unit": "ns/iter"
          },
          {
            "name": "impact_analysis/diamond_source/3",
            "value": 133,
            "range": "± 0",
            "unit": "ns/iter"
          },
          {
            "name": "impact_analysis/diamond_source/5",
            "value": 126,
            "range": "± 0",
            "unit": "ns/iter"
          },
          {
            "name": "impact_analysis/diamond_source/7",
            "value": 143,
            "range": "± 3",
            "unit": "ns/iter"
          },
          {
            "name": "graph_operations/add_single_node",
            "value": 534,
            "range": "± 2",
            "unit": "ns/iter"
          },
          {
            "name": "graph_operations/add_edge",
            "value": 82,
            "range": "± 3",
            "unit": "ns/iter"
          },
          {
            "name": "graph_operations/node_count/100",
            "value": 0,
            "range": "± 0",
            "unit": "ns/iter"
          },
          {
            "name": "graph_operations/node_count/1000",
            "value": 0,
            "range": "± 0",
            "unit": "ns/iter"
          },
          {
            "name": "graph_operations/node_count/10000",
            "value": 0,
            "range": "± 0",
            "unit": "ns/iter"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "email": "ethan@urbanskitech.com",
            "name": "Ethan Urbanski",
            "username": "ethan-tyler"
          },
          "committer": {
            "email": "noreply@github.com",
            "name": "GitHub",
            "username": "web-flow"
          },
          "distinct": true,
          "id": "0d2f860f7a448778212fe21859147353dcec62c7",
          "message": "feat: Advanced data platform features (Phases 2-5) (#41)\n\n* feat(quality): add referential integrity and schema match checks (Phase 5C)\n\nImplement two deferred check types to achieve full Python SDK parity:\n\n- Add referential_integrity check for foreign key validation\n  - Supports all 3 API patterns (decorator, fluent, stacked)\n  - Validates column values exist in reference data\n\n- Add schema_match check for column validation\n  - Validates expected columns exist\n  - Supports allow_extra_columns option\n  - Auto-detects pandas/polars column types\n\n- Add SchemaColumn dataclass and type mappings\n- Add helper functions for column value/schema extraction\n- Extend CheckDefinition.to_rust_check_type() for new checks\n- Add comprehensive unit tests (18 new test cases)\n- Add integration tests for Rust schema generation\n\n* feat(partitions): add advanced partitioning support (Phase 5B)\n\nImplement Dagster-compatible partition definitions for multi-dimensional,\ntime-based, static, and dynamic partitioning.\n\nPartition Types:\n- DailyPartition, HourlyPartition, WeeklyPartition, MonthlyPartition\n- StaticPartition for fixed key lists (e.g., regions)\n- DynamicPartition for runtime-discovered keys\n- MultiPartition for multi-dimensional Cartesian products\n\nPartition Mappings:\n- IdentityMapping for 1:1 upstream/downstream mapping\n- TimeWindowMapping for time-based aggregations (e.g., daily -> weekly)\n- AllUpstreamMapping for full history dependencies\n- DimensionMapping for multi-partition dimension remapping\n\nIntegration:\n- Added partition and partition_mappings parameters to @asset decorator\n- Extended AssetDefinition with partition metadata\n- Added PartitionContext for partition-aware execution\n- Added serialization/deserialization utilities\n\nTests:\n- Comprehensive unit tests for all partition types (53 new tests)\n- Mapping resolution tests\n- Serialization round-trip tests\n\n* feat(worker): add referential_integrity and schema_match check validators\n\nAdd backend validation support for two check types from the Python SDK:\n\n- ReferentialIntegrity: validates foreign key references exist\n  - Supports in-memory validation when reference data is provided\n  - Gracefully handles server-side registration when no reference data\n  - Skips null values by default\n\n- SchemaMatch: validates expected columns are present\n  - Supports allow_extra_columns option\n  - Reports missing and unexpected columns\n\nThis ensures the Rust worker can properly validate checks registered\nvia the Python SDK's check.referential_integrity() and check.schema_match()\ndecorators.\n\nTests: 28 passing check_validator tests including new coverage for both types\n\n* docs(partitions): clarify SDK-only status for partitioning feature\n\nAdd note to module docstring explaining that partitioning is currently\navailable for asset definition and metadata purposes only. Full runtime\nand scheduler support for partition-aware execution is planned for a\nfuture release.\n\n* style: fix formatting issues in Python and Rust files\n\nApply automatic formatting:\n- cargo fmt for Rust files\n- ruff format for Python files",
          "timestamp": "2025-11-29T20:26:19-05:00",
          "tree_id": "3cad79824a13b71feb8d7c15617efd566ab82ddd",
          "url": "https://github.com/ethan-tyler/servo-io/commit/0d2f860f7a448778212fe21859147353dcec62c7"
        },
        "date": 1764466618026,
        "tool": "cargo",
        "benches": [
          {
            "name": "graph_construction/chain/10",
            "value": 6260,
            "range": "± 60",
            "unit": "ns/iter"
          },
          {
            "name": "graph_construction/fan_out/10",
            "value": 6971,
            "range": "± 59",
            "unit": "ns/iter"
          },
          {
            "name": "graph_construction/chain/100",
            "value": 66824,
            "range": "± 140",
            "unit": "ns/iter"
          },
          {
            "name": "graph_construction/fan_out/100",
            "value": 67715,
            "range": "± 293",
            "unit": "ns/iter"
          },
          {
            "name": "graph_construction/chain/1000",
            "value": 669563,
            "range": "± 4289",
            "unit": "ns/iter"
          },
          {
            "name": "graph_construction/fan_out/1000",
            "value": 669888,
            "range": "± 1670",
            "unit": "ns/iter"
          },
          {
            "name": "upstream_queries/chain_last_node/10",
            "value": 44,
            "range": "± 0",
            "unit": "ns/iter"
          },
          {
            "name": "upstream_queries/chain_mid_node/10",
            "value": 44,
            "range": "± 0",
            "unit": "ns/iter"
          },
          {
            "name": "upstream_queries/chain_last_node/100",
            "value": 45,
            "range": "± 0",
            "unit": "ns/iter"
          },
          {
            "name": "upstream_queries/chain_mid_node/100",
            "value": 44,
            "range": "± 1",
            "unit": "ns/iter"
          },
          {
            "name": "upstream_queries/chain_last_node/500",
            "value": 44,
            "range": "± 0",
            "unit": "ns/iter"
          },
          {
            "name": "upstream_queries/chain_mid_node/500",
            "value": 46,
            "range": "± 0",
            "unit": "ns/iter"
          },
          {
            "name": "upstream_queries/single_upstream/10",
            "value": 43,
            "range": "± 0",
            "unit": "ns/iter"
          },
          {
            "name": "upstream_queries/single_upstream/100",
            "value": 43,
            "range": "± 1",
            "unit": "ns/iter"
          },
          {
            "name": "upstream_queries/single_upstream/500",
            "value": 43,
            "range": "± 0",
            "unit": "ns/iter"
          },
          {
            "name": "downstream_queries/fan_out/10",
            "value": 122,
            "range": "± 8",
            "unit": "ns/iter"
          },
          {
            "name": "downstream_queries/fan_out/100",
            "value": 491,
            "range": "± 5",
            "unit": "ns/iter"
          },
          {
            "name": "downstream_queries/fan_out/500",
            "value": 1838,
            "range": "± 4",
            "unit": "ns/iter"
          },
          {
            "name": "downstream_queries/chain_first/10",
            "value": 45,
            "range": "± 1",
            "unit": "ns/iter"
          },
          {
            "name": "downstream_queries/chain_first/100",
            "value": 44,
            "range": "± 0",
            "unit": "ns/iter"
          },
          {
            "name": "downstream_queries/chain_first/500",
            "value": 45,
            "range": "± 0",
            "unit": "ns/iter"
          },
          {
            "name": "cycle_detection/chain_no_cycle/10",
            "value": 145,
            "range": "± 10",
            "unit": "ns/iter"
          },
          {
            "name": "cycle_detection/chain_no_cycle/100",
            "value": 1248,
            "range": "± 34",
            "unit": "ns/iter"
          },
          {
            "name": "cycle_detection/chain_no_cycle/500",
            "value": 6527,
            "range": "± 438",
            "unit": "ns/iter"
          },
          {
            "name": "cycle_detection/chain_no_cycle/1000",
            "value": 13584,
            "range": "± 358",
            "unit": "ns/iter"
          },
          {
            "name": "cycle_detection/diamond_layers/3",
            "value": 451,
            "range": "± 5",
            "unit": "ns/iter"
          },
          {
            "name": "cycle_detection/diamond_layers/5",
            "value": 1019,
            "range": "± 38",
            "unit": "ns/iter"
          },
          {
            "name": "cycle_detection/diamond_layers/7",
            "value": 1627,
            "range": "± 21",
            "unit": "ns/iter"
          },
          {
            "name": "impact_analysis/fan_out_source/10",
            "value": 131,
            "range": "± 0",
            "unit": "ns/iter"
          },
          {
            "name": "impact_analysis/fan_out_source/100",
            "value": 483,
            "range": "± 10",
            "unit": "ns/iter"
          },
          {
            "name": "impact_analysis/fan_out_source/500",
            "value": 1826,
            "range": "± 34",
            "unit": "ns/iter"
          },
          {
            "name": "impact_analysis/chain_first/10",
            "value": 54,
            "range": "± 0",
            "unit": "ns/iter"
          },
          {
            "name": "impact_analysis/chain_first/100",
            "value": 53,
            "range": "± 1",
            "unit": "ns/iter"
          },
          {
            "name": "impact_analysis/chain_first/500",
            "value": 54,
            "range": "± 0",
            "unit": "ns/iter"
          },
          {
            "name": "impact_analysis/diamond_source/3",
            "value": 136,
            "range": "± 1",
            "unit": "ns/iter"
          },
          {
            "name": "impact_analysis/diamond_source/5",
            "value": 129,
            "range": "± 4",
            "unit": "ns/iter"
          },
          {
            "name": "impact_analysis/diamond_source/7",
            "value": 146,
            "range": "± 5",
            "unit": "ns/iter"
          },
          {
            "name": "graph_operations/add_single_node",
            "value": 550,
            "range": "± 8",
            "unit": "ns/iter"
          },
          {
            "name": "graph_operations/add_edge",
            "value": 83,
            "range": "± 2",
            "unit": "ns/iter"
          },
          {
            "name": "graph_operations/node_count/100",
            "value": 0,
            "range": "± 0",
            "unit": "ns/iter"
          },
          {
            "name": "graph_operations/node_count/1000",
            "value": 0,
            "range": "± 0",
            "unit": "ns/iter"
          },
          {
            "name": "graph_operations/node_count/10000",
            "value": 0,
            "range": "± 0",
            "unit": "ns/iter"
          }
        ]
      }
    ]
  }
}