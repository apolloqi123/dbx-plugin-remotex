// historyPanel 纯逻辑单测：过滤（空 query 全量 / 模糊排序 / 子串回落 /
// 上限截断）、循环导航与越界收拢、打开门判定矩阵。
import { describe, expect, it } from "vitest";
import {
  HISTORY_PANEL_LIMIT,
  HISTORY_PANEL_PREFERRED_HEIGHT,
  HISTORY_TIMES_LIMIT,
  canOpenHistoryPanel,
  chooseHistoryPanelPlacement,
  clampHistoryPanelIndex,
  commandHistoryHotkeyPassthrough,
  decorateHistoryEntries,
  historyPanelConfirmConsumesKey,
  historyPanelMaxHeight,
  historyPanelPreferredHeight,
  persistableHistoryMetaBuckets,
  pruneHistoryTimesBuckets,
  recordHistoryMeta,
  sanitizeHistoryMeta,
  sanitizeHistoryMetaBuckets,
  sanitizeHistoryTimesBuckets,
  filterHistoryEntries,
  moveHistoryPanelIndex,
  pruneHistoryTimes,
  recordHistoryTime,
  relativeHistoryAge,
  resolveHistoryPanelKey,
  sanitizeHistoryTimes,
} from "./historyPanel";

const HISTORY = [
  "kubectl get pods -n prod",
  "tail -f /var/log/syslog",
  "docker compose up -d",
  "git status",
  "echo hello",
];

describe("filterHistoryEntries", () => {
  it("empty query returns the newest entries capped at the limit, oldest on top (shell ↑ direction)", () => {
    const expected = [...HISTORY].reverse();
    expect(filterHistoryEntries(HISTORY, "")).toEqual(expected);
    expect(filterHistoryEntries(HISTORY, "   ")).toEqual(expected);
    const long = Array.from({ length: HISTORY_PANEL_LIMIT + 10 }, (_, i) => `cmd-${i}`);
    expect(filterHistoryEntries(long, "")).toHaveLength(HISTORY_PANEL_LIMIT);
    // 取最新的 50 条(cmd-0..cmd-49,cmd-0 最新)后反转:底部是最新。
    expect(filterHistoryEntries(long, "")[0]).toBe("cmd-49");
    expect(filterHistoryEntries(long, "")[HISTORY_PANEL_LIMIT - 1]).toBe("cmd-0");
  });

  it("ranks fuzzy matches by subsequence score, most-relevant at the bottom", () => {
    const rows = filterHistoryEntries(HISTORY, "kube");
    expect(rows[0]).toBe("kubectl get pods -n prod");
  });

  it("ranks multiple fuzzy hits by score and reverses into oldest-on-top order", () => {
    // "up"：docker 的 "up" 词首+连续命中得分最高；kubectl 里 u…p 的跨段子序列
    // 次之；其余无子序列命中。输出旧上新下:相关性最高的 docker 在底部。
    expect(filterHistoryEntries(HISTORY, "up")).toEqual(["kubectl get pods -n prod", "docker compose up -d"]);
  });

  it("handles over-long queries (beyond the search length gate) via substring fallback", () => {
    const long = "kubectl get pods -n prod and a very long continuation".slice(0, 80);
    expect(filterHistoryEntries(HISTORY, long)).toEqual([]);
    expect(filterHistoryEntries([...HISTORY, `run ${long}`], long)).toEqual([`run ${long}`]);
  });

  it("returns empty for queries matching nothing", () => {
    expect(filterHistoryEntries(HISTORY, "zzzz")).toEqual([]);
  });
});

describe("moveHistoryPanelIndex / clampHistoryPanelIndex", () => {
  it("wraps around both edges", () => {
    expect(moveHistoryPanelIndex(0, -1, 3)).toBe(2);
    expect(moveHistoryPanelIndex(2, 1, 3)).toBe(0);
    expect(moveHistoryPanelIndex(1, 1, 3)).toBe(2);
  });

  it("stays at zero for an empty list", () => {
    expect(moveHistoryPanelIndex(0, 1, 0)).toBe(0);
    expect(clampHistoryPanelIndex(5, 0)).toBe(0);
  });

  it("clamps after re-filtering shrinks the list", () => {
    expect(clampHistoryPanelIndex(7, 3)).toBe(2);
    expect(clampHistoryPanelIndex(-1, 3)).toBe(0);
  });
});

describe("resolveHistoryPanelKey (shell ↑ 语义键位映射)", () => {
  it("moves within the list and stays at the oldest (top) edge instead of wrapping", () => {
    expect(resolveHistoryPanelKey("ArrowUp", 2, 3)).toEqual({ kind: "move", delta: -1 });
    expect(resolveHistoryPanelKey("ArrowDown", 0, 3)).toEqual({ kind: "move", delta: 1 });
    // 顶部是最旧一条：再 ↑ 停住（消费但不回绕到最新）。
    expect(resolveHistoryPanelKey("ArrowUp", 0, 3)).toEqual({ kind: "stay" });
  });

  it("cancels on Esc and on ↓ past the newest (bottom) entry", () => {
    expect(resolveHistoryPanelKey("Escape", 1, 3)).toEqual({ kind: "cancel" });
    expect(resolveHistoryPanelKey("ArrowDown", 2, 3)).toEqual({ kind: "cancel" });
    expect(resolveHistoryPanelKey("ArrowDown", 0, 1)).toEqual({ kind: "cancel" });
  });

  it("fills the active entry on Enter/Tab and closes when the list is empty", () => {
    expect(resolveHistoryPanelKey("Enter", 2, 3)).toEqual({ kind: "fill" });
    expect(resolveHistoryPanelKey("Tab", 0, 1)).toEqual({ kind: "fill" });
    expect(resolveHistoryPanelKey("Enter", 0, 0)).toEqual({ kind: "close" });
    expect(resolveHistoryPanelKey("Tab", 0, 0)).toEqual({ kind: "close" });
  });

  it("lets unrelated keys pass through to the shell", () => {
    expect(resolveHistoryPanelKey("a", 1, 3)).toBeNull();
    expect(resolveHistoryPanelKey("Home", 1, 3)).toBeNull();
  });
});

describe("historyPanelConfirmConsumesKey (Enter 仅回填，不执行)", () => {
  it("always swallows Enter so the highlighted command is only filled, never executed", () => {
    // 命令行路径与搜索框路径都消费 Enter：行内容已在导航时回填，再按一次
    // 回车才交给 shell 执行（用户反馈：导航历史时回车不应直接跑命令）。
    expect(historyPanelConfirmConsumesKey("Enter", false)).toBe(true);
    expect(historyPanelConfirmConsumesKey("Enter", true)).toBe(true);
  });

  it("lets Tab through to shell completion from the terminal path", () => {
    expect(historyPanelConfirmConsumesKey("Tab", false)).toBe(false);
    expect(historyPanelConfirmConsumesKey("Tab", true)).toBe(true);
  });
});

describe("canOpenHistoryPanel", () => {
  const open = {
    completionOpen: false,
    suggestionOpen: false,
    quickSelectOpen: false,
    searchOpen: false,
    alternateActive: false,
    commandRunning: false,
    transferBusy: false,
    interactivePromptPending: false,
  };

  it("opens when every gate is clear", () => {
    expect(canOpenHistoryPanel(open)).toBe(true);
  });

  it("is blocked while any overlay, alternate screen, running command or transfer owns the input", () => {
    for (const key of Object.keys(open) as Array<keyof typeof open>) {
      expect(canOpenHistoryPanel({ ...open, [key]: true })).toBe(false);
    }
  });

  it("does not grab ↑ while an interactive prompt is pending (#150: MFA/选择场景 ↑ 留给远端)", () => {
    expect(canOpenHistoryPanel({ ...open, interactivePromptPending: true })).toBe(false);
  });
});

describe("commandHistoryHotkeyPassthrough", () => {
  it("passes through when the master switch is off (#169: Ctrl+R 留给 readline 反向搜索)", () => {
    expect(commandHistoryHotkeyPassthrough({ enabled: false, alternateActive: false })).toBe(true);
  });

  it("passes through on the alternate screen (#108 反馈二: vim 的 Ctrl+R 是 redo，不吞键)", () => {
    expect(commandHistoryHotkeyPassthrough({ enabled: true, alternateActive: true })).toBe(true);
    expect(commandHistoryHotkeyPassthrough({ enabled: false, alternateActive: true })).toBe(true);
  });

  it("keeps swallowing with the panel enabled on the normal screen（传输占用/OTP 待答等 gate 关闭场景不透传 ^R）", () => {
    expect(commandHistoryHotkeyPassthrough({ enabled: true, alternateActive: false })).toBe(false);
  });
});

describe("history times (Warp 式相对时间的数据面)", () => {
  it("sanitizeHistoryTimes keeps only well-formed entries and caps to the limit", () => {
    expect(sanitizeHistoryTimes(null)).toEqual({});
    expect(sanitizeHistoryTimes("junk")).toEqual({});
    expect(sanitizeHistoryTimes([{ c: "ls", t: 100 }, { c: "", t: 5 }, { c: "rm", t: -1 }, { c: "ok", t: "x" }, "junk", null])).toEqual({ ls: 100 });
    const flood = Array.from({ length: HISTORY_TIMES_LIMIT + 10 }, (_, i) => ({ c: `cmd-${i}`, t: i + 1 }));
    const sanitized = sanitizeHistoryTimes(flood);
    expect(Object.keys(sanitized)).toHaveLength(HISTORY_TIMES_LIMIT);
    expect(sanitized["cmd-0"]).toBeUndefined();
    expect(sanitized[`cmd-${HISTORY_TIMES_LIMIT + 9}`]).toBe(HISTORY_TIMES_LIMIT + 10);
  });

  it("recordHistoryTime upserts and evicts the oldest beyond the limit", () => {
    let times = recordHistoryTime({}, "a", 1);
    times = recordHistoryTime(times, "b", 2);
    times = recordHistoryTime(times, "a", 3);
    expect(times).toEqual({ b: 2, a: 3 });
    for (let i = 0; i < HISTORY_TIMES_LIMIT; i += 1) times = recordHistoryTime(times, `x${i}`, 10 + i);
    // 102 条裁到 100：按时间序淘汰最旧的两条（b@2、a@3）。
    expect(Object.keys(times)).toHaveLength(HISTORY_TIMES_LIMIT);
    expect(times.b).toBeUndefined();
    expect(times.a).toBeUndefined();
    expect(times.x0).toBe(10);
    expect(times.x99).toBe(109);
    expect(recordHistoryTime(times, "   ", 99)).toEqual(times);
  });

  it("pruneHistoryTimes drops entries no longer present in the history ring", () => {
    expect(pruneHistoryTimes({ a: 1, b: 2, gone: 3 }, ["a", "b"])).toEqual([{ c: "a", t: 1 }, { c: "b", t: 2 }]);
  });

  it("decorateHistoryEntries attaches timestamps, null when unknown", () => {
    expect(decorateHistoryEntries(["ls", "cd .."], { ls: 42 })).toEqual([
      { command: "ls", ts: 42, durationMs: null, exitCode: null },
      { command: "cd ..", ts: null, durationMs: null, exitCode: null },
    ]);
  });
});

describe("history meta (批 4d 富元数据: 时长/退出码)", () => {
  it("sanitizeHistoryMeta keeps valid rows, drops invalid, caps at limit", () => {
    const meta = sanitizeHistoryMeta([
      { c: "ls -la", d: 1200, x: 0 },
      { c: "fail", x: 1 },
      { c: "timed", d: 500 },
      { c: "", d: 1 },
      { c: "norow" },
      "junk",
    ]);
    expect(meta).toEqual({
      "ls -la": { durationMs: 1200, exitCode: 0 },
      fail: { durationMs: null, exitCode: 1 },
      timed: { durationMs: 500, exitCode: null },
    });
    expect(sanitizeHistoryMeta("junk")).toEqual({});
  });

  it("recordHistoryMeta merges partial patches and prunes to live commands", () => {
    const live = new Set(["ls", "gone"]);
    let meta = recordHistoryMeta({}, "ls", { exitCode: 0 }, live);
    meta = recordHistoryMeta(meta, "ls", { durationMs: 300 }, live);
    meta = recordHistoryMeta(meta, "gone", { exitCode: 2 }, live);
    meta = recordHistoryMeta(meta, "unknown", { exitCode: 9 }, live);
    expect(meta).toEqual({ ls: { durationMs: 300, exitCode: 0 }, gone: { durationMs: null, exitCode: 2 } });
  });

  it("decorateHistoryEntries carries meta into panel rows", () => {
    const entries = decorateHistoryEntries(["fail"], {}, { fail: { durationMs: 1500, exitCode: 1 } });
    expect(entries).toEqual([{ command: "fail", ts: null, durationMs: 1500, exitCode: 1 }]);
  });
});

describe("relativeHistoryAge", () => {
  const now = 1_000_000_000_000;

  it("never for missing or bogus timestamps (incl. future drift)", () => {
    expect(relativeHistoryAge(null, now)).toEqual({ kind: "never" });
    expect(relativeHistoryAge(0, now)).toEqual({ kind: "never" });
    expect(relativeHistoryAge(now + 60_000, now)).toEqual({ kind: "never" });
  });

  it("buckets into just-now / minutes / hours / days", () => {
    expect(relativeHistoryAge(now - 30_000, now)).toEqual({ kind: "just-now" });
    expect(relativeHistoryAge(now - 5 * 60_000, now)).toEqual({ kind: "minutes", count: 5 });
    expect(relativeHistoryAge(now - 3 * 3_600_000, now)).toEqual({ kind: "hours", count: 3 });
    expect(relativeHistoryAge(now - 50 * 3_600_000, now)).toEqual({ kind: "days", count: 2 });
  });
});

describe("historyPanelPreferredHeight / historyPanelMaxHeight (面板尺寸与光标位置解耦)", () => {
  it("caps the preferred height on tall viewports and shrinks on short ones", () => {
    expect(historyPanelPreferredHeight(900)).toBe(HISTORY_PANEL_PREFERRED_HEIGHT);
    expect(historyPanelPreferredHeight(400)).toBeCloseTo(220);
    expect(historyPanelPreferredHeight(0)).toBe(0);
  });

  it("gives the panel the same height whether the input line sits at the top or the bottom", () => {
    // 体验反馈：光标在上方时面板很小、在底部又很大——两侧现在都被理想高度
    // 钳住，顶部翻到下方、底部留在上方，尺寸一致。
    const top = chooseHistoryPanelPlacement(20, 17, 900);
    const bottom = chooseHistoryPanelPlacement(880, 17, 900);
    expect(top).toBe("below");
    expect(bottom).toBe("above");
    expect(historyPanelMaxHeight(top, 20, 17, 900)).toBe(HISTORY_PANEL_PREFERRED_HEIGHT);
    expect(historyPanelMaxHeight(bottom, 880, 17, 900)).toBe(HISTORY_PANEL_PREFERRED_HEIGHT);
  });

  it("never exceeds the space available on the chosen side", () => {
    // 矮视口两侧都不足理想高度：取更大一侧，并按该侧可用空间收窄。
    expect(chooseHistoryPanelPlacement(100, 17, 240)).toBe("below");
    expect(historyPanelMaxHeight("below", 100, 17, 240)).toBe(117);
  });
});

describe("chooseHistoryPanelPlacement (优先上方，按理想高度选侧)", () => {
  it("keeps the panel above the input line when both sides can host the preferred height", () => {
    expect(chooseHistoryPanelPlacement(500, 17, 900)).toBe("above");
    expect(chooseHistoryPanelPlacement(450, 17, 900)).toBe("above");
  });

  it("flips below when above is too cramped for a comfortable panel", () => {
    // 光标在上方四分之一处：上方 194px 放不下 360px 面板，下方充足。
    expect(chooseHistoryPanelPlacement(200, 17, 900)).toBe("below");
    // 光标贴视口顶部（如刚 clear）。
    expect(chooseHistoryPanelPlacement(20, 17, 900)).toBe("below");
    // 矮视口两侧都不足：取更大一侧（面板收窄 + 内部滚动）。
    expect(chooseHistoryPanelPlacement(130, 17, 300)).toBe("below");
    expect(chooseHistoryPanelPlacement(150, 17, 300)).toBe("above");
  });

  // 体验回归（#152 反馈）：光标在视口中部的刀锋线附近时，回填换行使锚点
  // 单行抖动，无迟滞的决策会随每次 ↑↓ 导航反复翻面（忽上忽下）。打开期间
  // 传入 previous 后，两侧容量相近时维持原侧。
  it("hysteresis: keeps the current side through single-row anchor jitter", () => {
    // 两侧都能放下完整面板（容量都被钳到理想高度）：y 在 433↔450 抖动不翻面。
    expect(chooseHistoryPanelPlacement(450, 17, 900, undefined, "above")).toBe("above");
    expect(chooseHistoryPanelPlacement(433, 17, 900, undefined, "above")).toBe("above");
    expect(chooseHistoryPanelPlacement(450, 17, 900, undefined, "below")).toBe("below");
    expect(chooseHistoryPanelPlacement(433, 17, 900, undefined, "below")).toBe("below");
  });

  it("hysteresis still flips when the current side is clearly worse", () => {
    // 上侧 144px vs 下侧 360px：另一侧明显更大才翻。
    expect(chooseHistoryPanelPlacement(150, 17, 900, undefined, "above")).toBe("below");
    expect(chooseHistoryPanelPlacement(20, 17, 900, undefined, "above")).toBe("below");
    // 翻到新侧后成为当前侧，立即获得同样的迟滞保护（不回翻）。
    expect(chooseHistoryPanelPlacement(20, 17, 900, undefined, "below")).toBe("below");
  });

  it("viewportless input keeps the previous side (defaults to above)", () => {
    expect(chooseHistoryPanelPlacement(150, 17, 0)).toBe("above");
    expect(chooseHistoryPanelPlacement(150, 17, 0, undefined, "below")).toBe("below");
  });

  it("keeps above when the viewport is unmeasurable", () => {
    expect(chooseHistoryPanelPlacement(300, 17, 0)).toBe("above");
  });
});

describe("分桶历史档（scope = 连接 id，本地/串口固定桶）", () => {
  it("sanitizeHistoryTimesBuckets migrates the legacy flat array into the legacy scope", () => {
    expect(sanitizeHistoryTimesBuckets([{ c: "ls", t: 100 }], { legacyScope: "local" })).toEqual({ local: { ls: 100 } });
    expect(sanitizeHistoryTimesBuckets(null, { legacyScope: "local" })).toEqual({});
    expect(sanitizeHistoryTimesBuckets([{ c: "", t: 5 }], { legacyScope: "local" })).toEqual({});
  });

  it("sanitizeHistoryTimesBuckets sanitizes each scope independently", () => {
    const raw = {
      "conn-1": [{ c: "ls", t: 100 }, { c: "bad", t: -1 }],
      "conn-2": "junk",
      "": [{ c: "dropped", t: 1 }],
    };
    expect(sanitizeHistoryTimesBuckets(raw, { legacyScope: "local" })).toEqual({ "conn-1": { ls: 100 } });
  });

  it("sanitizeHistoryMetaBuckets migrates the legacy flat array into the legacy scope", () => {
    expect(sanitizeHistoryMetaBuckets([{ c: "ls", d: 12, x: 0 }], { legacyScope: "local" })).toEqual({ local: { ls: { durationMs: 12, exitCode: 0 } } });
    expect(sanitizeHistoryMetaBuckets("junk", { legacyScope: "local" })).toEqual({});
  });

  it("pruneHistoryTimesBuckets prunes each scope against its own ring and drops empty scopes", () => {
    const times = { "conn-1": { a: 1, gone: 3 }, "conn-2": { gone: 4 } };
    const rings = { "conn-1": ["a", "b"] };
    expect(pruneHistoryTimesBuckets(times, rings)).toEqual({ "conn-1": [{ c: "a", t: 1 }] });
  });

  it("persistableHistoryMetaBuckets keeps only ring-live rows per scope and drops empty scopes", () => {
    const meta = { "conn-1": { a: { durationMs: 1, exitCode: 0 }, gone: { durationMs: 2, exitCode: 1 } } };
    const rings = { "conn-1": ["a"], "conn-2": [] };
    expect(persistableHistoryMetaBuckets(meta, rings)).toEqual({ "conn-1": [{ c: "a", d: 1, x: 0 }] });
  });
});
