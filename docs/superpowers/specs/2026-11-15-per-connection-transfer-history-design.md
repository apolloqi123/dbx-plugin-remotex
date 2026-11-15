# Per-Connection Transfer History Design

## Intent

SFTP 传输历史当前跨所有 SSH 连接共享，用户希望每个 SSH 连接独享历史。稳定归属键必须是保存的连接 id（`connectionId`），不是每次打开工作台都会变化的 `sessionId`。这个设计只改查询/清空范围与历史行的归属信息，不引入第二套存储。

## Approaches

1. **按 `sessionId` 分片**：改动最小，但重连、复制页签、宿主重建工作台都会换 id，历史会消失；否决。
2. **按 `connectionId` 分文件**：隔离最彻底，但要迁移现有 `transfer-history.json`、处理跨进程 last-writer-wins、改文件权限模型；超出需求，否决。
3. **全局环形账本 + 按 `connectionId` 精确过滤（选用）**：沿用 200 条上限和原子写，开始/终态行都带归属；查询与清空带 scope。改动局部，旧行不会串到别的连接，本机保存行显式标为 `local`。

## Decisions

- `sftp/transfer/history` 新增可选 `connectionId?`；与现有 `sessionId?` 可同时生效（AND），`limit?` 保持 1..=200。
- `sftp/transfer/history/clear` 新增可选 `connectionId?`；只删除该 scope 的持久化行和当前进程内历史，不删除其他连接或 `local` 行。无 scope 兼容旧调用者，仍全清。
- 精确字符串匹配；无 `connectionId` 的旧行对任何 scoped 查询都不可见，避免泄露。无 scope 查询仍能看到旧行。
- 所有终态 `record_transfer` 行先尝试从调用负载、活动会话、同 `taskId` 的起始行继承 `connectionId`。解析不到时保持空串，scoped 视图不展示。
- 活动 live 行仍按会话映射解析 `connectionId`；会话已注销时为空串，不归入当前连接。
- `local/saveFile` 与容器导出写入 `connectionId: "local"`，不属于任何 SSH 连接。
- 本地终端/RDP/VNC 等无 SSH context 的工作台发送 `connectionId: "local"`；不发送空参数。

## Non-goals

- 不按连接拆分文件、不提高/降低 200 条全局上限、不改 `STORAGE_VERSION`、不改 JSON-RPC 响应结构。
- 不删除旧行；升级后旧行只在 legacy/unscoped 视图可见。
- 不改活跃传输列表（`sftp/transfer/list`）的 `sessionId` 语义。

## Failure modes

- 两个连接同时传输：终态继承不得把 A 的行归到 B；`sessionId → connectionId` 解析与起始行继承都要有单测。
- 清空一个连接时另一个连接正在跑：只清理目标 scope 的持久化/内存行；live registry 本身不销毁。
- 会话关闭后收尾：`cleanup_session_transfers` 必须用关闭前捕获的归属，不能因为会话已从 map 移除而留下无归属失败行。
- 老版本前端不传 scope：后端继续返回合并后的全局历史，保持兼容。
- mock / visual fixture：必须模拟两个 scope，否则「清空当前连接」看起来仍像全清。
