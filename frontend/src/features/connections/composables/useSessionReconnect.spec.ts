// @vitest-environment happy-dom
// 重连梯子第一级策略回归：先复用现有连接直接重开会话；只有落到 error 态
// （典型是宿主侧凭据已更新、sidecar 仍持旧配置）才请宿主重放连接再试一次。
// 宿主重放会断开整条连接，连带杀掉同连接其它标签的会话与 SFTP 通道，不应
// 为单条 PTY 掉线付出这个代价。
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { defineComponent, ref } from "vue";
import { mount, type VueWrapper } from "@vue/test-utils";
import { useSessionReconnect } from "./useSessionReconnect";
import { TERMINAL_RECONNECT_DELAYS } from "../../terminal/lib/behavior/terminalReconnect";

type ReconnectApi = ReturnType<typeof useSessionReconnect>;

function setup(openSessionImpl: () => Promise<void>) {
  const terminalState = ref("connecting");
  const terminalError = ref("");
  const openSession = vi.fn(openSessionImpl);
  const requestHostReopenConnection = vi.fn(async () => true);
  let reconnect!: ReconnectApi;
  // 组件作用域内创建：梯子用 onBeforeUnmount 清理定时器，脱离实例会告警。
  const Host = defineComponent({
    setup() {
      reconnect = useSessionReconnect({
        t: (key) => key,
        terminalState,
        terminalError,
        isDisposed: () => false,
        openSession,
        requestHostReopenConnection,
      });
      return () => null;
    },
  });
  const wrapper = mount(Host);
  return { reconnect, terminalState, requestHostReopenConnection, openSession, wrapper };
}

let active: VueWrapper | null = null;

describe("useSessionReconnect (第一级先复用连接)", () => {
  beforeEach(() => vi.useFakeTimers());
  afterEach(() => {
    active?.unmount();
    active = null;
    vi.useRealTimers();
  });

  function mountWith(openSessionImpl: () => Promise<void>) {
    const context = setup(openSessionImpl);
    active = context.wrapper;
    return context;
  }

  it("reopens the session on the existing transport without resetting the host connection", async () => {
    const { reconnect, terminalState, requestHostReopenConnection, openSession } = mountWith(async () => {
      terminalState.value = "connected";
    });
    reconnect.scheduleSessionReconnect();
    await vi.advanceTimersByTimeAsync(TERMINAL_RECONNECT_DELAYS[0]);

    expect(openSession).toHaveBeenCalledTimes(1);
    expect(openSession).toHaveBeenCalledWith();
    expect(requestHostReopenConnection).not.toHaveBeenCalled();
  });

  it("falls back to a host connection re-push only when the direct attempt ends in error", async () => {
    const { reconnect, terminalState, requestHostReopenConnection, openSession } = mountWith(async () => {
      terminalState.value = "error";
    });
    reconnect.scheduleSessionReconnect();
    await vi.advanceTimersByTimeAsync(TERMINAL_RECONNECT_DELAYS[0]);

    expect(requestHostReopenConnection).toHaveBeenCalledTimes(1);
    expect(openSession).toHaveBeenCalledTimes(2);
    // 第二轮带上 hostReopenAttempted=true：本轮已重放连接，openSession 内部不再重复请求宿主。
    expect(openSession.mock.calls[1]).toEqual([false, false, false, true]);
  });

  it("keeps later ladder attempts on the plain open path", async () => {
    const { reconnect, terminalState, requestHostReopenConnection, openSession } = mountWith(async () => {
      terminalState.value = "error";
    });
    // 消耗第一级（各自场景见上），再调度第二级。
    reconnect.scheduleSessionReconnect();
    await vi.advanceTimersByTimeAsync(TERMINAL_RECONNECT_DELAYS[0]);
    requestHostReopenConnection.mockClear();
    openSession.mockClear();

    reconnect.scheduleSessionReconnect();
    await vi.advanceTimersByTimeAsync(TERMINAL_RECONNECT_DELAYS[1]);

    expect(openSession).toHaveBeenCalledTimes(1);
    expect(openSession).toHaveBeenCalledWith();
    expect(requestHostReopenConnection).not.toHaveBeenCalled();
  });
});
