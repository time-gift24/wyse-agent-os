# wyse-config 约定

- 配置、agent template 和 resolved definition 都使用严格 schema；未知字段必须拒绝，不能静默忽略拼写错误。
- 默认模型和 template 选择的模型必须属于对应已配置 provider 的 `models` 列表；provider 缺失或模型未登记均为配置错误。
- `ResolvedAgentDefinition` 只保存运行所需的名称、模型、工具和 prompt，不得包含 API key、token 或其他 provider secret；凭据只保留在 provider 配置与构造边界。
