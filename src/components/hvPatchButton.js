// Hunsterverse (1.5.4-hv3): state of the Patch button. HunsterversePanel.vue fills
// it (text, whether to show it, what a click does); HunsterversePatchButton.vue
// draws it inside each launcher layout, so it scales with the launcher and has
// its own place instead of floating over the footer's CAPCOM logo.
import { reactive } from "vue";

export const hvPatchButton = reactive({
  visible: false,
  text: "",
  title: "",
  open: () => {},
});
