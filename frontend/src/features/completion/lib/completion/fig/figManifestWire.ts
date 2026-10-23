// fig 语料的跨线程数据面（bundle 去重）：生成的 spec manifest 含少量函数值
// （generator postProcess/custom、generateSpec、parserDirectives.alias 等），
// structured clone 无法携带——序列化时替换为**标记对象**，保留字段存在性与
// 数组长度语义，使 worker 侧 resolve 与主线程逐分支一致（函数体只被
// collectGenerators / prewarm 使用，二者均不在 worker 内执行）。
//
// 唯一静态副本在主线程（figCompletionSource.ts）；worker 经 init 消息接收本
// 模块产出的 JSON，inline worker 不再内嵌第二份 6.5 MB 语料。

import type { FigSpecManifest } from "./source";

/** 标记对象的唯一键：函数值在数据面的占位形态。 */
const FUNCTION_MARKER_KEY = "__figWireFunction";
/** 稀疏数组“洞”的占位键：JSON 会把洞写成 null（如 next 的
 * `name: ["-H", , "--hostname"]` 会被 decode 注册成名为 "null" 的选项）。 */
const ARRAY_HOLE_MARKER_KEY = "__figWireArrayHole";

export const FIG_FUNCTION_MARKER_KEY = FUNCTION_MARKER_KEY;
export const FIG_ARRAY_HOLE_MARKER_KEY = ARRAY_HOLE_MARKER_KEY;

/** 序列化占位（值语义的“不可调用函数”）。 */
export function figFunctionMarker(): Record<string, true> {
  return { [FUNCTION_MARKER_KEY]: true };
}

/** 序列化占位（数组洞；decode 时删除该下标还原稀疏形态）。 */
export function figArrayHoleMarker(): Record<string, true> {
  return { [ARRAY_HOLE_MARKER_KEY]: true };
}

/** 是否为数据面函数标记（仅接受恰好一个键的形态，避免误判 spec 数据）。 */
export function isFigFunctionMarker(value: unknown): boolean {
  if (typeof value !== "object" || value === null) return false;
  const record = value as Record<string, unknown>;
  const keys = Object.keys(record);
  return keys.length === 1 && keys[0] === FUNCTION_MARKER_KEY && record[FUNCTION_MARKER_KEY] === true;
}

/** 是否为数据面数组洞标记（同样只接受恰好一个键的形态）。 */
export function isFigArrayHoleMarker(value: unknown): boolean {
  if (typeof value !== "object" || value === null) return false;
  const record = value as Record<string, unknown>;
  const keys = Object.keys(record);
  return keys.length === 1 && keys[0] === ARRAY_HOLE_MARKER_KEY && record[ARRAY_HOLE_MARKER_KEY] === true;
}

/** resolve 语义里的“函数形态”：真实函数或数据面标记（不可调用，但存在）。 */
export function isFigCallableValue(value: unknown): boolean {
  return typeof value === "function" || isFigFunctionMarker(value);
}

/** manifest → 跨线程 JSON（函数值转标记；数组洞转洞标记；其余保持 JSON 数据形态）。 */
export function encodeFigManifest(manifest: FigSpecManifest): string {
  return JSON.stringify(manifest, function (this: unknown, _key, value) {
    if (typeof value === "function") return figFunctionMarker();
    // JSON.stringify 对稀疏数组的洞写入 null。replacer 的 this 是持有者、
    // key 是下标：下标不在持有者上即洞，用标记保留“洞 ≠ null”的语义。
    if (
      value === undefined &&
      Array.isArray(this) &&
      typeof _key === "string" &&
      !(_key in this)
    ) {
      return figArrayHoleMarker();
    }
    return value;
  });
}

/** 跨线程 JSON → manifest 数据面；非对象载荷视为协议错误（调用方兜底）。
 * 洞标记原位删除下标，还原主线程语料的稀疏数组形态（forEach/map 跳过洞）。 */
export function decodeFigManifest(json: string): FigSpecManifest {
  const parsed: unknown = JSON.parse(json);
  if (typeof parsed !== "object" || parsed === null || Array.isArray(parsed)) {
    throw new Error("fig manifest wire payload must be an object");
  }
  // 绝大多数载荷不含洞：跳过整树遍历（worker init 的 6.5 MB 语料零额外成本）。
  if (json.includes(ARRAY_HOLE_MARKER_KEY)) restoreArrayHoles(parsed);
  return parsed as FigSpecManifest;
}

/** 递归把洞标记还原为稀疏数组下标（delete 保持 length 不变）。 */
function restoreArrayHoles(value: unknown): void {
  if (Array.isArray(value)) {
    for (let index = 0; index < value.length; index += 1) {
      const item = value[index];
      if (isFigArrayHoleMarker(item)) {
        delete value[index];
        continue;
      }
      restoreArrayHoles(item);
    }
    return;
  }
  if (typeof value !== "object" || value === null) return;
  for (const key of Object.keys(value)) {
    restoreArrayHoles((value as Record<string, unknown>)[key]);
  }
}
