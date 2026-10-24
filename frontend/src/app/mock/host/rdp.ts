/**
 * RDP 域夹具桥：把 mock/rdpFixture（帧流/证书挑战/失败分支）接到 rdp/* 方法上。
 * 参数语义（?rdpCert / ?rdpErr / ?autordp）见 rdpFixture 模块头。
 */

import { UNHANDLED, type Unhandled, base64, emitHostBinary, emitHostEvent } from "./shared";
import { context, fixtureParams } from "./fixture";
import { createRdpFixture } from "../rdpFixture";

const rdpFixture = createRdpFixture({
  params: fixtureParams,
  getWorkbenchId: () => context.workbenchId,
  emitEvent: (event) => emitHostEvent(event),
  emitBinary: (event) => emitHostBinary(event),
  base64,
});

export async function handleRdpInvoke(method: string, params: unknown): Promise<unknown | Unhandled> {
  if (method === "rdp/start") return rdpFixture.start(params as Record<string, unknown>);
  if (method === "rdp/resize") return rdpFixture.resize(params as Record<string, unknown>);
  if (method === "rdp/input" || method === "rdp/set-clipboard") return { success: true };
  if (method === "rdp/reconnect") return rdpFixture.reconnect(params as Record<string, unknown>);
  if (method === "rdp/close") {
    rdpFixture.close();
    return { success: true };
  }
  if (method === "rdp/list") return rdpFixture.list();
  if (method === "rdp/certificate/resolve") return rdpFixture.resolveCertificate(params as Record<string, unknown>);
  return UNHANDLED;
}
