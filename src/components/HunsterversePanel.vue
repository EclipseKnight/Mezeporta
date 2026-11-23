<!--
  Hunsterverse (1.5.4-hv2), shown in both launcher styles (mounted once in Main.vue):

  - Launcher update: at start the launcher asks the fork's latest GitHub release
    (hv_check_launcher_update, src-tauri/src/hv_update.rs). When it is newer, a bar
    offers "Update and restart": the backend downloads the binary, checks its
    signature, swaps it in and restarts. "Later" hides the bar until the next start.

  - Client patch: on the character page of a Hunsterverse login, a button shows the
    installed patch (drawn by each launcher layout: HunsterversePatchButton.vue);
    it opens a panel with what is installed, what the server gives this account,
    and the choice Stable / each published pre-release. The choice is
    the account setting the website's Account page edits (same API, src-tauri/src/
    hv_patch.rs); after a change the launcher logs in again, so the normal patch
    dialog downloads the chosen patch (or puts the stable one back).
-->
<script setup>
import { ref, computed, watch, watchEffect, onMounted } from "vue";
import { invoke } from "@tauri-apps/api";
import { open } from "@tauri-apps/api/shell";
import { storeMut, doLogin } from "../store";
import { CHARACTERS_PAGE } from "../common";
import { hvPatchButton } from "./hvPatchButton";

// --- launcher update -------------------------------------------------------
const update = ref(null); // UpdateInfo when one is available
const updateHidden = ref(false);
const updateBusy = ref(false);
const updateError = ref("");

async function checkUpdate() {
  try {
    const info = await invoke("hv_check_launcher_update");
    if (info && info.available) update.value = info;
  } catch (_e) {
    // Offline, GitHub down, or no release yet: say nothing.
  }
}

async function installUpdate() {
  updateBusy.value = true;
  updateError.value = "";
  try {
    // Only returns on failure: on success the launcher restarts into the new version.
    await invoke("hv_install_launcher_update");
  } catch (e) {
    updateError.value = String(e || "The update failed.");
  } finally {
    updateBusy.value = false;
  }
}

const updateNotes = computed(() => {
  const text = String(update.value?.notes || "").trim();
  return text.split("\n").map((l) => l.trim()).filter(Boolean).slice(0, 4);
});

onMounted(() => {
  window.setTimeout(checkUpdate, 1500);
});

// --- client patch ----------------------------------------------------------
const status = ref(null);
const panelOpen = ref(false);
const loading = ref(false);
const saving = ref(false);
const message = ref("");
const errorText = ref("");
const confirmText = ref("");
const pendingChannel = ref("");

function label(channel) {
  if (!channel || channel === "stable") return "Stable";
  return "Pre-release " + channel.replace(/^pre:/, "");
}

function withVersion(channel, version) {
  return version ? `${label(channel)} ${version}` : label(channel);
}

const api = computed(() => status.value?.api || null);
const slots = computed(() => (Array.isArray(api.value?.slots) ? api.value.slots : []));
const stableVersion = computed(() => api.value?.currentPatch || "");

const installedText = computed(() => {
  const s = status.value;
  if (!s || !s.installed) return "Not installed by this launcher yet";
  return withVersion(s.installed.channel, s.installed.version);
});

const servedText = computed(() => {
  const s = status.value;
  if (!s) return "";
  return withVersion(s.servedChannel, s.servedVersion);
});

const chipText = computed(() => {
  const s = status.value;
  if (!s || !s.supported) return "";
  if (s.updatePending) return "Patch: update waiting";
  if (s.installedUpToDate && s.installed) return "Patch: " + withVersion(s.installed.channel, s.installed.version);
  return "Patch: " + withVersion(s.servedChannel, s.servedVersion);
});

// The account chose a slot that is not out (or has no rights for it): the server gives stable.
const choiceNote = computed(() => {
  const s = status.value;
  if (!s || !s.accountChannel || s.accountChannel === s.servedChannel) return "";
  if (s.accountChannel !== "stable" && !slots.value.some((x) => "pre:" + x.slot === s.accountChannel)) {
    return `Your account chose ${label(s.accountChannel)}, which is not out right now, so the server gives Stable.`;
  }
  return `Your account chose ${label(s.accountChannel)}; log in again to get it.`;
});

const showChip = computed(
  () => storeMut.page === CHARACTERS_PAGE && status.value && status.value.supported
);

async function loadStatus() {
  loading.value = true;
  errorText.value = "";
  try {
    status.value = await invoke("hv_patch_status");
    if (status.value?.error) errorText.value = status.value.error;
  } catch (e) {
    status.value = null;
    errorText.value = String(e || "");
  } finally {
    loading.value = false;
  }
}

watch(
  () => storeMut.page,
  (page) => {
    if (page === CHARACTERS_PAGE) loadStatus();
    else panelOpen.value = false;
  }
);

function openPanel() {
  message.value = "";
  confirmText.value = "";
  panelOpen.value = true;
  loadStatus();
}

// Hunsterverse (1.5.4-hv3): the button itself is part of each launcher layout.
hvPatchButton.open = openPanel;
watchEffect(() => {
  hvPatchButton.visible = Boolean(showChip.value) && !panelOpen.value;
  hvPatchButton.text = chipText.value;
  hvPatchButton.title = servedText.value;
});

function closePanel() {
  if (saving.value) return;
  panelOpen.value = false;
  confirmText.value = "";
}

async function choose(channel, confirm = false) {
  if (saving.value) return;
  saving.value = true;
  message.value = "Saving.";
  errorText.value = "";
  try {
    const res = await invoke("hv_set_patch_channel", { channel, confirm });
    if (res.confirmationRequired) {
      pendingChannel.value = channel;
      confirmText.value = res.message || "Pre-release patches play only on the Test land.";
      message.value = "";
      return;
    }
    confirmText.value = "";
    message.value = (res.message || "Saved.") + " Logging in again to apply it.";
    // Log in again: the server now gives the chosen patch, and the launcher's
    // normal "Server Updates" dialog installs it (or puts Stable back).
    saving.value = false;
    panelOpen.value = false;
    await doLogin();
    // No patch to install (already in place): the page stays, so refresh here.
    if (storeMut.page === CHARACTERS_PAGE) await loadStatus();
  } catch (e) {
    errorText.value = String(e || "Could not save. Try again in a moment.");
    message.value = "";
  } finally {
    saving.value = false;
  }
}

// The full pre-release notes: the Client patch tab of the website's Account page
// (our own site, so no "leaving the launcher" prompt).
const NOTES_URL = "https://frontier.hunsterverse.net/account.html#client-patch";
function openNotes() {
  open(NOTES_URL).catch((e) => {
    errorText.value = "Could not open the browser: " + String(e || "");
  });
}

function isChosen(channel) {
  return (status.value?.accountChannel || "stable") === channel;
}
</script>

<template>
  <div class="hv-layer">
    <div v-if="update && !updateHidden" class="hv-update-bar">
      <div class="hv-update-text">
        <strong>Launcher update {{ update.latestVersion }}</strong>
        <span> (you have {{ update.currentVersion }})</span>
        <ul v-if="updateNotes.length" class="hv-notes">
          <li v-for="(n, i) in updateNotes" :key="i">{{ n }}</li>
        </ul>
        <div v-if="updateError" class="hv-error">{{ updateError }}</div>
      </div>
      <div class="hv-actions">
        <button class="hv-btn hv-primary" :disabled="updateBusy" @click="installUpdate">
          {{ updateBusy ? "Downloading..." : "Update and restart" }}
        </button>
        <button class="hv-btn" :disabled="updateBusy" @click="updateHidden = true">Later</button>
      </div>
    </div>

    <div v-if="panelOpen" class="hv-backdrop" @click.self="closePanel">
      <div class="hv-panel" role="dialog" aria-label="Client patch">
        <h2>Client patch</h2>
        <p v-if="loading && !status" class="hv-muted">Loading.</p>
        <template v-if="status">
          <table class="hv-facts">
            <tr>
              <th>Installed</th>
              <td>
                {{ installedText }}
                <span v-if="status.installedUpToDate" class="hv-ok">up to date</span>
                <span v-else-if="status.updatePending" class="hv-warn">update waiting</span>
              </td>
            </tr>
            <tr>
              <th>Server gives you</th>
              <td>{{ servedText }}</td>
            </tr>
          </table>
          <p v-if="choiceNote" class="hv-muted">{{ choiceNote }}</p>

          <template v-if="api">
            <h3>Choose</h3>
            <div class="hv-choices">
              <button
                class="hv-btn"
                :class="{ 'hv-on': isChosen('stable') }"
                :disabled="saving"
                @click="choose('stable')"
              >
                Stable{{ stableVersion ? " " + stableVersion : "" }}
              </button>
              <button
                v-for="s in slots"
                :key="s.slot"
                class="hv-btn"
                :class="{ 'hv-on': isChosen('pre:' + s.slot) }"
                :disabled="saving"
                @click="choose('pre:' + s.slot)"
              >
                Pre-release {{ s.slot }}{{ s.version ? " " + s.version : "" }}
              </button>
            </div>
            <p v-if="!slots.length" class="hv-muted">No pre-release is out right now.</p>
            <div v-for="s in slots" :key="'n' + s.slot">
              <template v-if="s.notes && s.notes.length">
                <p class="hv-muted">What {{ s.slot }} tests:</p>
                <ul class="hv-notes">
                  <li v-for="(n, i) in s.notes" :key="i">{{ n }}</li>
                </ul>
              </template>
            </div>
            <p v-if="slots.length">
              <a href="#" class="hv-link" @click.prevent="openNotes()">Full pre-release notes on the website</a>
            </p>
            <p v-if="slots.length" class="hv-muted">
              Pre-release patches play only on the Test land. Switch back to Stable any time to play normally.
            </p>
          </template>

          <div v-if="confirmText" class="hv-confirm">
            <p>{{ confirmText }}</p>
            <div class="hv-actions">
              <button class="hv-btn hv-primary" :disabled="saving" @click="choose(pendingChannel, true)">
                I understand
              </button>
              <button class="hv-btn" :disabled="saving" @click="confirmText = ''">Cancel</button>
            </div>
          </div>
        </template>
        <p v-if="message" class="hv-muted">{{ message }}</p>
        <p v-if="errorText" class="hv-error">{{ errorText }}</p>
        <div class="hv-actions hv-end">
          <button class="hv-btn" :disabled="saving" @click="closePanel">Close</button>
        </div>
      </div>
    </div>
  </div>
</template>

<style scoped>
.hv-layer {
  position: fixed;
  inset: 0;
  pointer-events: none;
  z-index: 900;
  font-family: inherit;
  color: #f3ead7;
}
.hv-update-bar,
.hv-backdrop {
  pointer-events: auto;
}
.hv-update-bar {
  position: absolute;
  top: 8px;
  left: 50%;
  transform: translateX(-50%);
  max-width: min(640px, 92vw);
  display: flex;
  gap: 12px;
  align-items: flex-start;
  padding: 10px 14px;
  background: rgba(30, 22, 14, 0.94);
  border: 1px solid #b8955a;
  border-radius: 6px;
  font-size: 13px;
  box-shadow: 0 4px 16px rgba(0, 0, 0, 0.5);
}
.hv-update-text {
  flex: 1;
}
.hv-backdrop {
  position: absolute;
  inset: 0;
  background: rgba(0, 0, 0, 0.55);
  display: flex;
  align-items: center;
  justify-content: center;
}
.hv-panel {
  width: min(560px, 94vw);
  max-height: 88vh;
  overflow-y: auto;
  padding: 16px 18px;
  background: rgba(30, 22, 14, 0.97);
  border: 1px solid #b8955a;
  border-radius: 8px;
  font-size: 13px;
}
.hv-panel h2 {
  font-size: 17px;
  margin: 0 0 10px;
}
.hv-panel h3 {
  font-size: 14px;
  margin: 12px 0 6px;
}
.hv-facts th {
  text-align: left;
  padding-right: 12px;
  font-weight: 600;
  white-space: nowrap;
}
.hv-choices {
  display: flex;
  flex-wrap: wrap;
  gap: 6px;
}
.hv-btn {
  padding: 5px 10px;
  font-size: 12px;
  background: #4a3a26;
  color: #f3ead7;
  border: 1px solid #b8955a;
  border-radius: 4px;
  cursor: pointer;
  white-space: nowrap;
}
.hv-btn:disabled {
  opacity: 0.5;
  cursor: default;
}
.hv-primary {
  background: #7a5a2a;
}
.hv-on {
  background: #b8955a;
  color: #1e160e;
}
.hv-actions {
  display: flex;
  gap: 6px;
  margin-top: 8px;
}
.hv-end {
  justify-content: flex-end;
}
.hv-link {
  color: #f2c14e;
  text-decoration: underline;
  cursor: pointer;
}
.hv-notes {
  margin: 4px 0 4px 18px;
  list-style: disc;
}
.hv-muted {
  opacity: 0.8;
  margin: 6px 0;
}
.hv-ok {
  color: #9fd59a;
  margin-left: 6px;
}
.hv-warn {
  color: #f0c36a;
  margin-left: 6px;
}
.hv-error {
  color: #ff9a8a;
  margin: 6px 0;
}
.hv-confirm {
  margin-top: 10px;
  padding: 8px;
  border: 1px solid #f0c36a;
  border-radius: 4px;
}
</style>
