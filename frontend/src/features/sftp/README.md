# features/sftp — 子域边界

SFTP 域按“数据入口 → 树/传输 → 编辑器/动作”分层。跨子域只能引用 `lib` 的具名导出，
不得直接引用其他子域的内部实现；`composables` 只做 Vue 状态与生命周期编排。

## lib

| 子域 | 职责 |
| --- | --- |
| `entries/` | 条目清洗、筛选、名称编码、错误文案（`sftpEntries` / `sftpFileFilters` / `sftpName` / `sftpErrors`） |
| `paths/` | 远端路径输入、书签、路径拼接（`remotePathInput` / `sftpBookmarks` / `sftpRemotePaths`） |
| `tree/` | 树模型、虚拟窗口、拖放条目、行键盘语义（`sftpFileTree` / `virtualWindow` / `dropEntries` / `fileRowKeydown`） |
| `transfer/` | 文件夹上传/压缩/覆盖策略、下载清洗、冲突偏好、宿主拖放映射（`folderUpload` / `folderArchive` / `folderOverwrite` / `sanitizedDownload` / `sftpFolderDownload` / `downloadPrefs` / `hostFileDrop` / `hostDragMapping`） |
| `editor/` | 编辑器规则、主题、文本嗅探、JSON 预览、外部改动监听（`editorRules` / `editorTheme` / `textSniff` / `jsonPreview` / `watchEdits`） |
| `actions/` | 右键菜单、行详情、重命名的纯逻辑（`sftpContextMenu` / `sftpDetails` / `sftpRename`） |

## composables

| 子域 | 职责 |
| --- | --- |
| `listing/` | 目录导航、列表布局、单栏树工作区；**唯一允许发起 `sftp/list` 的位置** |
| `tree/` | 树虚拟滚动、详情卡、宿主文件拖放目标 |
| `transfer/` | 上传链、重复策略、文件夹上传、宿主拖放、下载、归档 |
| `editor/` | 编辑页签、打开方式、分屏、预览、外部改动 |
| `actions/` | 选择/剪贴板、删除、重命名、新建、属性、书签、chmod、symlink、目录动作 |

## 规则

1. `composables/listing` 是列表数据的唯一出口；其他子域通过它的返回 API 读取，不自行 invoke `sftp/list`。
2. `components/` → `composables/` → `lib/` → `shared/` 单向依赖；`lib` 不得引用 `components/` 或 `composables/`。
3. 跨子域只经 `lib` 具名导出；不要深入另一个子域的文件路径。
4. spec 与被测模块同目录；纯函数测试优先放在 `lib/<subdomain>/`。
