<script setup lang="ts">
// 容器日志页外壳（Task 7）：真实空态，实时流/跟随/清空由 Wave 2 的
// useDockerLogStream 填入本组件内部；props 契约先固定，后续任务不改。
// 尚未建立日志流，因此只显示中性的“尚未加载”，不伪装成“暂无日志输出”。
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
  <div class="container-tab-shell" data-tab-panel="logs">
    <p class="container-tab-empty">日志尚未加载</p>
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
