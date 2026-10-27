<script setup lang="ts">
/** SFTP 树行类型图标：实心双色几何，替代「彩色小方块 + 线性图标」的叠层
 *  画法。13–14px 下实心轮廓比描边图标更清晰，也避免每行都出现一个彩色底块。
 *  颜色由类型 class 决定，跟随宿主主题的 tone token。 */
export type SftpTreeIconKind = "directory" | "directory-open" | "file" | "symlink" | "other";

defineProps<{ kind: SftpTreeIconKind }>();
</script>

<template>
  <svg
    class="sftp-tree-kind-icon"
    :class="`is-${kind}`"
    viewBox="0 0 24 24"
    width="14"
    height="14"
    aria-hidden="true"
    focusable="false"
  >
    <template v-if="kind === 'directory'">
      <path d="M3 7.6a2.2 2.2 0 0 1 2.2-2.2h4l1.9 2.1h7.7A2.2 2.2 0 0 1 21 9.7v1.5H3Z" fill="currentColor" opacity=".45" />
      <path d="M3 10.2h18v7.4a2.2 2.2 0 0 1-2.2 2.2H5.2A2.2 2.2 0 0 1 3 17.6Z" fill="currentColor" />
    </template>
    <template v-else-if="kind === 'directory-open'">
      <path d="M3 7.6a2.2 2.2 0 0 1 2.2-2.2h4l1.9 2.1h7.7A2.2 2.2 0 0 1 21 9.7v3.5H3Z" fill="currentColor" opacity=".45" />
      <path d="M3 10.6h18l-2.4 7.1a2.2 2.2 0 0 1-2.1 1.5H5.3a2.2 2.2 0 0 1-2.1-2.9Z" fill="currentColor" />
    </template>
    <template v-else>
      <path d="M5.6 3.6h7.1L18 8.9v11a2 2 0 0 1-2 2H5.6a2 2 0 0 1-2-2V5.6a2 2 0 0 1 2-2Z" fill="currentColor" />
      <path d="M12.7 3.6 18 8.9h-5.3Z" fill="#fff" fill-opacity=".28" />
      <template v-if="kind === 'symlink'">
        <path d="M12.4 17.4 18 11.8M14.6 11.8H18v3.4" fill="none" stroke="#fff" stroke-opacity=".9" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" />
      </template>
      <template v-else>
        <path d="M7.4 12.5h5.4M7.4 15.5h4.2" fill="none" stroke="#fff" stroke-opacity=".45" stroke-width="1.3" stroke-linecap="round" />
      </template>
    </template>
  </svg>
</template>

<style scoped>
.sftp-tree-kind-icon {
  box-sizing: border-box;
  width: 18px;
  height: 18px;
  flex: 0 0 18px;
  padding: 2px;
}
.is-directory,
.is-directory-open { color: var(--tone-amber); }
.is-file { color: var(--tone-blue); }
.is-symlink { color: var(--tone-cyan); }
.is-other { color: var(--muted-foreground); }
</style>
