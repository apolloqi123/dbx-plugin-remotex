// @vitest-environment happy-dom
// RDP 文件传输入口只解释剪贴板用法；双栏 SFTP 窗口是 VNC 的入口。
// 提示文案随宿主平台切换：只有 macOS 宿主能用 Finder 文件剪贴板。
import { describe, expect, it, vi } from "vitest";
import { mount } from "@vue/test-utils";
import RdpToolbarControls from "./RdpToolbarControls.vue";

const t = (key: string) => key;

const stubs = {
  Popover: { template: "<div><slot /></div>" },
  PopoverAnchor: { template: "<span><slot /></span>" },
  PopoverContent: { template: "<div><slot /></div>" },
};

function mountControls(
  applePlatform = true,
  nativeFileClipboard = applePlatform,
  serverFiles = { supported: true, known: true },
  clipboardEnabled = true,
) {
  const actions = {
    toggleRdpResolutionMenu: vi.fn(),
    applyRdpResolution: vi.fn(),
    rdpResolutionLabel: () => "1280 × 720",
    isRdpResolutionActive: () => false,
    rdpWindowProportionPreset: () => ({ width: 1280, height: 720 }),
    enableRdpAutoFit: vi.fn(),
    applyRdpCustomResolution: vi.fn(),
    toggleRdpPasteMenu: vi.fn(),
    toggleRdpFileHintMenu: vi.fn(),
    openRemoteFileTransfer: vi.fn(),
    rdpWriteRemoteClipboard: vi.fn(),
    rdpTypeAsKeystrokes: vi.fn(),
    closeRdpSession: vi.fn(),
  };
  const wrapper = mount(RdpToolbarControls, {
    props: {
      t,
      applePlatform,
      nativeFileClipboard,
      serverFileClipboard: serverFiles.supported,
      serverFileClipboardKnown: serverFiles.known,
      clipboardEnabled,
      resolutionPresets: [{ width: 1280, height: 720 }],
      rdpResizing: false,
      rdpAutoFit: false,
      rdpDesktopSize: null,
      rdpTypingBusy: false,
      rdpTypingSent: 0,
      rdpTypingTotal: 0,
      rdpScaleMode: "fit" as const,
      rdpResolutionOpen: false,
      rdpCustomWidth: "",
      rdpCustomHeight: "",
      rdpPasteOpen: false,
      rdpPasteText: "",
      rdpFileHintOpen: false,
      actions,
    },
    global: { stubs },
  });
  return { wrapper, actions };
}

function hintRows(wrapper: ReturnType<typeof mountControls>["wrapper"]) {
  return wrapper.findAll(".rdp-file-hint-row").map((row) => row.text());
}

describe("RdpToolbarControls file transfer entry", () => {
  it("shows clipboard instructions instead of opening the SFTP window", async () => {
    const { wrapper, actions } = mountControls();
    const trigger = wrapper.find(".rdp-file-hint-trigger");
    expect(trigger.attributes("title")).toBe("rdp.clipboardFiles.hintTitle");
    expect(trigger.attributes("aria-expanded")).toBe("false");

    expect(wrapper.text()).toContain("rdp.clipboardFiles.hintIntro");
    await trigger.trigger("click");
    expect(actions.toggleRdpFileHintMenu).toHaveBeenCalledTimes(1);
    // 文件按钮本身不再触发任何窗口/连接动作。
    for (const action of Object.values(actions)) {
      if (action !== actions.toggleRdpFileHintMenu && vi.isMockFunction(action)) {
        expect(action).not.toHaveBeenCalled();
      }
    }
  });

  it("points macOS hosts at Finder copy/paste with ⌘", () => {
    const { wrapper } = mountControls(true);
    expect(hintRows(wrapper)).toEqual([
      "rdp.clipboardFiles.hintRemoteToHost",
      "rdp.clipboardFiles.hintHostToRemote",
    ]);
    expect(wrapper.find(".rdp-file-hint-note").text()).toBe("rdp.clipboardFiles.hintDrag");
  });

  it("points Windows/Linux hosts at native Ctrl+C/V when the sidecar supports it", () => {
    const { wrapper } = mountControls(false, true);
    expect(hintRows(wrapper)).toEqual([
      "rdp.clipboardFiles.hintRemoteToHostNative",
      "rdp.clipboardFiles.hintHostToRemoteNative",
    ]);
    expect(wrapper.find(".rdp-file-hint-note").text()).toBe("rdp.clipboardFiles.hintDrag");
  });

  it("falls back to the desktop card when the host has no file clipboard", () => {
    const { wrapper } = mountControls(false, false);
    expect(hintRows(wrapper)).toEqual([
      "rdp.clipboardFiles.hintRemoteToHostOther",
      "rdp.clipboardFiles.hintHostToRemoteOther",
    ]);
    // 非 macOS 无原生文件剪贴板时拖拽说明已并入宿主 → 远端行。
    expect(wrapper.find(".rdp-file-hint-note").exists()).toBe(false);
  });

  it("routes Linux/GNOME servers without file streams to SSH/SFTP", async () => {
    const { wrapper, actions } = mountControls(true, true, { supported: false, known: true });
    expect(hintRows(wrapper)).toEqual([]);
    expect(wrapper.text()).toContain("rdp.clipboardFiles.serverUnsupported");
    const ssh = wrapper.find(".rdp-file-hint-ssh");
    expect(ssh.text()).toBe("rdp.clipboardFiles.openSshTransfer");
    await ssh.trigger("click");
    expect(actions.openRemoteFileTransfer).toHaveBeenCalledTimes(1);
  });

  it("keeps the Ctrl+C/V guidance while the server capability is unknown", () => {
    const { wrapper } = mountControls(false, true, { supported: false, known: false });
    expect(hintRows(wrapper)).toEqual([
      "rdp.clipboardFiles.hintRemoteToHostNative",
      "rdp.clipboardFiles.hintHostToRemoteNative",
    ]);
  });

  it("offers SSH/SFTP when clipboard sharing is disabled", async () => {
    const { wrapper, actions } = mountControls(true, true, { supported: true, known: true }, false);
    expect(hintRows(wrapper)).toEqual([]);
    expect(wrapper.text()).toContain("rdp.clipboardFiles.disabled");
    await wrapper.find(".rdp-file-hint-ssh").trigger("click");
    expect(actions.openRemoteFileTransfer).toHaveBeenCalledTimes(1);
  });

  it("keeps the disconnect action on its own button", async () => {
    const { wrapper, actions } = mountControls();
    await wrapper.find('button[title="rdp.disconnect"]').trigger("click");
    expect(actions.closeRdpSession).toHaveBeenCalledTimes(1);
  });
});
