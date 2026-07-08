# Apply Patch Tool Design

## Context

Wyse needs an `apply_patch` runtime tool compatible with OpenAI's apply patch
tool operation model. The tool must edit files through Wyse's virtual
filesystem boundary instead of touching host paths directly.

The current workspace already has:

- `wyse-filesystem`: virtual paths, a `Filesystem` trait, and a sandboxed local
  backend.
- `wyse-tools`: runtime tool traits, a builtin registry, and an `EchoTool`.
- `.github/workflows/rust.yml`: Rust formatting, clippy, unit tests, image
  checks, and an integration job for crate-owned container tests.

`docker.test` in this design means crate-owned container test assets and CI
integration for `wyse-tools`, not an agent-callable runtime tool.

## Goals

- Add reusable apply-patch support on top of `wyse-filesystem`.
- Add a `wyse-tools` builtin `ApplyPatchTool` that uses `Filesystem`.
- Keep path validation and host-path isolation inside the filesystem boundary.
- Return OpenAI-style patch results with `completed` or `failed` status.
- Add `wyse-tools` container integration test assets and wire them into GitHub
  Actions.

## Non-Goals

- No multi-file transaction semantics.
- No backups, snapshots, approval workflow, or rollback.
- No shell execution.
- No direct host-path editing.
- No agent-callable `docker.test` runtime tool.
- No automatic registration of a filesystem-writing tool in
  `BuiltinToolRegistry`.

## Architecture

`wyse-filesystem` remains the lower-level shared crate. It owns patch operation
types, diff application, and all reads and writes through the existing
`Filesystem` trait. It must not know about agents, LLM providers, tool
registries, or provider-specific runtime loops.

`wyse-tools` owns the runtime tool wrapper. `ApplyPatchTool` receives JSON tool
input, deserializes it into filesystem patch operations, calls
`wyse-filesystem`, and returns a `ToolOutput` JSON payload. Callers must
construct this tool with an explicit `Arc<dyn Filesystem>` so write capability
is opt-in.

The CI integration lives in `.github/workflows/rust.yml`. It should add
`wyse-tools` integration-test startup, test execution, and cleanup steps in the
existing integration job, following the `wyse-infra` pattern.

## Filesystem API Shape

`wyse-filesystem` should add a focused patch module, for example
`src/apply_patch.rs`, with public types similar to:

- `ApplyPatchOperation`
- `ApplyPatchOperationKind`
- `ApplyPatchStatus`
- `ApplyPatchOutput`
- `ApplyPatchError`
- `apply_patch(filesystem, operation)`

Operation variants:

- `create_file`: apply a V4A create diff to empty content, then write the new
  file.
- `update_file`: read existing file content, apply a V4A update diff, then
  write the updated file.
- `delete_file`: remove the target file.

Paths entering this crate are `VirtualPath`. The OpenAI-style relative path
normalization belongs at the tool boundary before constructing `VirtualPath`.

The patch implementation should support the OpenAI apply patch V4A diff shape
needed for create and update operations. It should return a conflict error when
context does not match the current file.

## Tool API Shape

`wyse-tools` should add a builtin `ApplyPatchTool` in a module separate from the
registry and echo implementation.

Input JSON shape:

```json
{
  "operation": {
    "type": "update_file",
    "path": "src/lib.rs",
    "diff": "@@\n-old\n+new\n"
  }
}
```

Accepted operation types:

- `create_file`
- `update_file`
- `delete_file`

Path normalization:

- `src/lib.rs` becomes `/src/lib.rs`.
- `/src/lib.rs` stays `/src/lib.rs`.
- Empty paths, `..`, `.`, empty segments, backslashes, and NUL bytes are
  rejected by `VirtualPath`.

Output JSON shape:

```json
{
  "status": "completed",
  "output": "updated src/lib.rs"
}
```

On recoverable patch failures, `ApplyPatchTool` should return a successful
`ToolOutput` with:

```json
{
  "status": "failed",
  "output": "file not found at path 'src/lib.rs'"
}
```

This matches the OpenAI guidance to return `failed` plus a clear output string
so a model can adjust future patches.

## Error Handling

`wyse-filesystem` should define patch errors separately from trait definitions.
Patch errors should preserve source chains for filesystem failures and avoid
host paths or file contents in display messages.

Expected failure categories:

- Invalid operation shape or missing diff.
- Invalid or escaping virtual path.
- File not found.
- File already exists for create.
- Context mismatch while applying an update diff.
- Filesystem read, write, or remove failure.

`wyse-tools` should reserve `ToolError` for malformed tool input or internal
tool execution failures. Patch conflicts and ordinary file failures should be
encoded in the tool result as `status: "failed"`.

## Testing

`wyse-filesystem` unit tests should cover:

- Creating a file from a V4A diff.
- Updating a file with context.
- Rejecting update diffs whose context does not match.
- Deleting files through the filesystem trait.
- Rejecting invalid virtual paths.
- Avoiding host paths and file contents in error text.

`wyse-tools` tests should cover:

- Registering and calling `ApplyPatchTool` through `BuiltinToolRegistry`.
- JSON input/output for completed create, update, and delete operations.
- JSON output for failed patch operations.
- Explicit construction with a filesystem dependency.

`crates/wyse-tools` should own:

- `docker-compose.test.yml`
- `Makefile`
- `tests/apply_patch_docker.rs`

The ignored docker integration test should use `LocalFilesystem` against a
crate-owned test sandbox and verify real create, update, delete, and rejection
behavior.

Local command:

```sh
make -C crates/wyse-tools test-integration
```

CI command:

```sh
cargo test -p wyse-tools --test apply_patch_docker -- --ignored --nocapture
```

## CI Integration

Update `.github/workflows/rust.yml` in the `integration` job to add
`wyse-tools` steps:

1. Start the `wyse-tools` test stack with
   `docker compose -f crates/wyse-tools/docker-compose.test.yml up -d`.
2. Run the ignored `wyse-tools` docker integration test.
3. Stop the stack with
   `docker compose -f crates/wyse-tools/docker-compose.test.yml down -v`.

The cleanup step must run with `if: always()` like the existing `wyse-infra`
cleanup step.

## Documentation Follow-Up

After implementation, archive the final crate-level design conventions in the
relevant crate `AGENTS.md` files before PR merge:

- `crates/wyse-filesystem/AGENTS.md`: document that apply-patch support is now
  a concrete shared filesystem capability and still must not expose host paths.
- `crates/wyse-tools/AGENTS.md`: document that filesystem-mutating builtin
  tools require explicit filesystem injection and are not auto-registered.

## Verification

Before claiming implementation complete, run:

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace --all-targets
make -C crates/wyse-tools test-integration
```
