// 工具栏统一录制入口的动作契约（feature 内声明，app 侧类型组合复用）。
export interface RecordingMenuActions {
  toggleRecording(): void;
  toggleRecordings(): void;
  cancelRecordCountdown(): void;
}
