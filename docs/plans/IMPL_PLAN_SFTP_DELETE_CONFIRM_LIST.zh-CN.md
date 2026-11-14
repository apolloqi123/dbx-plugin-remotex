# SFTP 删除确认列表专项方案（2026-11-14）

> 现象：批量删除确认框只显示「已选择 N 项」，不显示具体要删哪些文件/文件夹；
> 用户无法在不可撤销操作前核对目标。单条删除也只显示文件名，没有类型提示。

## 目标

1. 批量删除确认框列出全部选中条目：类型图标 + 名称 + 「文件/文件夹」标签。
2. 单条删除确认框在名称旁显示同样的类型图标与标签。
3. 列表可滚动，长选择不把弹窗撑出屏幕。
4. 复用现有 i18n 类型键（`sftpAttrs.kind.*`），只新增一个列表标题键。

非目标：不改变删除逻辑、RPC、选择模型；不做虚拟滚动（删除确认是低频弹窗，
列表最大高度内滚动即可）。

## 实现

`frontend/src/app/components/WorkbenchSftpDialogs.vue`：

- `@lucide/vue` 增加 `File`、`Folder` 图标。
- 放宽 props：
  - `deleteTarget: { name: string; kind?: string } | undefined`
  - `selectedEntries: ReadonlyArray<{ name: string; uri?: string; kind?: string }>`
- 新增 `kindLabel(kind?: string): string`：映射到
  `sftpAttrs.kind.directory | symlink | other | file`。
- 单条删除：名称前加图标，名称后加类型标签。
- 批量删除：在「此操作无法撤销。」下方增加
  `sftpBatch.deleteListTitle` 标签 + `<ul>` 列表：
  - `v-for="entry in selectedEntries"`，key 用 `entry.uri || entry.name`；
  - 每行：图标 + 省略号名称（`title` 保留全名）+ 右对齐类型标签；
  - `max-height: min(38vh, 240px); overflow-y: auto;`，细边框、圆角、
    轻背景；滚动条使用浏览器默认（与现有弹窗一致）。
- 列表在删除进行中（`batchProgress` 存在）仍然显示，方便核对。

i18n（7 个 base locale 全加）：

- `sftpBatch.deleteListTitle`
  - en: `The following items will be deleted:`
  - zh-CN: `将删除以下项目：`
  - zh-TW: `將刪除以下項目：`
  - ja: `次の項目を削除します：`
  - es: `Se eliminarán los siguientes elementos:`
  - it: `Verranno eliminati i seguenti elementi:`
  - pt-BR: `Os seguintes itens serão excluídos:`

## 验收

```bash
pnpm --dir frontend typecheck
pnpm --dir frontend test
pnpm --dir frontend build
```

七语 key 对齐由 `workbench.spec.ts` 的既有断言覆盖；新增键必须出现在全部
7 个 locale 中，中文不得回退英文。
