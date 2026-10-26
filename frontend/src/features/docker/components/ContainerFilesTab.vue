<script setup lang="ts">
// 容器文件页外壳（Task 7）：真实空态，目录浏览/上传下载由 Wave 2 的
// useContainerFiles/useContainerFileTransfer 填入本组件内部；props 契约先固定。
// 尚未发起任何目录请求，因此只显示中性的“尚未加载”，不伪装成“空文件夹”。
import type { DockerContainer } from "../lib/dockerPanelModel";

defineProps<{
  t: (key: string, values?: Record<string, string | number>) => string;
  container: DockerContainer;
  sessionId?: string;
  connectionId?: string;
  canWrite?: boolean;
}>();
</script>

<template>
  <div class="container-tab-shell" data-tab-panel="files">
    <p class="container-tab-empty">文件列表尚未加载</p>
  </div>
</template>

<style scoped>
.container-tab-shell {
  display: flex;
  flex: 1;
  min-height: 0;
  overflow: auto;
}

.container-tab-empty {
  margin: auto;
  padding: 12px;
  font-size: 12px;
  opacity: 0.65;
}
</style>
