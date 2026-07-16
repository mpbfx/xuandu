export const RATE_MIN = -50;
export const RATE_MAX = 100;

export function rateMultiplier(rate: number): string {
  return `${((rate + 100) / 100).toFixed(1)}×`;
}

export function activeSpeaker(settings: {
  speakerId: string;
  customSpeakerId?: string | null;
}): string {
  return settings.customSpeakerId?.trim() || settings.speakerId;
}

export function statusLabel(mode: string): string {
  const labels: Record<string, string> = {
    off: "朗读已关闭",
    armed: "等待选中文字",
    playing: "正在朗读",
    error: "需要处理",
  };
  return labels[mode] ?? "准备就绪";
}

export function genderLabel(gender: string): string {
  const labels: Record<string, string> = {
    female: "女声",
    male: "男声",
    child: "童声",
  };
  return labels[gender] ?? gender;
}

export function displayShortcut(shortcut: string): string {
  const isMac = typeof navigator !== "undefined" && /Mac/i.test(navigator.platform);
  if (!isMac) return shortcut;

  const symbols: Record<string, string> = {
    Command: "⌘",
    Option: "⌥",
    Ctrl: "⌃",
    Shift: "⇧",
  };
  return shortcut
    .split("+")
    .map((part) => symbols[part] ?? part)
    .join("");
}

type ShortcutKeyEvent = Pick<
  KeyboardEvent,
  "key" | "code" | "metaKey" | "ctrlKey" | "altKey" | "shiftKey"
>;

export function shortcutFromKeyEvent(event: ShortcutKeyEvent): string | null {
  const ignoredKeys = new Set(["Meta", "Control", "Alt", "Shift", "CapsLock"]);
  if (ignoredKeys.has(event.key)) return null;

  const isMac = typeof navigator !== "undefined" && /Mac/i.test(navigator.platform);
  const modifiers = [
    event.metaKey ? "Command" : null,
    event.ctrlKey ? "Ctrl" : null,
    event.altKey ? (isMac || event.metaKey ? "Option" : "Alt") : null,
    event.shiftKey ? "Shift" : null,
  ].filter((value): value is string => Boolean(value));
  if (modifiers.length === 0) return null;

  const namedKeys: Record<string, string> = {
    " ": "Space",
    Esc: "Escape",
    Del: "Delete",
    ArrowUp: "Up",
    ArrowDown: "Down",
    ArrowLeft: "Left",
    ArrowRight: "Right",
  };
  const codeKey = event.code.match(/^Key([A-Z])$/)?.[1] ?? event.code.match(/^Digit([0-9])$/)?.[1];
  const key = codeKey ?? namedKeys[event.key] ?? (event.key.length === 1 ? event.key.toUpperCase() : event.key);
  if (["Dead", "Unidentified", "Process"].includes(key) || key.includes("+")) return null;
  return [...modifiers, key].join("+");
}
