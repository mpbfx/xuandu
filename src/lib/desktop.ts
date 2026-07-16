import { invoke } from "@tauri-apps/api/core";
import type {
  AppStatus,
  Settings,
  SettingsPatch,
  ShortcutValidation,
  VoiceCatalog,
} from "../types";

export const desktop = {
  getStatus: () => invoke<AppStatus>("get_status"),
  getSettings: () => invoke<Settings>("get_settings"),
  getVoiceCatalog: () => invoke<VoiceCatalog>("get_voice_catalog"),
  updateSettings: (patch: SettingsPatch) =>
    invoke<Settings>("update_settings", { patch }),
  saveApiKey: (apiKey: string) => invoke<void>("save_api_key", { apiKey }),
  toggleReading: () => invoke<AppStatus>("toggle_reading"),
  previewVoice: (speakerId: string, applyInstruction: boolean) =>
    invoke<void>("preview_voice", { speakerId, applyInstruction }),
  stopPlayback: () => invoke<AppStatus>("stop_playback"),
  requestAccessibility: () => invoke<void>("request_accessibility"),
  clearApiKey: () => invoke<void>("clear_api_key"),
  validateShortcut: (shortcut: string) =>
    invoke<ShortcutValidation>("validate_shortcut", { shortcut }),
  beginShortcutRecording: () => invoke<void>("begin_shortcut_recording"),
  commitShortcutRecording: (shortcut: string) =>
    invoke<Settings>("commit_shortcut_recording", { shortcut }),
  cancelShortcutRecording: () => invoke<void>("cancel_shortcut_recording"),
  exportDiagnostics: () => invoke<string>("export_diagnostics"),
};
