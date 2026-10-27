<script setup lang="ts">
// 直接引用插件真实 logo（单一来源：assets/plugin.svg，宿主插件中心同款图形）。
// 新标识自带深色圆角瓦片与品牌色（青色提示符 + 翡翠盾徽），不再由 CSS 重染
// 描边；连接卡徽标/端点只负责尺寸与留白。
import pluginLogo from "../../../../assets/plugin.svg?raw";
</script>

<template>
  <span class="plugin-logo" aria-hidden="true" v-html="pluginLogo" />
</template>
