// FigCompletionSource 主线程入口：全量 spec manifest 的**唯一静态副本**在此
// 注入（figSourceCore 只接受注入语料）。inline worker 经 init 消息接收
// figManifestWire 序列化的数据面副本，ui/index.html 因此只内嵌一份语料。
//
// 本模块保持批次 1/2-1 的既有导出面：`FigCompletionSourceImpl`、
// `createFigCompletionSource`、默认实例 `figCompletionSource`（App 的
// generator 通道直引其 collectGenerators）。

// @ts-nocheck 无法只作用于单条 import；manifest 为生成物（自带 @ts-nocheck）
import generatedManifest from "../../../../../../vendor/fig-specs/spec-manifest.generated";
import type { FigSpecManifest } from "./source";
import { encodeFigManifest } from "./figManifestWire";
import { FigCompletionSourceImpl, type FigSourceOptions } from "./figSourceCore";

export { FigCompletionSourceImpl };
export type { FigSourceOptions };
export type { FigSourceRuntime } from "./figSourceCore";

/** 生成物是 `Readonly<Record<string, FigSpecRaw>>`；接缝类型不透明，直接收敛。 */
const manifestStore = generatedManifest as unknown as FigSpecManifest;

/** 测试/fixture 注入点：省略 manifest 时使用生成的全量语料（既有调用面）。 */
export function createFigCompletionSource(
  options: Partial<FigSourceOptions> = {},
): FigCompletionSourceImpl {
  return new FigCompletionSourceImpl({ manifest: options.manifest ?? manifestStore });
}

export const figCompletionSource = createFigCompletionSource();

export default figCompletionSource;

let workerManifestJson: string | null = null;

/** worker init 载荷：语料数据面的 JSON 字符串（首次调用时序列化并缓存，
 * worker 崩溃重启复用同一份）。主线程语料是唯一副本，函数转标记。 */
export function serializeFigManifestForWorker(): string {
  if (workerManifestJson === null) {
    workerManifestJson = encodeFigManifest(manifestStore);
  }
  return workerManifestJson;
}
