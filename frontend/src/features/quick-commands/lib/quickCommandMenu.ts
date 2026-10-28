// 快速命令管理器上下文菜单的契约：目标 + 动作（组件与父级共用一份类型）。
import type { QuickCommand, QuickCommandGroupView } from "./quickCommands";

export type QuickCommandExportChannel = "clipboard" | "file";

export type QuickCommandsMenuTarget =
  | { kind: "group"; group: QuickCommandGroupView }
  | { kind: "command"; command: QuickCommand }
  | { kind: "overflow" };

export type QuickCommandsMenuAction =
  | { type: "new-command"; group?: QuickCommandGroupView }
  | { type: "new-group" }
  | { type: "rename-group"; group: QuickCommandGroupView }
  | { type: "export-group"; group: QuickCommandGroupView; channel: QuickCommandExportChannel }
  | { type: "delete-group"; group: QuickCommandGroupView }
  | { type: "run"; command: QuickCommand }
  | { type: "paste"; command: QuickCommand }
  | { type: "copy"; command: QuickCommand }
  | { type: "edit"; command: QuickCommand }
  | { type: "export-command"; command: QuickCommand; channel: QuickCommandExportChannel }
  | { type: "delete-command"; command: QuickCommand }
  | { type: "import" }
  | { type: "export-current-command"; channel: QuickCommandExportChannel }
  | { type: "export-current-group"; channel: QuickCommandExportChannel }
  | { type: "export-all"; channel: QuickCommandExportChannel };
