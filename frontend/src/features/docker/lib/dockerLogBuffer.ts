// 容器日志环形缓冲与跟随判定（纯逻辑，无 Vue 依赖，便于单测）。
//
// 后端 `docker/logs/chunk` 的 UTF-8 有损解码可能切在半行上，所以缓冲按 `\n`
// 拆分、末段留作 partial，下一次 append 先拼接。缓冲同时受行数与字符数两个
// 上限约束，超限一律从头部裁剪并记住“已截断”供 UI 提示；`clear` 只清视图
// （流不停止）。

/** 最多保留的行数（含未完成的 partial 行）。 */
export const MAX_LOG_LINES = 10_000;
/** 最多保留的字符数（含换行与 partial 行）。 */
export const MAX_LOG_CHARS = 2_000_000;

/** 距底部 ≤4px 视为贴底：自动恢复跟随。 */
export const FOLLOW_RESUME_PX = 4;
/** 向上滚动超过 24px 视为用户已离开底部：暂停自动跟随。 */
export const FOLLOW_PAUSE_PX = 24;

/**
 * 是否仍贴底（可以跟随）。滚动容器的 three-value 关系：
 * `scrollHeight - scrollTop - clientHeight` 即距底距离。
 */
export function shouldFollow(scrollTop: number, clientHeight: number, scrollHeight: number): boolean {
  return scrollHeight - scrollTop - clientHeight <= FOLLOW_RESUME_PX;
}

export class LogBuffer {
  private lines: string[] = [];
  private partial = "";
  /** 已完成的各行的字符数之和（不含换行）。 */
  private lineChars = 0;
  /** 当前整段文本的字符数（含换行与 partial）。 */
  private chars = 0;
  private wasTruncated = false;

  /** 追加一个分片：可能跨行、可能以半行结尾。 */
  append(text: string): void {
    if (!text) return;
    const parts = (this.partial + text).split("\n");
    this.partial = parts.pop() ?? "";
    for (const line of parts) {
      this.lines.push(line);
      this.lineChars += line.length;
    }
    this.recount();
    this.trim();
  }

  /** 只清空视图；不改变已有流的订阅状态。 */
  clear(): void {
    this.lines = [];
    this.partial = "";
    this.lineChars = 0;
    this.chars = 0;
    this.wasTruncated = false;
  }

  /** 当前完整文本：已完成行以 `\n` 连接，未完成行追加在末尾。 */
  text(): string {
    const tail = this.partial ? `${this.lines.length ? "\n" : ""}${this.partial}` : "";
    return this.lines.join("\n") + tail;
  }

  /** 已完成行数 + 未完成行（若有）。 */
  get lineCount(): number {
    return this.lines.length + (this.partial ? 1 : 0);
  }

  /** 是否因上限裁剪过（一旦裁剪保持 true，直到 clear）。 */
  get truncated(): boolean {
    return this.wasTruncated;
  }

  private recount(): void {
    this.chars = this.lineChars + Math.max(0, this.lines.length - 1) + (this.partial ? this.partial.length + 1 : 0);
  }

  /** 从头部裁剪到上限内：先按行数超量丢行，再按字符数继续丢行。 */
  private trim(): void {
    let total = this.lines.length + (this.partial ? 1 : 0);
    if (total <= MAX_LOG_LINES && this.chars <= MAX_LOG_CHARS) return;
    this.wasTruncated = true;
    let drop = Math.max(0, total - MAX_LOG_LINES);
    let chars = this.chars;
    for (let i = 0; i < drop && i < this.lines.length; i += 1) chars -= this.lines[i].length + 1;
    while (drop < this.lines.length && chars > MAX_LOG_CHARS) {
      chars -= this.lines[drop].length + 1;
      drop += 1;
    }
    if (drop <= 0) return;
    this.lines.splice(0, drop);
    this.lineChars = 0;
    for (const line of this.lines) this.lineChars += line.length;
    this.recount();
  }
}
