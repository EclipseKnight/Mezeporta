// Hunsterverse (1.5.4-hv6): "Debug the next game start" (Linux; src-tauri/src/
// hv_crash.rs), shared by the crash message (HunsterversePanel.vue) and Settings,
// Advanced, Problem report (SettingsList.vue), so both show the same state.
import { reactive } from "vue";
import { invoke } from "@tauri-apps/api";

export const hvDebugLaunch = reactive({ on: false });

export async function setHvDebugLaunch(on) {
  try {
    hvDebugLaunch.on = await invoke("hv_set_debug_launch", { on });
  } catch (_error) {
    hvDebugLaunch.on = false;
  }
  return hvDebugLaunch.on;
}

export async function loadHvDebugLaunch() {
  try {
    hvDebugLaunch.on = await invoke("hv_debug_launch_set");
  } catch (_error) {
    hvDebugLaunch.on = false;
  }
}
