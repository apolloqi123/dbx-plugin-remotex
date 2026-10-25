import { computed, ref, watch, type Ref } from "vue";
import type { Terminal } from "@xterm/xterm";
import { syncWebglRenderer, type WebglRecoveryOptions, type WebglRendererLike } from "../../lib/appearance/terminalWebgl";

/**
 * 背景图（对标 NyaTerm；MVP 简化）：图源权威态在 sidecar
 * （local/wallpaper/get|set|clear，桌面端落盘 <plugin_data_dir>/wallpaper，
 * ≤8MiB png/jpeg/webp）；web/docker 形态 set 失败（sidecar 存储不在本机）或
 * 旧 sidecar 无此方法时降级为仅本次会话内存态，UI 有说明且不持久化。
 * 开关/透明度经 preferences allowlist 键持久化。
 */
export function useWallpaper(options: {
  webglEnabled: Ref<boolean>;
  getTerminal: () => Terminal | undefined;
  getWebglRenderer: () => WebglRendererLike | null;
  setWebglRenderer: (renderer: WebglRendererLike | null) => void;
  createWebglAddon: () => WebglRendererLike;
  webglRecoveryOptions: () => WebglRecoveryOptions<WebglRendererLike>;
  persistPrefs: (patch: Record<string, unknown>) => void;
}) {
  const { webglEnabled, getTerminal, getWebglRenderer, setWebglRenderer, createWebglAddon, webglRecoveryOptions, persistPrefs } = options;

  const wallpaperEnabled = ref(false);
  // 百分比 10..=90（sidecar 侧钳制同口径），渲染时 /100。
  const wallpaperOpacity = ref(45);
  const wallpaperDataUrl = ref("");
  const wallpaperSessionOnly = ref(false);
  const wallpaperActive = computed(() => wallpaperEnabled.value && !!wallpaperDataUrl.value);
  // 背景图开启时强制回退 DOM 渲染器：WebGL 画布不透明，盖死背景层；关闭
  // 背景图后按用户 WebGL 开关恢复。复用现有 syncWebglRenderer 切换点。
  const rendererWebglEffective = computed(() => webglEnabled.value && !wallpaperActive.value);

  async function loadWallpaperImage() {
    try {
      const result = await window.dbxPlugin.invoke<{ dataUrl?: string }>("local/wallpaper/get", {});
      if (typeof result.dataUrl === "string") wallpaperDataUrl.value = result.dataUrl;
    } catch {
      // 旧 sidecar：背景图缺席，保持内存态。
    }
  }

  async function setWallpaperImage(image: { base64: string; mime: string }) {
    try {
      const result = await window.dbxPlugin.invoke<{ dataUrl?: string }>("local/wallpaper/set", { imageBase64: image.base64 });
      if (typeof result.dataUrl === "string") wallpaperDataUrl.value = result.dataUrl;
      wallpaperSessionOnly.value = false;
    } catch {
      // web/docker 形态或旧 sidecar：仅本次会话内存态（mime 来自上传文件读取）。
      wallpaperDataUrl.value = `data:${image.mime || "image/png"};base64,${image.base64}`;
      wallpaperSessionOnly.value = true;
    }
  }

  async function clearWallpaperImage() {
    wallpaperSessionOnly.value = false;
    wallpaperDataUrl.value = "";
    try {
      await window.dbxPlugin.invoke("local/wallpaper/clear", {});
    } catch {
      // 同 set：会话内存态已清，落盘态留待桌面形态下次清除。
    }
  }

  function updateWallpaperEnabled(enabled: boolean) {
    wallpaperEnabled.value = enabled;
    void persistPrefs({ wallpaper_enabled: enabled });
  }

  function updateWallpaperOpacity(percent: number) {
    wallpaperOpacity.value = Math.min(90, Math.max(10, Math.round(percent)));
    void persistPrefs({ wallpaper_opacity: wallpaperOpacity.value });
  }

  // 注册点必须在 wallpaperActive 定义之后：watch 首次求值会沿依赖链触达它。
  watch(rendererWebglEffective, (next) => {
    const terminal = getTerminal();
    if (!terminal) return;
    setWebglRenderer(syncWebglRenderer(terminal, next, getWebglRenderer(), createWebglAddon, webglRecoveryOptions()));
  });

  return {
    wallpaperEnabled,
    wallpaperOpacity,
    wallpaperDataUrl,
    wallpaperSessionOnly,
    wallpaperActive,
    rendererWebglEffective,
    loadWallpaperImage,
    setWallpaperImage,
    clearWallpaperImage,
    updateWallpaperEnabled,
    updateWallpaperOpacity,
  };
}
