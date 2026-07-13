# stratum-tools AGENTS.md

## Scope

`stratum-tools` owns runtime tool traits, builtin tool wrappers, and tool registry behavior.

## Design Rules

- Tool names are provider-visible identities.
- Filesystem-mutating builtin tools require explicit filesystem injection.
- Do not auto-register filesystem-mutating tools in `BuiltinToolRegistry`.
- Filesystem-backed read-only builtin tools also require explicit filesystem injection.
- Keep filesystem-backed agent code workflow tools small and provider-visible by capability: `read_file_lines`, `list_dir`, `file_metadata`, and `search_text`.
- `search_text` is a controlled literal search over the injected virtual filesystem; do not expose shell or `rg` directly through this tool.
- Recoverable tool-domain failures should return structured tool output when the caller can act on them.
- Keep concrete builtin implementations separate from registry code.
- Do not add remote tool adapters, MCP adapters, shell tools, or approval flows until a concrete caller needs them.

## Tool Permissions

- Every registered tool declares `ToolKind` and `DangerLevel`.
- `Allow` authorizes every tool.
- `PartialAllow` authorizes only `Read + Low`; all other calls require approval.
- `RequireApproval` requires approval for every tool.
- Permission mode and registration metadata are immutable after sharing the registry.
- Keep provider-visible specs independent from runtime permission metadata.
