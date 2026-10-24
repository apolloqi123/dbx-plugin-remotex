/**
 * OTP 域夹具（otp/*）：镜像 sidecar 的脱敏视图形状（hasSecret 不回传
 * secret）与 bindings 映射（connectionId → entryId）。generate 不做真 RFC
 * 6238，用密钥+时间窗的确定性伪码，剩余秒数按 period 取模，让倒计时环真实走动。
 */

import { UNHANDLED, type Unhandled } from "./shared";

interface MockOtpEntry {
  id: string;
  otpType: "totp" | "hotp";
  issuer: string;
  username: string;
  secret: string;
  algorithm: string;
  digits: number;
  period: number;
  counter: number | null;
  createdAt: number;
  updatedAt: number;
}
const otpEntriesState: MockOtpEntry[] = [];
const otpBindingsState: Record<string, string> = {};
let otpSeq = 0;

function fakeOtpCode(entry: MockOtpEntry): { code: string; remainingSeconds: number } {
  const nowSec = Math.floor(Date.now() / 1000);
  if (entry.otpType === "hotp") {
    entry.counter = (entry.counter ?? 0) + 1;
    const material = `${entry.secret}:${entry.counter}`;
    let h = 2166136261;
    for (let i = 0; i < material.length; i += 1) {
      h ^= material.charCodeAt(i);
      h = Math.imul(h, 16777619);
    }
    return { code: String((h >>> 0) % 10 ** entry.digits).padStart(entry.digits, "0"), remainingSeconds: 0 };
  }
  const step = Math.floor(nowSec / entry.period);
  const material = `${entry.secret}:${step}`;
  let h = 2166136261;
  for (let i = 0; i < material.length; i += 1) {
    h ^= material.charCodeAt(i);
    h = Math.imul(h, 16777619);
  }
  return { code: String((h >>> 0) % 10 ** entry.digits).padStart(entry.digits, "0"), remainingSeconds: entry.period - (nowSec % entry.period) };
}

export async function handleOtpInvoke(method: string, params: unknown): Promise<unknown | Unhandled> {
  if (method === "otp/list") {
    return {
      entries: otpEntriesState.map(({ secret, ...rest }) => ({ ...rest, hasSecret: secret.length > 0 })),
      bindings: { ...otpBindingsState },
    };
  }
  if (method === "otp/save") {
    const input = (params || {}) as Record<string, unknown>;
    const id = String(input.id || "");
    const secret = String(input.secret || "");
    const digits = Math.min(10, Math.max(4, Number(input.digits) || 6));
    const period = Math.max(1, Number(input.period) || 30);
    if (id) {
      const entry = otpEntriesState.find((item) => item.id === id);
      if (!entry) throw new Error("otp: no such entry");
      entry.otpType = input.otpType === "hotp" ? "hotp" : "totp";
      entry.issuer = String(input.issuer || "");
      entry.username = String(input.username || "");
      if (secret) entry.secret = secret;
      entry.algorithm = String(input.algorithm || "SHA1");
      entry.digits = digits;
      entry.period = period;
      if (input.otpType === "hotp") entry.counter = Number(input.counter) || 0;
      entry.updatedAt = Math.floor(Date.now() / 1000);
      return { entry: { ...entry, secret: undefined, hasSecret: entry.secret.length > 0 } };
    }
    if (!secret) throw new Error("otp: secret required");
    const entry: MockOtpEntry = {
      id: `otp-visual-${++otpSeq}`,
      otpType: input.otpType === "hotp" ? "hotp" : "totp",
      issuer: String(input.issuer || ""),
      username: String(input.username || ""),
      secret,
      algorithm: String(input.algorithm || "SHA1"),
      digits,
      period,
      counter: input.otpType === "hotp" ? Number(input.counter) || 0 : null,
      createdAt: Math.floor(Date.now() / 1000),
      updatedAt: Math.floor(Date.now() / 1000),
    };
    otpEntriesState.push(entry);
    return { entry: { id: entry.id, otpType: entry.otpType, issuer: entry.issuer, username: entry.username, hasSecret: true, algorithm: entry.algorithm, digits: entry.digits, period: entry.period, counter: entry.counter } };
  }
  if (method === "otp/delete") {
    const id = String((params as Record<string, unknown>)?.id || "");
    const index = otpEntriesState.findIndex((item) => item.id === id);
    if (index >= 0) otpEntriesState.splice(index, 1);
    for (const [connectionId, bound] of Object.entries(otpBindingsState)) {
      if (bound === id) delete otpBindingsState[connectionId];
    }
    return { deleted: index >= 0 };
  }
  if (method === "otp/generate") {
    const id = String((params as Record<string, unknown>)?.entryId || "");
    const entry = otpEntriesState.find((item) => item.id === id);
    if (!entry) throw new Error("otp: no such entry");
    return fakeOtpCode(entry);
  }
  if (method === "otp/bind") {
    const input = (params || {}) as Record<string, unknown>;
    otpBindingsState[String(input.connectionId || "")] = String(input.entryId || "");
    return { success: true };
  }
  if (method === "otp/unbind") {
    const connectionId = String((params as Record<string, unknown>)?.connectionId || "");
    const removed = delete otpBindingsState[connectionId];
    return { removed };
  }
  if (method === "otp/import-qr") {
    // 无真解码器：返回确定性的演示条目草稿，让「导入二维码」在夹具里可走查。
    return { otpType: "totp", issuer: "QR Import", label: "QR Import:demo@example.com", secretBase32: "JBSWY3DPEHPK3PXP", algorithm: "SHA1", digits: 6, period: 30, counter: null };
  }
  return UNHANDLED;
}
