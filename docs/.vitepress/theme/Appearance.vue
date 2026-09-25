<script setup>
import { onMounted, onUnmounted, ref } from 'vue'
import { useData } from 'vitepress'
const { isDark } = useData()
const selection = ref('auto')
let media
function apply() {
  isDark.value = selection.value === 'dark' || (selection.value === 'auto' && media.matches)
  try { localStorage.setItem('enderpin-docs-theme', selection.value) } catch {}
}
onMounted(() => {
  media = window.matchMedia('(prefers-color-scheme: dark)')
  try {
    const saved = localStorage.getItem('enderpin-docs-theme')
    if (['auto', 'light', 'dark'].includes(saved)) selection.value = saved
  } catch {}
  apply()
  media.addEventListener('change', apply)
})
onUnmounted(() => media?.removeEventListener('change', apply))
</script>

<template>
  <details class="appearance-settings">
    <summary>表示設定</summary>
    <label for="docs-theme">テーマ</label>
    <select id="docs-theme" v-model="selection" @change="apply">
      <option value="auto">システムに合わせる</option>
      <option value="light">ライト</option>
      <option value="dark">ダーク</option>
    </select>
  </details>
</template>
