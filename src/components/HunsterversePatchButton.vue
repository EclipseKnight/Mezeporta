<!--
  Hunsterverse (1.5.4-hv3): the "Patch: Stable v1.30" button, placed by each launcher
  layout: classic after the footer's status text ("Online Players"), PS4 right of the
  Settings button. Opens the client patch panel of HunsterversePanel.vue.
-->
<script setup>
import { hvPatchButton } from "./hvPatchButton";

defineProps({
  variant: { type: String, default: "classic" },
});
</script>

<template>
  <button
    v-if="hvPatchButton.visible"
    type="button"
    class="hv-chip"
    :class="variant === 'ps4' ? 'hv-chip-ps4' : 'hv-chip-classic'"
    :title="hvPatchButton.title"
    @click="hvPatchButton.open()"
  >
    {{ hvPatchButton.text }}
  </button>
</template>

<style scoped>
.hv-chip {
  padding: 0 10px;
  white-space: nowrap;
  background: rgba(30, 22, 14, 0.85);
  color: #f3ead7;
  border: 1px solid #b8955a;
  border-radius: 4px;
  cursor: pointer;
  flex: 0 0 auto;
}
.hv-chip:hover {
  background: rgba(74, 58, 38, 0.95);
}
.hv-chip-classic {
  height: 24px;
  font-size: 12px;
}
/* Same row and height as the PS4 Settings button (left 157px, width 285px). */
.hv-chip-ps4 {
  position: absolute;
  left: 454px;
  top: 43px;
  height: 32px;
  font-size: 14px;
}
</style>
