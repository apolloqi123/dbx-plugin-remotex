<script setup lang="ts">
// 应用根（T32）：仅创建工作台实例（setup 内注册 mount/unmount 生命周期）并把
// 其交给外壳组件；/main 壳模板与模板绑定全部收口在 components/WorkbenchShell.vue。
import WorkbenchShell from "./components/WorkbenchShell.vue";
import { useWorkbench } from "./workbench/useWorkbench";

const wb = useWorkbench();
</script>

<template>
  <WorkbenchShell :wb="wb" />
</template>
