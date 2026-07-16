export type ReadingMode = "off" | "armed" | "playing" | "error";
export type NoticeKind = "info" | "success" | "warning" | "error";

export interface AppNotice {
  kind: NoticeKind;
  code?: string | null;
  message: string;
}

export interface AppStatus {
  mode: ReadingMode;
  hasApiKey: boolean;
  accessibilityTrusted: boolean;
  notice?: AppNotice | null;
}

export interface Settings {
  speakerId: string;
  customSpeakerId?: string | null;
  speechRate: number;
  loudnessRate: number;
  pitch: number;
  voiceInstruction?: string | null;
  shortcut: string;
  favoriteSpeakerIds: string[];
  launchAtLogin: boolean;
}

export interface SettingsPatch {
  speakerId?: string;
  customSpeakerId?: string;
  speechRate?: number;
  loudnessRate?: number;
  pitch?: number;
  voiceInstruction?: string;
  shortcut?: string;
  favoriteSpeakerIds?: string[];
  launchAtLogin?: boolean;
}

export interface VoiceCatalogEntry {
  speakerId: string;
  name: string;
  gender: "female" | "male" | "child" | string;
  category: string;
  tags: string[];
  model: string;
  resourceId: string;
  recommended: boolean;
}

export interface VoiceCatalog {
  catalogVersion: string;
  model: string;
  language: string;
  voices: VoiceCatalogEntry[];
}

export interface ShortcutValidation {
  shortcut: string;
}

export interface ShortcutError {
  code:
    | "invalid_combination"
    | "reserved_combination"
    | "already_registered"
    | "registration_failed"
    | "persistence_failed";
  message: string;
}
