/** RDP 远程桌面夹具（nyaterm-parity P3-4）：rdp/* 全链路。
 *
 * 帧补丁与 vnc/frame 同一 44 字节头（LE）+ RGBA；start 后推 connected →
 * 桌面帧序列 → rdp/pointer 光标形状；?rdpCert=1 先走证书挑战、
 * ?rdpErr=authfail|transport 走失败分支（见 mockDbxHost 顶部参数说明）。
 *
 * 已知偏差（mock vs 真实 sidecar，rdp-security-review【mock 偏差】清单）：
 * 已对齐 1（rdp/start 参数校验）、2/3（证书 resolve 校验 challengeId +
 * 一次性 + 120s fail-closed 超时）、7（帧序号跨重连单调）。仍保留的偏差：
 * - 偏差 4：mock 永不发 graceful closed 终态；transport 夹具以 error 收尾
 *   后不再补发 closed（真实 rdp_session.rs 终态 error 后会补 closed）。
 * - 偏差 5：rdp/list 固定 username "demo"/hasPassword true，不反映实际输入。
 * - 偏差 6：rdp/clipboard 无 CF_UNICODETEXT/16 MiB/分片语义，固定 900ms 推
 *   一条小文本。
 * - 偏差 8：rdp/reconnect 无 generation/重新 prompt 语义（直接重拨）。
 */
export interface RdpFixtureDeps {
  params: URLSearchParams;
  getWorkbenchId: () => unknown;
  emitEvent: (event: DbxPluginEvent) => void;
  emitBinary: (event: DbxPluginBinaryEvent) => void;
  base64: (bytes: Uint8Array) => string;
}

export interface RdpStartResult {
  sessionId: string;
  host: string;
  port: number;
  useNla: boolean;
  certificatePolicy: string;
  clipboard: boolean;
  reconnectAttempts: number;
}

export function createRdpFixture(deps: RdpFixtureDeps) {
  const { params, getWorkbenchId, emitEvent, emitBinary, base64 } = deps;
  const certChallenge = params.get("rdpCert") === "1";
  const errKind = params.get("rdpErr");

  let desktopW = 320;
  let desktopH = 200;
  let sessionId = "";
  // 帧序号全局单调（mock 偏差 7）：真实 sidecar 的 frame_sequence 跨代单调，
  // 前端按 `sequence <= lastSequence` 丢弃乱序补丁——mock 重连后归零会让画面
  // 假死并污染走查结论。新会话有新 sessionId，单调计数跨会话无副作用。
  let sequence = 0;
  let frameTimer = 0;
  let frameCount = 0;
  // 证书挑战闸门：resolve(accept=true) 后放行桌面帧（fail-closed：不 accept 不放行）。
  let certGate: (() => void) | null = null;
  // 一次性 challengeId（mock 偏差 2/3）：resolve 必须携带当前 id，已决/未知 id
  // 报错；120s 未决 fail-closed 超时。
  let certChallengeId = "";
  let certTimeoutTimer = 0;

  function emitState(state: string, extra: Record<string, unknown> = {}) {
    if (!sessionId) return;
    emitEvent({ method: "rdp/session/state", params: { sessionId, workbenchId: getWorkbenchId(), state, ...extra } });
  }

  /** 编码一个 44 字节 patch 头（LE）+ RGBA 负载的帧补丁（与 sidecar vnc 同构封装）。 */
  function framePatch(seq: number, x: number, y: number, width: number, height: number, payload: Uint8Array): Uint8Array {
    const frame = new Uint8Array(44 + payload.length);
    const view = new DataView(frame.buffer);
    view.setBigUint64(0, BigInt(seq), true);
    view.setUint32(8, desktopW, true);
    view.setUint32(12, desktopH, true);
    view.setUint32(16, x, true);
    view.setUint32(20, y, true);
    view.setUint32(24, width, true);
    view.setUint32(28, height, true);
    view.setUint32(32, width * 4, true);
    view.setUint32(36, 2, true); // RGBA8888
    view.setUint32(40, payload.length, true);
    frame.set(payload, 44);
    return frame;
  }

  function emitFrame() {
    if (!sessionId) return;
    sequence += 1;
    frameCount += 1;
    // 桌面底色一次整帧 + 之后一个移动色块补丁（验证增量 patch 绘制路径）。
    const tick = frameCount;
    if (tick === 1) {
      const payload = new Uint8Array(desktopW * desktopH * 4);
      for (let row = 0; row < desktopH; row += 1) {
        for (let col = 0; col < desktopW; col += 1) {
          const offset = (row * desktopW + col) * 4;
          payload[offset] = (col * 3) % 256;
          payload[offset + 1] = (row * 3) % 256;
          payload[offset + 2] = 96;
          payload[offset + 3] = 255;
        }
      }
      emitBinary({ channel: `rdp/frame/${sessionId}`, data: framePatch(sequence, 0, 0, desktopW, desktopH, payload) });
      return;
    }
    const patchW = 48;
    const patchH = 32;
    const payload = new Uint8Array(patchW * patchH * 4);
    for (let row = 0; row < patchH; row += 1) {
      for (let col = 0; col < patchW; col += 1) {
        const offset = (row * patchW + col) * 4;
        payload[offset] = 250;
        payload[offset + 1] = (col * 5) % 256;
        payload[offset + 2] = (row * 7 + tick * 5) % 256;
        payload[offset + 3] = 255;
      }
    }
    const x = 24 + ((tick * 13) % (desktopW - patchW - 48));
    const y = 24 + ((tick * 29) % (desktopH - patchH - 48));
    emitBinary({ channel: `rdp/frame/${sessionId}`, data: framePatch(sequence, x, y, patchW, patchH, payload) });
  }

  /** 16x16 白色箭头位图光标（RGBA + base64）：演示 rdp/pointer bitmap → CSS cursor。 */
  function bitmapCursor(): string {
    const size = 16;
    const rgba = new Uint8Array(size * size * 4);
    const inside = (x: number, y: number) => x <= y && y - x <= 11 && x <= 11;
    const outline = (x: number, y: number) => inside(x, y) && (!inside(x - 1, y) || !inside(x, y - 1) || !inside(x + 1, y) || !inside(x, y + 1));
    for (let row = 0; row < size; row += 1) {
      for (let col = 0; col < size; col += 1) {
        const offset = (row * size + col) * 4;
        if (outline(col, row)) {
          rgba[offset] = 16; rgba[offset + 1] = 16; rgba[offset + 2] = 18; rgba[offset + 3] = 255;
        } else if (inside(col, row)) {
          rgba[offset] = 255; rgba[offset + 1] = 255; rgba[offset + 2] = 255; rgba[offset + 3] = 255;
        }
      }
    }
    return base64(rgba);
  }

  function stopFrames() {
    if (frameTimer) {
      window.clearInterval(frameTimer);
      frameTimer = 0;
    }
  }

  /** 拨号成功后的桌面流：connected → 底帧+补丁 → 指针/位图光标。 */
  function startDesktop(target: string) {
    if (sessionId !== target) return;
    // sequence 不归零（mock 偏差 7）：跨重连单调，对齐真实 frame_sequence。
    frameCount = 0;
    emitState("connected");
    setTimeout(() => {
      if (sessionId !== target) return;
      emitFrame();
      frameTimer = window.setInterval(() => {
        if (frameCount >= 12) {
          stopFrames();
          return;
        }
        emitFrame();
      }, 120);
      emitEvent({ method: "rdp/pointer", params: { sessionId: target, type: "position", x: 160, y: 100 } });
      setTimeout(() => {
        if (sessionId !== target) return;
        emitEvent({ method: "rdp/pointer", params: { sessionId: target, type: "bitmap", width: 16, height: 16, hotspotX: 0, hotspotY: 0, rgbaBase64: bitmapCursor() } });
      }, 1600);
      setTimeout(() => {
        if (sessionId !== target) return;
        emitEvent({ method: "rdp/clipboard", params: { sessionId: target, text: "hello from rdp fixture" } });
      }, 900);
    }, 120);
  }

  function dial(target: string) {
    emitState("connecting");
    if (certChallenge) {
      certGate = () => {
        certGate = null;
        startDesktop(target);
      };
      // 一次性 challengeId + 120s fail-closed 超时（mock 偏差 2/3）：与真实
      // 一次性注册表 + CERTIFICATE_PROMPT_TIMEOUT 对齐，超时未决 = 拒绝。
      certChallengeId = `rdp-cert-visual-${Math.random().toString(36).slice(2, 8)}`;
      if (certTimeoutTimer) window.clearTimeout(certTimeoutTimer);
      certTimeoutTimer = window.setTimeout(() => {
        certTimeoutTimer = 0;
        if (sessionId !== target || !certGate) return;
        certGate = null;
        certChallengeId = "";
        emitState("error", { errorKind: "certificate", error: "certificate rejected" });
      }, 120000);
      setTimeout(() => {
        if (sessionId !== target) return;
        emitEvent({
          method: "connection/challenge",
          params: {
            challengeId: certChallengeId,
            kind: "rdp-certificate",
            sessionId: target,
            host: "rdp.demo.internal",
            port: 3389,
            fingerprint: "SHA256:9f86d081884c7d659a2feaa0c55ad015a3bf4f1b2b0b822cd15d6c15b0f00a08",
            knownHostStatus: "unknown",
          },
        });
      }, 80);
      return;
    }
    if (errKind === "authfail") {
      // 认证失败：永不自动重试，直接终态（文案统一 "RDP authentication failed"）。
      setTimeout(() => {
        if (sessionId !== target) return;
        emitState("error", { errorKind: "authentication", error: "RDP authentication failed" });
      }, 400);
      return;
    }
    startDesktop(target);
    if (errKind === "transport") {
      // 传输类失败：先跑一段桌面流，再走 reconnecting 退避梯子（1/2）→ 终态。
      setTimeout(() => {
        if (sessionId !== target) return;
        stopFrames();
        emitState("reconnecting", { attempt: 1, maxAttempts: 2 });
        setTimeout(() => {
          if (sessionId !== target) return;
          emitState("reconnecting", { attempt: 2, maxAttempts: 2 });
          setTimeout(() => {
            if (sessionId !== target) return;
            emitState("error", { errorKind: "transport", error: "connection reset by peer" });
          }, 1500);
        }, 1500);
      }, 2200);
    }
  }

  function validateDesktopSize(width: number, height: number) {
    if (width < 640 || width > 3840 || height < 480 || height > 2160) {
      throw new Error("RDP desktop size must be within 640x480 .. 3840x2160");
    }
  }

  function close() {
    stopFrames();
    if (certTimeoutTimer) {
      window.clearTimeout(certTimeoutTimer);
      certTimeoutTimer = 0;
    }
    certChallengeId = "";
    certGate = null;
    sessionId = "";
  }

  /** rdp/start：参数校验与真实 sidecar 同款（rdp_session.rs validate）。 */
  function start(input: Record<string, unknown>): RdpStartResult {
    const host = String(input.host ?? "").trim();
    if (!host) throw new Error("rdp/start: host is required");
    if (!String(input.username ?? "").trim()) throw new Error("rdp/start: username is required");
    const port = Number(input.port) || 3389;
    if (port <= 0 || port > 65535) throw new Error("rdp/start: port must be between 1 and 65535");
    const width = Number(input.width) || 1280;
    const height = Number(input.height) || 800;
    validateDesktopSize(width, height);
    const certificatePolicy = String(input.certificatePolicy || "prompt");
    if (!["prompt", "strict", "accept-temporarily"].includes(certificatePolicy)) {
      throw new Error("rdp/start: certificatePolicy must be prompt, strict or accept-temporarily");
    }
    desktopW = width;
    desktopH = height;
    close();
    sessionId = `visual-rdp-${Math.random().toString(36).slice(2, 8)}`;
    dial(sessionId);
    return {
      sessionId,
      host,
      port,
      useNla: input.useNla !== false,
      certificatePolicy,
      clipboard: input.clipboard !== false,
      reconnectAttempts: Number(input.reconnectAttempts) || 5,
    };
  }

  /** rdp/resize：对齐真实 sidecar 的 resize 语义（重连生效）。 */
  function resize(input: Record<string, unknown>) {
    const target = String(input.sessionId ?? "");
    if (!target || target !== sessionId) throw new Error("RDP session was not found");
    const width = Number(input.width) || 1280;
    const height = Number(input.height) || 800;
    validateDesktopSize(width, height);
    desktopW = width;
    desktopH = height;
    emitState("connecting");
    setTimeout(() => startDesktop(target), 120);
    return { sessionId: target, success: true, reconnected: true };
  }

  function reconnect(input: Record<string, unknown>) {
    const target = String(input.sessionId ?? "");
    if (!target || target !== sessionId) throw new Error("RDP session was not found");
    stopFrames();
    dial(target);
    return { sessionId: target, success: true };
  }

  function list() {
    return {
      sessions: sessionId
        ? [{ sessionId, workbenchId: getWorkbenchId(), host: "rdp.demo.internal", port: 3389, username: "demo", hasPassword: true, useNla: true, certificatePolicy: "prompt", clipboard: true, createdAt: Math.floor(Date.now() / 1000) }]
        : [],
    };
  }

  /** rdp/certificate/resolve：一次性注册表语义——必须携带当前签发的
   *  challengeId；未知/已决 id 报错（走查前端 showError 分支可达）。 */
  function resolveCertificate(input: Record<string, unknown>) {
    const challengeId = String(input.challengeId ?? "");
    if (!certChallengeId || challengeId !== certChallengeId) {
      throw new Error("RDP certificate challenge was not found or already resolved");
    }
    certChallengeId = "";
    if (certTimeoutTimer) {
      window.clearTimeout(certTimeoutTimer);
      certTimeoutTimer = 0;
    }
    const accepted = input.accept === true;
    if (accepted && certGate) {
      const gate = certGate;
      certGate = null;
      gate();
    } else if (!accepted) {
      // 拒绝 = fail-closed 终态（对齐真实 verify 失败路径）。
      emitState("error", { errorKind: "certificate", error: "certificate rejected" });
    }
    return { success: true };
  }

  // ?autordp=1（配合 ?proto=rdp）：夹具预拨一个活的 RDP 会话，让页面加载时
  // 走真实的 rdp/list 重接路径（模拟"DBX 重启后恢复活会话"的走查入口）。
  if (params.get("proto") === "rdp" && params.get("autordp") === "1") {
    sessionId = `visual-rdp-pre${Math.random().toString(36).slice(2, 6)}`;
    dial(sessionId);
  }

  return {
    get sessionId() {
      return sessionId;
    },
    start,
    resize,
    reconnect,
    close,
    list,
    resolveCertificate,
  };
}
