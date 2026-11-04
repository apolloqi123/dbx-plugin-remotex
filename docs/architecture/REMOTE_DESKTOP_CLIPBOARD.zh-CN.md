# 远程桌面剪贴板共享（RDP / VNC）

本文描述远程桌面剪贴板的端到端设计、协议边界与安全约束。实现入口：

- 前端共享同步器：`frontend/src/features/remote-desktop/composables/useDesktopClipboard.ts`
- RDP 文件剪贴板前端：`useRdpFileClipboard.ts`、`lib/clipboardFiles.ts`、`RdpClipboardFilesOverlay.vue`
- RDP CLIPRDR 后端：`backend/src/sessions/rdp/clipboard.rs`、`backend/src/sessions/rdp/file_clipboard/`
- VNC 文本剪贴板：`backend/src/sessions/vnc/{worker,types,limits}.rs`

## 1. 文本剪贴板

### 1.1 共享同步器（RDP/VNC 同构）

`useDesktopClipboard` 同时服务 RDP 与 VNC，关键不变量：

1. **连接即共享**：会话首次成功读取到的宿主文本会推送到远端（mstsc 同语义），不再作为静默基线丢弃；空文本不推送。连接前复制的敏感内容会随共享开关进入本次会话，关闭开关即停止。
2. **可见即可轮询**：仅在会话 running、文档可见、同步开关开启且远端通道可用时轮询（1.5s），不再要求 iframe 或画布持有焦点——用户在其它 macOS 应用中复制的文本也能同步；DBX 窗口仍在前台时宿主读取只受 1 次/秒限流约束。
3. **宿主桥优先**：存在 `window.dbxPlugin.clipboard` 时绝不回退 Web Clipboard，避免绕过宿主的授权/限流/审计；桥缺失时才走 `navigator.clipboard`。
4. **成功后记账**：宿主→远端写入 RPC、远端→宿主写入均在成功后更新基线；失败保留旧基线，下一轮重试。
5. **回声抑制**：远端回写宿主后同时更新两个基线，轮询不会把刚写入的文本再推回去。
6. **失败分级退避**：连续 3 次普通读取失败暂停 15s 后重试；限流错误不计入失败；宿主明确拒绝授权（首次授权的系统弹窗被拒/权限缺失）时暂停 60s 并给出一次可操作提示（重新打开该连接标签页会重建宿主桥，可再次弹出授权）。显式 Ctrl/Cmd+V 始终可重试。
7. **一次粘贴、绝不吞键**：Ctrl/Cmd+V 读取宿主文本（最近 900ms 内已读则直接复用缓存，避免争用 1 次/秒配额）→ `rdp/set-clipboard` / `vnc/set-clipboard` → 等待 80ms 让 sidecar 记账 → 由画布暴露的 `sendPasteShortcut()` 向远端补发一次 Ctrl+V（macOS 同样映射为 Control）。宿主读取失败或远端写入失败时**仍然补发 Ctrl+V**，远端可粘贴上一次已暂存的文本；原生 paste 事件携带的文本走同一路径，不重复读桥。宿主桥限流时显式粘贴等待 1.1s 重试一次；显式粘贴期间暂停后台轮询。连接表单关闭剪贴板共享时画布不拦截 Ctrl/Cmd+V，按键按普通远端输入转发。
8. **macOS 复制映射**：远程画布内 Cmd+C 被翻译为远端 Ctrl+C（Cmd 按下/释放对应 Control 按下/释放），因此 Windows 远端执行复制后经 CLIPRDR 回写宿主；在宿主 macOS 应用中 Cmd+C 后切到远端按 Ctrl+V，会走第 7 条一次性同步+粘贴。VNC 的 `sendPasteShortcut` 在物理 Control 已按住时复用该 Control，不会提前释放。
8. **远端→宿主**：RDP `rdp/clipboard`（分片）与 VNC `vnc/clipboard` 统一交给同步器；写入路径为宿主桥 → `navigator.clipboard`；存在宿主桥时事件驱动写入不使用 execCommand 兜底（隐藏 textarea 会抢焦点，画布 blur 后释放按键），无桥时仍保留该兜底。全部失败时提示用户并在下一轮轮询重试，不静默丢弃。
9. **失败不记账**：宿主→远端 RPC 或远端→宿主写入失败时基线保持原值，下一轮轮询/下一次显式粘贴会重试；显式粘贴使用“最近一次实际读到的宿主文本”缓存，即使它的推送失败也不会退回更旧的文本。
10. **手动写入远端**：工具栏“粘贴到剪贴板/输入文本”直接写远端时不改动宿主基线，下一次轮询不会用旧宿主文本覆盖手动内容；直到宿主剪贴板真的变化才恢复自动同步。

### 1.2 RDP CLIPRDR 初始化

根因：`on_request_format_list` 只在“已经暂存文本”时才发送 FormatList。IronRDP 客户端在 Monitor Ready 时依赖该回调完成 `Capabilities + TemporaryDirectory + FormatList` 初始化；没有文本时通道停留在 `Initialization`，远端复制文本会触发 `require_ready` 失败，vendored 客户端会把该错误向上传播并终止会话。

修复：`on_request_format_list` 无条件广告 `CF_UNICODETEXT`；没有暂存文本时格式数据请求返回合法空字符串（不是错误）。文本回调只依赖本通道自己的 `channel_ready`（`on_ready` 置位），不再借用文件剪贴板阶段的 Ready 标记；若服务器迟迟不返回 `FormatListResponse`/返回 Fail，通道停留 `Initialization` 时会在远端复制或格式数据请求到来时做最多 3 次、间隔 2s 的重新初始化广告（`initiate_copy` 在 Initialization 会重发 Capabilities + TemporaryDirectory + FormatList）。`on_ready` 只在“已有暂存文本且没有待发送文件”时补一次文本广告，覆盖拨号期间 `rdp/set-clipboard` 早于通道加入的情况；不会清空已接受的文件夹广告。`FormatList` 被拒且通道尚未 Ready 时同样走文本重建，不再向引擎发送会因 `require_ready` 失败而终止会话的文件广告。远端同时提供文本与文件时，先请求极小的 `FileGroupDescriptorW` 元数据，再串行请求文本（Cliprdr 只有一个 pending FormatDataRequest 槽位）；每收到新的远端 FormatList 都先清空链式文本标记，避免上一次的链式请求误配到新的剪贴板代。

通道状态：`on_ready` 发 `rdp/clipboard-status {state:"ready"}`；首帧后 8s 仍未 Ready（通常是服务器策略禁用了剪贴板重定向，或虚拟通道未被接受）发 `state:"unavailable"`，前端停止无意义的后台读取并提示一次，用户可改用“输入文本”工具栏动作。诊断（阶段、格式数量、文本长度，绝不含正文）写入插件数据目录的 `rdp-debug.log`。

### 1.3 VNC 文本

- 入站：`ServerCutText` 由 vnc-rs 转为字符串；现代服务端（TigerVNC/KasmVNC 等）使用 UTF-8。
- 出站：`ClientCutText` 以 UTF-8 字节写出，上限 1 MiB。
- 已知边界：经典 RFB 规范将 cut text 定义为 Latin-1；坚持 Latin-1 的服务端在中文等非 ASCII 文本上可能出现替换字符。vnc-rs 0.6 在解码阶段已做 `from_utf8_lossy`，插件层无法无损恢复原始字节；若要覆盖这类服务端，需要 fork/上游修改 vnc-rs 以保留原始字节并做编码探测。

## 2. 文件/文件夹传输

标准 RFB 没有文件剪贴板，CLIPRDR 只能承载远端用户主动复制的文件，因此两条路径并存：

- **RDP 原生文件剪贴板**（2.1–2.3）：在 Windows 资源管理器与 Finder/Explorer 之间直接复制粘贴，由 CLIPRDR 文件流承载。
- **RDP/VNC 文件传输浮窗**（2.5）：两种远程桌面工具栏都提供「文件传输」按钮，打开宿主机 ↔ 远端双栏，通过独立 SSH/SFTP 会话双向复制。VNC 的文件操作只走这条路径。

### 2.1 宿主 → 远端

1. 用户点击 RDP 桌面右上角“发送文件…”或把文件/文件夹拖到桌面。
2. 前端通过宿主 `fileTransfer.pick` / `onDrop` 取得文件句柄（文件夹由宿主展开为带 `relativePath` 的文件列表）。
3. `rdp/clipboard-files/begin` 创建上传（一次一个，临时目录带 session+UUID）；按 1 MiB 分块 `rdp/clipboard-files/chunk`，最后 `commit`。
4. 后端逐段校验路径（穿越、绝对路径、保留 Windows 设备名、控制字符、259 字符线制名上限）、声明大小与总量；写入临时文件后构建 `FileDescriptor` 列表。
5. worker 发送 `ClipboardMessage::SendInitiateFileCopy`（要求远端已协商 `STREAM_FILECLIP_ENABLED` 且通道 Ready）。远端用户在资源管理器中 Ctrl+V 时，服务端按需发 `FileContentsRequest`；后端从临时文件回答 SIZE / 1 MiB RANGE。
6. 文本与文件互斥：写入文本会清除待发送文件；提交文件会清除暂存文本（最后一次复制获胜）。

### 2.2 远端 → 宿主

1. 远端复制文件后，FormatList 中出现 `FileGroupDescriptorW`；只有通道 Ready 且服务端协商了 `STREAM_FILECLIP_ENABLED` 时，后端才请求元数据（否则只走文本，绝不触发会终止会话的文件请求）。`on_remote_file_list` 校验数量/大小后只发出 `rdp/clipboard-files/available`，**不读取文件正文、不写临时文件**。
2. 用户点击“下载文件”后才调用 `rdp/clipboard-files/download`，后端在 2000 项 / 512 MiB 上限内逐文件 SIZE → 1 MiB RANGE，写入私密临时目录；目录按相对路径创建。进度经 `rdp/clipboard-files/progress` 发送（每完成一个文件或约 4 MiB）。超过上限时只发错误事件，不写盘。
3. 全部完成后打包为 `remote-clipboard-<timestamp>.zip`，发出 `rdp/clipboard-files/ready`。zip 使用快速压缩级别（level 1），避免为临时容器消耗大量 CPU；打包仍是同步磁盘操作（极端 512 MiB 会短暂占用引擎线程），当前接受此延迟，后续可移到独立任务。
4. 若远端同时提供文本与文件，后端先取极小的文件元数据再串行拉文本；文件元数据请求失败（错误或解析失败）时仍会补拉文本，不会因文件通道问题丢失文本。远端复制的文件在用户点击下载前只占元数据内存，不产生磁盘/网络放大。
5. 前端浮层显示“远端剪贴板有 N 个文件”，用户点击“保存 zip”后通过宿主 `beginSave` 流式写入（`rdp/clipboard-files/read` 每次 ≤1 MiB，响应带 `archiveName`；保存期间归档被新的远端复制替换会中止并提示，避免拼接两个 zip）。失败会取消句柄，不留下半成品。
6. 任何错误只发 `rdp/clipboard-files/error` 并清理临时目录，绝不让异常穿回引擎泵；生成结束时也会中止未完成的入站下载，防止进度条冻结。

### 2.3 原生宿主文件剪贴板（宿主文件管理器 ↔ 远端）

宿主 0.6.62 的剪贴板桥只有文本/图片 API（已核对宿主源码），无法读写 Finder /
Explorer 的文件列表。插件 sidecar 因此直接访问宿主系统文件剪贴板
（`backend/src/platform/pasteboard/`，按平台实现）：

- **macOS**：`NSPasteboard`（`public.file-url`；写入用 `osascript` JXA + `NSURL`）。
- **Windows**：Win32 `CF_HDROP` + `Preferred DropEffect=copy`；
  `GetClipboardSequenceNumber` 作为变化计数。
- **Linux**：`xclip`（X11）或 `wl-clipboard`（Wayland）读写 `text/uri-list` /
  `x-special/gnome-copied-files`；探测放在独立线程并缓存结果，RDP worker 的
  `change_count()` 不阻塞引擎。缺少这两个工具时能力关闭，浮层按钮路径照常可用。

1. **宿主 → 远端**：worker 每 700ms 只读变化计数（Linux 由后台线程探测）；变化后
   读取文件列表（从不读文件内容）。文件/文件夹展开为 `FileGroupDescriptorW`
   （目录带 `DIRECTORY`，子项带相对路径）并作为本地文件剪贴板广告；只有远端用户
   真的 Ctrl+V、服务端发 `FileContentsRequest` 时才按需打开原文件并按 SIZE / 1 MiB
   RANGE 应答。复制时名称按 Windows 合法字符清洗、保留设备名加前缀、跳过符号
   链接/特殊文件，沿用 2000 项 / 512 MiB / 单文件 512 MiB 上限。文件管理器复制
   同时产生的文件名字符串文本会被后端忽略（`has_file_urls`），不会用文本广告
   替换文件列表。
2. **远端 → 宿主**：远端复制文件后，小集合（≤8 个且 ≤32 MiB）在宿主原生文件
   剪贴板可用时自动下载；更大的集合保持“仅元数据”，由浮层“下载文件”显式触发。
   下载完成后把临时目录的顶层文件/文件夹写入宿主文件剪贴板（macOS NSPasteboard、
   Windows CF_HDROP、Linux xclip/wl-copy），文件管理器可直接 Ctrl/Cmd+V；同时
   保留原始文件供按需“保存 zip”。写剪贴板失败或平台能力不可用时仍可保存 zip，
   并给出可见提示。插件自己写入的下载目录路径（temp 根下）不会被再次广告回远端。
3. **能力上报**：`rdp/clipboard-files/status` 返回 `nativeFiles` / `serverFiles` / `fileClipboardReady`，`rdp/clipboard-status` 的 `ready` 事件带 `fileCopy`。工具栏“文件传输”提示据此在三种出口间切换：宿主原生 Ctrl+C/V（macOS/Windows/Linux helper）、桌面浮层的“下载文件 / 发送文件…”（宿主没有原生剪贴板）、以及 **SSH/SFTP 双栏窗口**（远端服务器不支持 CLIPRDR 文件流，典型是 Linux 的 GNOME Remote Desktop——它只协商文本剪贴板）。Linux Wayland 宿主的探测优先 `xclip`（经 XWayland 剪贴板桥；GNOME 不提供 wlr-data-control），其他 Wayland 桌面再用 `wl-clipboard`。
4. **Linux/GNOME 路径差异**：GRD 的服务端→客户端文件列表由 WinPR 生成，嵌套条目把相对路径放在
   `cFileName` 里并使用 `/` 分隔（`sub/deep/c.txt`），而不是 MS-RDPECLIP 的 `\`；sidecar 下载前会把
   两种分隔符都拆成组件，文件夹树才能落盘。反方向（宿主 → GRD）受 GRD 51 实现限制：它把客户端
   文件列表转成 URI 列表时会跳过任何包含 `/` 或 `\` 的名字，因此**从宿主粘贴文件夹到 GNOME 远端
   只能得到顶层条目，子项会被丢弃**；文件本身可以正常粘贴。需要完整文件夹时请用 SSH/SFTP。
5. **资源与隐私**：轮询只读变化计数；URL 只在变化时读取一次；文件内容只在远端
   粘贴（或用户显式下载）时读取。不记录路径/内容。下载完成后的剪贴板写入只包含
   插件自己的临时文件；会话关闭/新的远端复制会清理临时目录（若剪贴板仍引用已清理
   文件，文件管理器粘贴会失败，属于资源回收的已知边界）。Linux 的 xclip/wl-copy
   会驻留为选区所有者直到被下一次复制替换，这是这些工具的固有行为。

### 2.4 文件传输浮窗（RDP/VNC 通用）

VNC（RFB）没有文件剪贴板，RDP 的 CLIPRDR 也只能承载远端用户主动复制的文件。为让两种远程桌面都能浏览并传输文件，RDP/VNC 工具栏新增「文件传输」按钮，打开一个固定居中的浮窗：

1. **传输通道**：浮窗首次使用时要求填写远端 SSH 主机/端口/用户名/认证方式（密码、私钥路径、私钥+口令、SSH Agent、None）。后端 `filetransfer/connect` 用这些信息建立一条**免 PTY 的 SSH/SFTP 会话**：复用既有拨号/认证与 host-key 弹窗，只注册 transport lease 与 SFTP 缓存，不开 shell、不回放、不进入 `ssh/sessions/list`；关闭浮窗或工作台时调用 `filetransfer/disconnect` 回收。同一工作台重试连接会先关闭旧会话；面板每 30s 心跳（`filetransfer/ping`），后端回收 3 分钟无心跳或 transport 已关闭的会话。VNC/RDP 本身保持连接不变。**自动连接**：VNC/RDP 进入 running 或再次打开浮窗时，若已保存私钥/Agent/None 参数，或本次 webview 内成功登录过密码认证，则直接建立伴随会话并显示两栏，不再要求重复填写；用户从工具栏打开浮窗且尚无 SSH 密码时，会先用当前 RDP/VNC 连接已暂存的同账号密码尝试一次（Windows/Linux 需已启用 SSH/SFTP，否则回落到表单）；之前成功连接过的 SSH 密码用插件 vault（AES-256-GCM，0600 keyfile）按 host/port/username 加密保存，插件重启或新标签页打开浮窗也会自动读回，表单提供“清除已保存的 SSH 密码”；预填只补空字段，不会用 VNC/RDP 主机覆盖已保存的自定义 SSH 主机/用户名；密码缓存只存在于当前 workbench 内存（作用域销毁即丢弃，不落盘），自动重连失败会清除缓存密码并回落表单。
2. **双栏**：左栏宿主机文件系统（`filetransfer/local/list`，只列元数据，目录优先、隐藏项淡显），右栏远端（`sftp/list`）。单击选择、双击进入目录；路径栏可直接输入绝对路径，`↑`/主目录/刷新按钮与两栏独立选择。中间 `↑上传` / `↓下载` 把选中项复制到对侧当前目录；顶部显示待上传的剪贴板文件（来自 VNC 的文件粘贴）。
3. **传输**：后端按任务执行（`filetransfer/upload/start` / `filetransfer/download/start`），复用既有 SFTP 1 MiB 分块、进度事件与原子提交。文件夹递归展开、保留结构；符号链接/特殊文件跳过并计数；同名文件默认追加 ` (n)`（不覆盖），只有显式 `overwrite` 才替换同名文件；单文件 16 GiB、单次 200 个顶层源、树内 20000 项/深度 64 的上限。上传会等待每个文件的远端推送真正完成后再继续下一个（避免提前报成功/多个推送占满并发额度）；压缩下载会先等待准备完成或回退普通管线。进度统一为 `filetransfer/progress`（含 `sessionId`、方向、当前文件、文件数、字节数、百分比），完成发 `filetransfer/done`（含失败文件数）；任务按会话隔离，可随时取消。
4. **剪贴板衔接**：VNC 画布内 Ctrl/Cmd+V 若检测到 Finder 文件剪贴板（`filetransfer/clipboard/files`），会打开浮窗并把这些路径列为待上传项，而不是发送空文本；拖入文件时提示使用该按钮。下载完成后浮窗提供「复制到 Mac 剪贴板」（`filetransfer/clipboard/write`，仅 macOS），Finder 可直接 ⌘V；另有「在 Finder 中显示」。
5. **凭据与隐私**：密码/口令只在本次连接 RPC 内使用，不落盘、不写日志；浮窗只记住 host/port/username/认证方式/私钥路径（pluginStore），不记住密码。本地列表只返回名称/大小/修改时间等元数据，上传只读取用户显式选择的源；远端目录必须已存在，路径逐段校验，不跟随符号链接。
6. **VNC 桌面复制文件 → 宿主粘贴**：RFB 本身传不了文件，但远端 Thunar/Nautilus 复制文件时会设置 X11 的 URI-list。插件有两条互补路径：
   - 若 VNC 服务端把 URI-list 当文本转发，worker 解析 `file://` 行后发 `vnc/clipboard-files`，前端直接调用 `filetransfer/clipboard/fetch`；
   - 否则经伴随 SSH 会话执行固定只读探针 `filetransfer/clipboard/remote`（`xclip`/`xsel`，逐显示尝试、`timeout` 限时），读取同一个 URI-list。
   两条路径都只接受绝对 `file://` 路径（纯文本路径不会误触发），把文件/文件夹经 SFTP 拉到 `~/Downloads/DBX Remote Clipboard`，完成后写入 macOS 文件剪贴板，Finder 里 ⌘V 即得到真实文件。VNC 画布 Ctrl/Cmd+C 会立即探测并记录文件列表，但**不会立刻下载**：只有用户在复制后 15 秒内离开 VNC 窗口（切到 Finder/其他 Mac 应用）才拉取并写入文件剪贴板；留在 VNC 内继续粘贴不会改动 Mac 剪贴板。窗口失焦时补探测一次，另有 3s 轮询兜底右键菜单复制；远端剪贴板变成文本时丢弃待处理的文件列表。全部下载任务的进度带 `background:true`，不占用浮窗汇总条。远端无 `xclip`/`xsel` 且服务端也不转发 URI-list 时，会提示改用浮窗。
7. **前提**：远端主机需要 SSH/SFTP 服务；若还需从 VNC 桌面直接复制文件（而非用浮窗），远端通常还需要 `xclip` 或 `xsel`（X11）。纯 VNC 且没有 SSH 的服务器无法使用上述伴随通道，浮窗会明确提示连接失败；这不会影响 VNC 桌面本身。

### 2.5 限制

| 项 | 上限 |
| --- | --- |
| 文件数 | 2000 |
| 单文件 | 512 MiB |
| 单次总大小 | 512 MiB |
| 单块读写 | 1 MiB |
| 文本 | 16 MiB（RDP）/ 1 MiB（VNC） |

临时目录：出站/入站均为 `temp_dir()/dbx-sshx-rdp-clipboard/<session>-<uuid>`；Unix 上目录以 0700、文件以 0600 创建（避免同机其他用户读取剪贴板文件）。会话关闭、替换上传、clear、生成结束或对象 Drop 时删除。路径写入前拒绝符号链接；上传分块必须严格顺序写入，提交时再核对声明大小。

## 3. DBX 宿主边界

当前宿主剪贴板桥（`host.clipboard:read` + 无门槛的 `host.copy`）只支持文本与 PNG 图片，不支持 Finder/Explorer 文件列表（NSPasteboard `public.file-url` / Windows `CF_HDROP`），也没有写入文件 URL 的 API。插件通过 sidecar 直接访问 macOS 剪贴板补齐了 macOS 侧的原生文件复制粘贴（见 2.3）；Windows/Linux 宿主上的原生文件剪贴板仍需宿主新增能力（或对应平台的 sidecar 实现），见 `HOST_CLIPBOARD_FILES_PROPOSAL.zh-CN.md`。2.4 的双栏文件传输不依赖宿主新增 API，但要求远端主机提供 SSH/SFTP；它不能替代原生“在远端资源管理器按 Ctrl+V”的体验，只提供显式的双栏传输。

## 4. 安全与隐私

- 文本读取受宿主 `host.clipboard:read` 权限、首次会话授权、1 次/秒限流与审计约束；桥存在时不回退 Web API。
- 基线先行避免连接前敏感文本外泄；窗口失焦/文档隐藏时停止自动读取。
- 文件路径逐段校验，限制数量/大小，拒绝符号链接与 Windows 保留设备名；临时目录/文件在 Unix 上使用 0700/0600；文件内容不写日志。
- 远端复制文件默认只取元数据；≤8 个且 ≤32 MiB 的小集合自动下载并写入 macOS 剪贴板（用户已选择该策略），更大集合必须点击“下载文件”，避免无意图的大流量/磁盘占用。超出 2000 项 / 512 MiB 时只提示错误。
- 原生 macOS 文件剪贴板只读取 `public.file-url` 与文件元数据；正文只在远端粘贴或用户显式下载时读取；写剪贴板只使用插件临时目录中的下载结果；不记录路径与内容。
- 不新增驱动器重定向；文件只通过 CLIPRDR 文件流传输。

## 5. 验证

- 前端：共享同步器 23 项、VNC 会话 27 项、RDP 文件数据层 21 项、文件传输浮窗 6 项（连接、双栏、上传/下载事件、剪贴板、断连）、VNC 远端文件拉取 5 项、画布快捷键/`sendPasteShortcut` 13 项、事件路由 9 项；全量 2814 项。
- 后端：CLIPRDR 初始化/空文本/远端复制/能力门控/错误回退/未 Ready 重建 19 项；文件剪贴板路径、上传、SIZE/RANGE、入站下载与 zip、上限与错误 等 10+ 项；文件传输本地列表/路径校验/源树规划/冲突命名/剪贴板路径校验/连接参数 15 项；URI-list 解析与远端剪贴板探针 4 项；全量 1453 项（1 ignored）。
- 真机待验证：Windows 10/11 RDP 文件双向、VNC 服务端（TigerVNC/KasmVNC）文本剪贴板、严格 Latin-1 VNC 服务端的非 ASCII 行为；宿主首次授权弹窗被拒后重新打开标签页的恢复路径；以及 RDP/VNC 文件传输浮窗对真实 SSH/SFTP 服务端的连接、双栏上传/下载、压缩下载准备、Finder 文件粘贴与下载后写回 macOS 剪贴板；VNC 桌面复制文件 → Mac ⌘V 需要真机验证服务端 URI-list 转发与远端 xclip/xsel 两种情况。