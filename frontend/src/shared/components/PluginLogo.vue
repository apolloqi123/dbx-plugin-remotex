<script setup lang="ts">
// 直接引用插件真实 logo（单一来源：assets/plugin.svg，宿主插件中心同款图形）。
// 标识为青→翡翠渐变的盾形主体 + 深色终端提示符，不再由 CSS 重染描边；
// 连接卡徽标/端点只负责尺寸与留白。
import pluginLogo from "../../../../assets/plugin.svg?raw";
</script>

<template>
  <span class="plugin-logo" aria-hidden="true" v-html="pluginLogo" />
</template>
