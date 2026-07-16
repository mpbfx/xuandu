import { describe, expect, it } from "vitest";
import {
  activeSpeaker,
  displayShortcut,
  rateMultiplier,
  shortcutFromKeyEvent,
  statusLabel,
} from "./format";

describe("settings presentation", () => {
  it("maps provider rate values to the visible multiplier", () => {
    expect(rateMultiplier(-50)).toBe("0.5×");
    expect(rateMultiplier(0)).toBe("1.0×");
    expect(rateMultiplier(100)).toBe("2.0×");
  });

  it("prefers a custom speaker without exposing secrets", () => {
    expect(activeSpeaker({ speakerId: "preset", customSpeakerId: " own-speaker " })).toBe(
      "own-speaker",
    );
  });

  it("uses clear Chinese status labels", () => {
    expect(statusLabel("armed")).toBe("等待选中文字");
  });

  it("records a Tauri-compatible shortcut without making the user type syntax", () => {
    expect(
      shortcutFromKeyEvent({
        key: "r",
        code: "KeyR",
        metaKey: true,
        ctrlKey: false,
        altKey: true,
        shiftKey: false,
      }),
    ).toBe("Command+Option+R");
    expect(
      shortcutFromKeyEvent({
        key: "r",
        code: "KeyR",
        metaKey: false,
        ctrlKey: false,
        altKey: false,
        shiftKey: false,
      }),
    ).toBeNull();
  });

  it("keeps the stored shortcut readable in the macOS control", () => {
    expect(displayShortcut("Command+Option+R")).toMatch(/R$/);
  });
});
