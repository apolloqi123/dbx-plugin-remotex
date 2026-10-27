<script setup lang="ts">
// 直接引用插件真实 logo（单一来源：assets/plugin.svg，宿主插件中心同款图形）。
// 标识只保留紫色终端提示符 `>_`（透明底、无盾形/无瓦片），圆形背景由宿主
// 提供；连接卡徽标/端点只负责尺寸与留白，不再由 CSS 重染。
import pluginLogo from "../../../../assets/plugin.svg?raw";
</script>

<template>
  <span class="plugin-logo" aria-hidden="true" v-html="pluginLogo" />
</template>
