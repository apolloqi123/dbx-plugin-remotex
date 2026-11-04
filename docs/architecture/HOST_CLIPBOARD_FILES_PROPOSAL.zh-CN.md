# DBX 宿主文件剪贴板能力提案（PR-ready）

状态（2026-11-01 更新）：macOS 侧已由插件 sidecar 直接读写 `NSPasteboard` 实现原生 Finder ↔ 远端文件复制粘贴（见 `REMOTE_DESKTOP_CLIPBOARD.zh-CN.md` §2.3），不再依赖宿主新增接口。以下提案仍适用于：Windows/Linux 宿主上的原生文件剪贴板、沙箱 iframe 读文件 URL、以及把读取纳入宿主的授权/审计/限流。遵循宿主边界：本仓库不修改 DBX。

## 1. 现状与缺口

宿主 `PluginHostBridge` 的剪贴板接口只有：

- `copyText(pluginId, text)` / `host.copy`
- `clipboardRead(pluginId)`（`host.clipboard:read`，首次授权 + 1 次/秒 + 审计）
- `clipboardReadImage(pluginId)`

原生实现基于 `tauri-plugin-clipboard-manager`，只暴露 text/image 的 allowlist；没有读取 `NSPasteboardTypeFileURL`（macOS）或 `CF_HDROP`（Windows），也没有写入文件 URL 的接口。因此插件无法：

1. 读取用户在 Finder/Explorer 中复制的文件/文件夹列表；
2. 把远端复制出的文件写成系统剪贴板中的文件项，供 Finder/Explorer 直接粘贴；
3. 让插件 iframe 获得拖拽源文件的真实路径（当前只转发不透明句柄，且 HTML5 drop 不到 web 内容）。

## 2. 建议新增能力

### 2.1 能力标志

`capabilities` 增加：

```ts
clipboardFileRead: boolean;
clipboardFileWrite: boolean;
```

缺省省略，旧宿主保持“不支持”语义；插件必须按能力降级。

### 2.2 读取系统文件剪贴板

```ts
clipboardReadFiles?(pluginId: string): Promise<PluginClipboardFile[]>;

interface PluginClipboardFile {
  handleId: string;       // 复用现有 open-file handle 读取字节
  name: string;
  relativePath?: string;  // 文件夹内相对路径，'/' 分隔
  size: number;
  contentType?: string;
  isDirectory?: boolean;
}
```

原生侧：

- macOS：读取 `NSPasteboard.general` 的 `public.file-url` / `NSFilenamesPboardType`，为每个 URL 打开只读句柄并收集元数据；目录可递归展开（沿用现有 drop 的深度/数量上限）。
- Windows：读取 `CF_HDROP`，同样转换为只读句柄与元数据。
- 与文本读取共用同一套授权/限流/审计；新增独立权限 `host.clipboard:files` 或复用 `host.clipboard:read`（建议独立，便于用户区分“文本”与“文件”）。
- 上限建议：2000 项、总计 512 MiB、目录深度 8；超限返回受控错误而不是截断。

### 2.3 写入系统文件剪贴板

```ts
copyFiles?(pluginId: string, files: PluginClipboardFileWrite[]): Promise<void>;

interface PluginClipboardFileWrite {
  name: string;
  relativePath?: string;
  // 二选一：已有宿主路径（插件数据目录/临时目录），或调用方提供的字节流句柄
  path?: string;          // 仅限宿主允许的目录
  handleId?: string;      // 插件已通过 beginFileSave/临时区写入的句柄
  size: number;
}
```

原生侧：

- macOS：写出 `public.file-url`（必要时应使用 `NSPasteboardItem` + 文件承诺，或先把字节落到宿主临时目录再写 URL）；设置 `NSPasteboard.general.clearContents()` 后写入。
- Windows：构造 `DROPFILES` + `CF_HDROP`。
- 写入前校验路径属于插件临时区/用户明确选择的目录，防止插件把任意系统路径塞入用户剪贴板。
- 与文本写入一致，无需每次授权，但应记录审计（不含文件内容）。

### 2.4 可选的拖出（drag-out）

若希望从远端文件列表直接拖到 Finder/Explorer，需要宿主暴露原生拖拽源（`NSFilePromiseProvider` / `DoDragDrop`）。这比剪贴板写入复杂，可作为后续增强，不阻塞“复制/粘贴文件”主流程。

## 3. 插件侧对接方式（已就绪）

`useRdpFileClipboard` 已把文件传输抽象为两条路径：

- 宿主→远端：`fileTransfer.pick` / `onDrop` → `rdp/clipboard-files/begin|chunk|commit`；
- 远端→宿主：`rdp/clipboard-files/ready` → `beginSave` 流式保存 zip。

宿主新增 `clipboardReadFiles` 后，可在同一 composable 中优先读取文件剪贴板并直接 `sendFiles`；新增 `copyFiles` 后，可把远端下载的临时文件直接写入系统剪贴板，替换“保存 zip”提示。这样不会改变 CLIPRDR 后端协议。

## 4. 验证清单（宿主侧）

- macOS：Finder 复制单文件/多文件/文件夹 → 插件读取句柄、名称、相对路径、大小；插件写入后 Finder 可直接粘贴；重名/超大/空目录行为。
- Windows：Explorer 复制文件/文件夹 → 插件读取 `CF_HDROP`；插件写入后 Explorer 可直接粘贴。
- 权限：未声明权限时拒绝；首次授权弹窗；拒绝后不重复打扰；限流与审计条目。
- 安全：路径穿越/符号链接/保留设备名/超大文件/超多文件均受控失败；插件不能借此读取任意系统路径。