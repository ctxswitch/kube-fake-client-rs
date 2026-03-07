# OpenAPI vs kube.rs vs kube-fake-client Matrix

This document compares behavior across:

- Kubernetes swagger / kube-apiserver semantics (`kubernetes/api/openapi/swagger.json`)
- `kube.rs` client behavior (currently `kube-core 3.0.1` in this repo)
- `kube-fake-client` HTTP behavior (`src/mock_service.rs`)

Snapshot date: 2026-03-07.

## Compatibility Matrix

| Behavior | Swagger / kube-apiserver | kube.rs (3.0.1) | kube-fake-client (current) | Gap |
|---|---|---|---|---|
| SSA `fieldManager` required | Required for apply patch | `PatchParams::apply("mgr")` sets it; request can include `fieldManager` | Enforced for apply patch, missing/empty => 422 | None for this rule |
| `force` only for apply patch | Must be unset for non-apply patch | Client-side validation rejects `force` with non-apply patch | Parsed from query; no server-side guard for non-apply | If callers bypass kube.rs validation, fake is looser than apiserver |
| SSA managedFields conflict semantics | Conflicts when ownership overlaps unless forced | Exposes `force`; ownership semantics enforced by apiserver | Implemented: conflict on overlap with `force=false`, transfer with `force=true` | Mostly aligned |
| `fieldValidation` (PATCH) | Supported (`Ignore`/`Warn`/`Strict`) | Supported on `PatchParams.field_validation` and serialized to query | Not parsed or acted on | Missing in fake |
| `fieldValidation` (POST/PUT) | Supported for mutating requests | No `field_validation` in `PostParams` | Not parsed or acted on | Missing in kube.rs surface and fake |
| `dryRun` (POST/PUT/PATCH/DELETE) | Supported (`dryRun=All` / body for delete options) | Supported (`PostParams`, `PatchParams`, `DeleteParams`) | Not enforced; operations persist | Missing in fake |
| `labelSelector` list filtering | Supported | Supported via `ListParams` | Implemented | None |
| `fieldSelector` list filtering | Supported (resource-specific) | Supported via `ListParams` | Implemented via pre-registered field extraction | Mostly aligned for supported fields |
| `limit` + `continue` pagination contract | Full chunked pagination and continue token semantics | Supports limit/continue params | `limit` truncates locally; `continue` parsed but ignored | Missing real pagination semantics |
| `resourceVersion` / `resourceVersionMatch` (list) | Supported with consistency semantics | Supported in `ListParams` + validation | `resourceVersion` parsed but ignored; `resourceVersionMatch` not parsed | Missing in fake |
| `timeoutSeconds` (list/watch) | Supported | Supported in list/watch params | Parsed in list params, not enforced | Partial in fake |
| `watch` / `allowWatchBookmarks` / `sendInitialEvents` | Supported | Supported in `WatchParams` | Not implemented in mock_service | Missing in fake |
| `propagationPolicy` + `orphanDependents` (delete) | Supported; mutually exclusive; valid enum values | Exposed via `DeleteParams.propagation_policy` (legacy orphan not first-class) | Implemented: parses body + query, query precedence, enum validation, mutual exclusivity check | Partial: FG vs BG behavior collapsed to same cascade boolean |
| `gracePeriodSeconds` (delete) | Supported | Exposed in `DeleteParams.grace_period_seconds` | Ignored | Missing in fake |
| Delete body decode errors | Invalid body should fail request | Sends body from `DeleteParams` | Malformed body returns 400 | Aligned |
| Delete preconditions | Supported (`uid`, `resourceVersion`) | Exposed in `DeleteParams.preconditions` | Not enforced in delete handler | Missing in fake |

## Gap Details

1. `fieldValidation` handling is missing in fake HTTP handlers.
- `PATCH` can carry it from kube.rs, but fake currently ignores it.
- `POST/PUT` handling is also absent; kube.rs `PostParams` does not expose `field_validation`.

2. `dryRun` is not honored by fake for mutating calls.
- kube.rs can emit dry-run options, but fake persists changes anyway.

3. List pagination/consistency is only partially implemented.
- `limit` is a local truncate.
- `continue`, `resourceVersion`, `resourceVersionMatch` semantics are not implemented.

4. Watch API path is unsupported.
- kube.rs can request watch/bookmarks/initial events.
- fake service currently only implements CRUD-style request handling.

5. Delete behavior is improved but still not full fidelity.
- Good: query+body parsing for propagation/orphan, enum checks, mutual exclusivity, malformed-body 400.
- Missing: `gracePeriodSeconds`, preconditions, and distinct foreground/background lifecycle behavior.

## Key Evidence

- Fake client HTTP behavior: `src/mock_service.rs`
- Fake client SSA ownership logic: `src/managed_fields.rs`
- Fake client tests: `src/mock_service_test.rs`, `src/managed_fields_test.rs`
- kube.rs parameter behavior: `~/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/kube-core-3.0.1/src/params.rs`
- kube.rs request-side validation hooks: `~/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/kube-core-3.0.1/src/request.rs`

## Out Of Scope (Intentional)

Client-level discovery helpers in `kube.rs` (for example `Client::list_api_groups`, `Client::list_core_api_versions`, and related aggregated discovery calls) are intentionally out of scope for this matrix. This analysis is focused on typed resource request semantics and fake-server behavior parity for CRUD/list/watch/patch/delete paths.
