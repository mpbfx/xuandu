import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import {
  Accessibility,
  AudioLines,
  CheckCircle2,
  CircleAlert,
  ChevronDown,
  FileDown,
  Heart,
  KeyRound,
  Keyboard,
  LockKeyhole,
  MessageSquareText,
  Music2,
  Info,
  Play,
  Power,
  RotateCcw,
  Search,
  ShieldCheck,
  Square,
  Star,
  Trash2,
  TriangleAlert,
  Volume2,
  X,
} from "lucide-react";
import { desktop } from "./lib/desktop";
import {
  RATE_MAX,
  RATE_MIN,
  activeSpeaker,
  displayShortcut,
  genderLabel,
  rateMultiplier,
  shortcutFromKeyEvent,
  statusLabel,
} from "./lib/format";
import type {
  AppStatus,
  Settings,
  SettingsPatch,
  ShortcutError,
  NoticeKind,
  VoiceCatalog,
} from "./types";

const DEFAULT_SPEAKER = "zh_female_tianmeitaozi_uranus_bigtts";

const DEFAULT_SETTINGS: Settings = {
  speakerId: DEFAULT_SPEAKER,
  customSpeakerId: null,
  speechRate: 0,
  loudnessRate: 0,
  pitch: 0,
  voiceInstruction: null,
  shortcut: "Command+Option+R",
  favoriteSpeakerIds: [],
  launchAtLogin: false,
};

const EMPTY_CATALOG: VoiceCatalog = {
  catalogVersion: "",
  model: "seed-tts-2.0",
  language: "zh-CN",
  voices: [],
};

const VOICE_INSTRUCTION_PRESETS = [
  { id: "natural", label: "自然", description: "保留音色原本的表达", value: "" },
  { id: "gentle", label: "温和", description: "放松、亲近、轻柔", value: "请用温柔、自然、放松的语气说话。" },
  { id: "broadcast", label: "播报", description: "清晰、沉稳、专业", value: "请用清晰、沉稳、专业的新闻播报语气说话。" },
  { id: "bright", label: "活力", description: "明快、有感染力", value: "请用充满活力、明快且有感染力的语气说话。" },
  { id: "serious", label: "沉稳", description: "低沉、严肃、克制", value: "请用低沉、严肃、克制的语气说话。" },
  { id: "sad", label: "悲伤", description: "缓慢、痛心、清晰", value: "请用悲伤、痛心但保持清晰的语气说话。" },
] as const;

type Tab = "general" | "voices" | "advanced";
type CatalogScope = "recommended" | "favorites" | "all";
type SaveState = "idle" | "saving" | "saved" | "failed";
type UiFeedback = { kind: NoticeKind; message: string };

function isCustomVoice(settings: Settings): boolean {
  return Boolean(settings.customSpeakerId?.trim());
}

function readableError(error: unknown): string {
  if (typeof error === "string") return error;
  if (error instanceof Error) return error.message;
  if (typeof error === "object" && error !== null && "message" in error) {
    return String(error.message);
  }
  return "操作未完成，请检查设置后重试。";
}

export default function App() {
  const [status, setStatus] = useState<AppStatus>({
    mode: "off",
    hasApiKey: false,
    accessibilityTrusted: false,
    notice: null,
  });
  const [settings, setSettings] = useState<Settings>(DEFAULT_SETTINGS);
  const [catalog, setCatalog] = useState<VoiceCatalog>(EMPTY_CATALOG);
  const [tab, setTab] = useState<Tab>("general");
  const [catalogScope, setCatalogScope] = useState<CatalogScope>("recommended");
  const [search, setSearch] = useState("");
  const [gender, setGender] = useState("all");
  const [category, setCategory] = useState("all");
  const [apiKey, setApiKey] = useState("");
  const [recordingShortcut, setRecordingShortcut] = useState(false);
  const [shortcutDraft, setShortcutDraft] = useState<string | null>(null);
  const [shortcutError, setShortcutError] = useState<string | null>(null);
  const [toggleBusy, setToggleBusy] = useState(false);
  const [keyBusy, setKeyBusy] = useState(false);
  const [previewingSpeakerId, setPreviewingSpeakerId] = useState<string | null>(null);
  const [unavailableSpeakerIds, setUnavailableSpeakerIds] = useState<Set<string>>(() => new Set());
  const [saveState, setSaveState] = useState<SaveState>("idle");
  const [feedback, setFeedback] = useState<UiFeedback | null>(null);
  const [dismissedNotice, setDismissedNotice] = useState<string | null>(null);
  const [diagnosticsPath, setDiagnosticsPath] = useState<string | null>(null);

  const saveTimer = useRef<number | undefined>(undefined);
  const pendingPatch = useRef<SettingsPatch>({});
  const requestInFlight = useRef(false);
  const settingsRevision = useRef(0);
  const voiceViewport = useRef<HTMLDivElement>(null);
  const voiceOptionRefs = useRef(new Map<string, HTMLButtonElement>());
  const shortcutRecorder = useRef<HTMLButtonElement>(null);

  function showFeedback(message: string, kind: NoticeKind = "info") {
    setFeedback({ kind, message });
  }

  const flushSettings = useCallback(async () => {
    saveTimer.current = undefined;
    if (requestInFlight.current || Object.keys(pendingPatch.current).length === 0) return;

    const patch = pendingPatch.current;
    const revision = settingsRevision.current;
    pendingPatch.current = {};
    requestInFlight.current = true;
    setSaveState("saving");
    setFeedback(null);

    try {
      const saved = await desktop.updateSettings(patch);
      if (revision === settingsRevision.current) setSettings(saved);
      setSaveState("saved");
    } catch (error) {
      setSaveState("failed");
      showFeedback(readableError(error), "error");
      if (revision === settingsRevision.current) {
        void desktop
          .getSettings()
          .then((saved) => {
            if (revision === settingsRevision.current) setSettings(saved);
          })
          .catch(() => undefined);
      }
    } finally {
      requestInFlight.current = false;
      if (Object.keys(pendingPatch.current).length > 0) await flushSettings();
    }
  }, []);

  const queueSettings = useCallback(
    (patch: SettingsPatch, delay = 300) => {
      settingsRevision.current += 1;
      setSettings((current) => ({ ...current, ...patch }));
      pendingPatch.current = { ...pendingPatch.current, ...patch };
      if (saveTimer.current !== undefined) window.clearTimeout(saveTimer.current);
      saveTimer.current = window.setTimeout(() => void flushSettings(), delay);
    },
    [flushSettings],
  );

  useEffect(() => {
    let active = true;
    void Promise.all([desktop.getStatus(), desktop.getSettings(), desktop.getVoiceCatalog()])
      .then(([nextStatus, nextSettings, nextCatalog]) => {
        if (!active) return;
        setStatus(nextStatus);
        setSettings(nextSettings);
        setCatalog(nextCatalog);
      })
      .catch((error: unknown) => {
        if (active) showFeedback(readableError(error), "error");
      });

    const unlisten = listen<AppStatus>("app-state", (event) => {
      if (active) setStatus(event.payload);
    });
    const refreshOnFocus = () => {
      void desktop.getStatus().then(setStatus).catch(() => undefined);
    };
    window.addEventListener("focus", refreshOnFocus);

    return () => {
      active = false;
      window.removeEventListener("focus", refreshOnFocus);
      void unlisten.then((dispose) => dispose());
    };
  }, []);

  useEffect(
    () => () => {
      if (saveTimer.current !== undefined) window.clearTimeout(saveTimer.current);
    },
    [],
  );

  const cancelShortcutRecording = useCallback(async (message?: string) => {
    try {
      await desktop.cancelShortcutRecording();
    } catch (error) {
      setShortcutError(readableError(error));
    } finally {
      setRecordingShortcut(false);
      setShortcutDraft(null);
      if (message) showFeedback(message);
    }
  }, []);

  useEffect(() => {
    if (!recordingShortcut) return;

    const handleKeyDown = (event: KeyboardEvent) => {
      event.preventDefault();
      event.stopImmediatePropagation();
      if (event.key === "Escape") {
        void cancelShortcutRecording("已取消快捷键录制。");
        return;
      }

      const shortcut = shortcutFromKeyEvent(event);
      if (!shortcut) {
        setShortcutDraft(null);
        setShortcutError("请按住 Command、Ctrl、Option/Alt 或 Shift，再按一个普通按键。");
        return;
      }

      setShortcutDraft(shortcut);
      setShortcutError(null);
      void desktop
        .commitShortcutRecording(shortcut)
        .then((saved) => {
          settingsRevision.current += 1;
          setSettings(saved);
          setRecordingShortcut(false);
          setShortcutDraft(null);
          setSaveState("saved");
          showFeedback("快捷键已更新，现在可以立即使用。", "success");
        })
        .catch((error: ShortcutError | unknown) => {
          const message =
            typeof error === "object" && error !== null && "message" in error
              ? String(error.message)
              : readableError(error);
          setShortcutError(message);
          setRecordingShortcut(false);
          setShortcutDraft(null);
        });
    };
    const handleBlur = () => void cancelShortcutRecording("窗口失去焦点，已恢复原快捷键。");
    window.addEventListener("keydown", handleKeyDown, true);
    window.addEventListener("blur", handleBlur);
    return () => {
      window.removeEventListener("keydown", handleKeyDown, true);
      window.removeEventListener("blur", handleBlur);
    };
  }, [cancelShortcutRecording, recordingShortcut]);

  useEffect(
    () => () => {
      if (recordingShortcut) void desktop.cancelShortcutRecording().catch(() => undefined);
    },
    [recordingShortcut],
  );

  const favorites = useMemo(() => new Set(settings.favoriteSpeakerIds), [settings.favoriteSpeakerIds]);
  const categories = useMemo(
    () => [...new Set(catalog.voices.map((voice) => voice.category))].sort((left, right) => left.localeCompare(right, "zh-CN")),
    [catalog.voices],
  );
  const selectedVoice = useMemo(
    () => catalog.voices.find((voice) => voice.speakerId === settings.speakerId),
    [catalog.voices, settings.speakerId],
  );
  const currentSpeakerId = activeSpeaker(settings);
  const usingCustomVoice = isCustomVoice(settings);
  const currentVoiceName = usingCustomVoice ? "自定义音色" : selectedVoice?.name ?? "未选择";
  const voiceInstruction = settings.voiceInstruction?.trim() ?? "";
  const selectedExpression = VOICE_INSTRUCTION_PRESETS.find((preset) => preset.value === voiceInstruction);
  const expressionName = selectedExpression?.label ?? "自定义";
  const expressionDescription = selectedExpression?.description ?? "按你的要求调整表达";

  const filteredVoices = useMemo(() => {
    const query = search.trim().toLocaleLowerCase();
    return catalog.voices.filter((voice) => {
      if (catalogScope === "recommended" && !voice.recommended) return false;
      if (catalogScope === "favorites" && !favorites.has(voice.speakerId)) return false;
      if (gender !== "all" && voice.gender !== gender) return false;
      if (category !== "all" && voice.category !== category) return false;
      if (!query) return true;
      return [voice.name, voice.speakerId, voice.category, ...voice.tags]
        .join(" ")
        .toLocaleLowerCase()
        .includes(query);
    });
  }, [catalog.voices, catalogScope, category, favorites, gender, search]);

  useEffect(() => {
    voiceViewport.current?.scrollTo({ top: 0 });
  }, [catalogScope, category, gender, search]);

  async function toggleReading() {
    setToggleBusy(true);
    setFeedback(null);
    try {
      setStatus(await desktop.toggleReading());
    } catch (error) {
      showFeedback(readableError(error), "error");
    } finally {
      setToggleBusy(false);
    }
  }

  async function stopPlayback() {
    try {
      setStatus(await desktop.stopPlayback());
      setPreviewingSpeakerId(null);
    } catch (error) {
      showFeedback(readableError(error), "error");
    }
  }

  async function requestAccessibility() {
    try {
      await desktop.requestAccessibility();
      showFeedback("已打开系统辅助功能设置；授权后回到这里点击“重新检查”。");
    } catch (error) {
      showFeedback(readableError(error), "error");
    }
  }

  async function refreshAccessibility() {
    try {
      const nextStatus = await desktop.getStatus();
      setStatus(nextStatus);
      showFeedback(
        nextStatus.accessibilityTrusted ? "已检测到辅助功能授权。" : "仍未检测到辅助功能授权。",
        nextStatus.accessibilityTrusted ? "success" : "warning",
      );
    } catch (error) {
      showFeedback(readableError(error), "error");
    }
  }

  async function saveApiKey() {
    if (!apiKey.trim()) {
      showFeedback("请输入 API Key 后再保存。", "warning");
      return;
    }
    setKeyBusy(true);
    setFeedback(null);
    try {
      await desktop.saveApiKey(apiKey);
      setApiKey("");
      setStatus(await desktop.getStatus());
      showFeedback("API Key 已保存到系统钥匙串，不会在界面中回显。", "success");
    } catch (error) {
      showFeedback(readableError(error), "error");
    } finally {
      setKeyBusy(false);
    }
  }

  async function clearApiKey() {
    if (!window.confirm("确定要从系统钥匙串清除 API Key 吗？这不会撤销火山引擎侧的 Key。")) return;
    try {
      await desktop.clearApiKey();
      setStatus(await desktop.getStatus());
      showFeedback("API Key 已从系统钥匙串清除。", "success");
    } catch (error) {
      showFeedback(readableError(error), "error");
    }
  }

  async function exportDiagnostics() {
    setFeedback(null);
    try {
      const path = await desktop.exportDiagnostics();
      setDiagnosticsPath(path);
      showFeedback("诊断文件已导出，其中不包含 API Key 或选中文字。", "success");
    } catch (error) {
      showFeedback(readableError(error), "error");
    }
  }

  function selectVoice(speakerId: string) {
    queueSettings({ speakerId, customSpeakerId: "" }, 0);
  }

  function toggleFavorite(speakerId: string) {
    const nextFavorites = favorites.has(speakerId)
      ? settings.favoriteSpeakerIds.filter((id) => id !== speakerId)
      : [...settings.favoriteSpeakerIds, speakerId];
    queueSettings({ favoriteSpeakerIds: nextFavorites }, 0);
  }

  async function previewVoice(speakerId: string, applyInstruction = false) {
    setPreviewingSpeakerId(speakerId);
    setFeedback(null);
    try {
      await desktop.previewVoice(speakerId, applyInstruction);
    } catch (error) {
      setUnavailableSpeakerIds((current) => new Set(current).add(speakerId));
      showFeedback(readableError(error), "error");
    } finally {
      setPreviewingSpeakerId((current) => (current === speakerId ? null : current));
    }
  }

  async function previewCurrentVoice(applyInstruction: boolean) {
    if (saveTimer.current !== undefined) {
      window.clearTimeout(saveTimer.current);
      saveTimer.current = undefined;
    }
    await flushSettings();
    await previewVoice(settings.speakerId, applyInstruction);
  }

  async function beginShortcutRecording() {
    if (recordingShortcut) return;
    setShortcutError(null);
    setShortcutDraft(null);
    setFeedback(null);
    try {
      await desktop.beginShortcutRecording();
      setRecordingShortcut(true);
      window.requestAnimationFrame(() => shortcutRecorder.current?.focus());
    } catch (error) {
      setShortcutError(readableError(error));
    }
  }

  async function resetShortcut() {
    const defaultShortcut = /Mac/i.test(navigator.platform) ? "Command+Option+R" : "Ctrl+Alt+R";
    setShortcutError(null);
    try {
      await desktop.beginShortcutRecording();
      const saved = await desktop.commitShortcutRecording(defaultShortcut);
      settingsRevision.current += 1;
      setSettings(saved);
      setSaveState("saved");
      showFeedback("已恢复默认快捷键。", "success");
    } catch (error) {
      setShortcutError(readableError(error));
    }
  }

  function moveVoiceFocus(event: React.KeyboardEvent<HTMLDivElement>) {
    if (event.key !== "ArrowDown" && event.key !== "ArrowUp") return;
    const focusedId = document.activeElement?.getAttribute("data-voice-option");
    const currentIndex = filteredVoices.findIndex((voice) => voice.speakerId === focusedId);
    if (currentIndex < 0) return;
    event.preventDefault();
    const nextIndex = Math.max(
      0,
      Math.min(filteredVoices.length - 1, currentIndex + (event.key === "ArrowDown" ? 1 : -1)),
    );
    const nextVoice = filteredVoices[nextIndex];
    window.requestAnimationFrame(() => {
      const nextOption = voiceOptionRefs.current.get(nextVoice.speakerId);
      nextOption?.scrollIntoView({ block: "nearest" });
      nextOption?.focus();
    });
  }

  const saveLabel =
    saveState === "saving"
      ? "正在保存…"
      : saveState === "saved"
        ? "已保存"
        : saveState === "failed"
          ? "保存失败"
          : "本机自动保存";
  const noticeSignature = status.notice
    ? `${status.notice.kind}:${status.notice.code ?? ""}:${status.notice.message}`
    : null;
  const backendNotice = noticeSignature !== dismissedNotice ? status.notice : null;
  const visibleNotice = feedback ?? backendNotice;

  useEffect(() => {
    setDismissedNotice(null);
  }, [noticeSignature]);

  useEffect(() => {
    if (!visibleNotice || visibleNotice.kind === "warning" || visibleNotice.kind === "error") return;
    const timer = window.setTimeout(() => {
      if (feedback) setFeedback(null);
      else if (noticeSignature) setDismissedNotice(noticeSignature);
    }, 4200);
    return () => window.clearTimeout(timer);
  }, [feedback, noticeSignature, visibleNotice]);

  function dismissVisibleNotice() {
    if (feedback) setFeedback(null);
    else if (noticeSignature) setDismissedNotice(noticeSignature);
  }

  return (
    <main className="app-shell">
      <section className="desk-tool" aria-labelledby="app-title">
        <header className="tool-header">
          <div className="brand-block">
            <p className="eyebrow">XUANDU · SELECT TO LISTEN</p>
            <div className="title-row">
              <h1 id="app-title">选读</h1>
              <span className={`status-dot ${status.mode}`} aria-hidden="true" />
            </div>
            <p className="header-subtitle">选中一段文字，让它自然开口。</p>
          </div>
          <div className="header-actions">
            <span className={`save-state ${saveState}`} aria-live="polite">
              {saveLabel}
            </span>
            {status.mode === "playing" && (
              <button className="quiet-button button-with-icon" type="button" onClick={() => void stopPlayback()}>
                <Square size={13} aria-hidden="true" /> 停止
              </button>
            )}
            <button
              className={`mode-switch mode-${status.mode}`}
              type="button"
              onClick={() => void toggleReading()}
              disabled={toggleBusy}
              aria-pressed={status.mode === "armed" || status.mode === "playing"}
            >
              <span className="button-with-icon"><Power size={14} aria-hidden="true" />{statusLabel(status.mode)}</span>
              <kbd title={settings.shortcut}>{displayShortcut(settings.shortcut)}</kbd>
            </button>
          </div>
        </header>

        <section className="voice-strip" aria-label="当前音色">
          <div>
            <span>当前音色</span>
            <strong>{currentVoiceName}</strong>
          </div>
          <code title={currentSpeakerId}>{currentSpeakerId}</code>
          <button className="text-button button-with-icon" type="button" onClick={() => setTab("voices")}>
            <AudioLines size={14} aria-hidden="true" /> 更换音色
          </button>
        </section>

        <div className="tabs" role="tablist" aria-label="设置分类">
          <TabButton active={tab === "general"} id="general" onClick={() => setTab("general")}>
            常规
          </TabButton>
          <TabButton active={tab === "voices"} id="voices" onClick={() => setTab("voices")}>
            音色 <span>{catalog.voices.length || ""}</span>
          </TabButton>
          <TabButton active={tab === "advanced"} id="advanced" onClick={() => setTab("advanced")}>
            高级
          </TabButton>
        </div>

        <div
          className={`tab-panel tab-${tab}`}
          role="tabpanel"
          id={`panel-${tab}`}
          aria-labelledby={`tab-${tab}`}
        >
          {tab === "general" && (
            <section className="general-grid" aria-label="常规设置">
              <article className="setting-card mode-card">
                <p className="card-kicker">朗读状态</p>
                <h2>{statusLabel(status.mode)}</h2>
                <p>
                  {status.mode === "armed" || status.mode === "playing"
                    ? "在支持的前台应用中用鼠标选中文字即可朗读。"
                    : "开启后才会读取鼠标选区；不会读取剪贴板。"}
                </p>
                <button className="secondary-button button-with-icon" type="button" onClick={() => void toggleReading()} disabled={toggleBusy}>
                  <Power size={14} aria-hidden="true" />
                  {status.mode === "armed" || status.mode === "playing" ? "关闭朗读" : "开启朗读"}
                </button>
              </article>

              <article className="setting-card access-card">
                <p className="card-kicker">辅助功能</p>
                <h2>{status.accessibilityTrusted ? "已授权" : "需要授权"}</h2>
                <p>
                  {status.accessibilityTrusted
                    ? "可以读取支持辅助功能的前台控件选区。"
                    : "macOS 需要系统辅助功能权限；Windows 会安全跳过不支持的窗口。"}
                </p>
                <div className="button-row">
                  <button className="secondary-button button-with-icon" type="button" onClick={() => void refreshAccessibility()}>
                    {status.accessibilityTrusted ? <ShieldCheck size={14} aria-hidden="true" /> : <Accessibility size={14} aria-hidden="true" />}
                    重新检查
                  </button>
                  {!status.accessibilityTrusted && (
                    <button className="text-button" type="button" onClick={() => void requestAccessibility()}>
                      去授权 →
                    </button>
                  )}
                </div>
              </article>

              <article className="setting-card shortcut-card">
                <p className="card-kicker button-with-icon"><Keyboard size={13} aria-hidden="true" />模式切换快捷键</p>
                <div className="shortcut-actions">
                  <button
                    ref={shortcutRecorder}
                    className={`shortcut-recorder ${recordingShortcut ? "recording" : ""}`}
                    type="button"
                    onClick={() => void beginShortcutRecording()}
                    aria-label="录制模式切换快捷键"
                    aria-describedby="shortcut-help shortcut-error"
                    aria-pressed={recordingShortcut}
                  >
                    <Keyboard size={15} aria-hidden="true" />
                    {recordingShortcut
                      ? shortcutDraft ? displayShortcut(shortcutDraft) : "现在按下组合键…"
                      : displayShortcut(settings.shortcut)}
                  </button>
                  <button className="icon-button reset-shortcut" type="button" onClick={() => void resetShortcut()} title="恢复默认快捷键" aria-label="恢复默认快捷键">
                    <RotateCcw size={15} aria-hidden="true" />
                  </button>
                </div>
                <p id="shortcut-help">{recordingShortcut ? "按 Escape 取消；切换窗口会自动恢复原快捷键。" : "点击后直接按键录制，并立即检查系统冲突。"}</p>
                {shortcutError && <p className="field-error" id="shortcut-error" role="alert"><CircleAlert size={13} aria-hidden="true" />{shortcutError}</p>}
              </article>

              <label className="launch-row">
                <span>
                  <strong>登录时启动</strong>
                  <small>后台启动，朗读模式仍默认关闭。</small>
                </span>
                <input
                  type="checkbox"
                  checked={settings.launchAtLogin}
                  onChange={(event) => queueSettings({ launchAtLogin: event.target.checked }, 0)}
                />
              </label>
            </section>
          )}

          {tab === "voices" && (
            <section className="voice-panel" aria-label="官方中文 Seed TTS 2.0 音色">
              <div className="voice-topline">
                <div className="segmented-control" aria-label="音色范围">
                  <ScopeButton active={catalogScope === "recommended"} onClick={() => setCatalogScope("recommended")}>
                    推荐
                  </ScopeButton>
                  <ScopeButton active={catalogScope === "favorites"} onClick={() => setCatalogScope("favorites")}>
                    收藏 {favorites.size > 0 && <span>{favorites.size}</span>}
                  </ScopeButton>
                  <ScopeButton active={catalogScope === "all"} onClick={() => setCatalogScope("all")}>
                    全部
                  </ScopeButton>
                </div>
                <span className="catalog-version">目录 {catalog.catalogVersion || "加载中"}</span>
              </div>

              <div className="voice-filters">
                <label className="search-field">
                  <span className="sr-only">搜索音色</span>
                  <Search className="field-icon" size={14} aria-hidden="true" />
                  <input
                    value={search}
                    onChange={(event) => setSearch(event.target.value)}
                    placeholder="搜索名称或 Speaker ID"
                    type="search"
                  />
                </label>
                <label className="select-field">
                  <span className="sr-only">筛选性别</span>
                  <select value={gender} onChange={(event) => setGender(event.target.value)}>
                    <option value="all">全部性别</option>
                    <option value="female">女声</option>
                    <option value="male">男声</option>
                    <option value="child">童声</option>
                  </select>
                </label>
                <label className="select-field">
                  <span className="sr-only">筛选场景</span>
                  <select value={category} onChange={(event) => setCategory(event.target.value)}>
                    <option value="all">全部场景</option>
                    {categories.map((value) => (
                      <option key={value} value={value}>
                        {value}
                      </option>
                    ))}
                  </select>
                </label>
              </div>

              <div
                className="voice-results"
                ref={voiceViewport}
                role="listbox"
                aria-label="官方中文音色"
                onKeyDown={moveVoiceFocus}
              >
                {filteredVoices.length === 0 ? (
                  <p className="empty-voices">没有符合条件的音色。试试清除筛选，或先收藏一个音色。</p>
                ) : (
                  <div className="voice-list-inner">
                    {filteredVoices.map((voice) => {
                      const selected = !usingCustomVoice && settings.speakerId === voice.speakerId;
                      const favorite = favorites.has(voice.speakerId);
                      const unavailable = unavailableSpeakerIds.has(voice.speakerId);
                      const previewing = previewingSpeakerId === voice.speakerId;
                      return (
                        <div className={`voice-row ${selected ? "selected" : ""}`} key={voice.speakerId}>
                          <button
                            className="voice-choice"
                            data-voice-option={voice.speakerId}
                            ref={(node) => {
                              if (node) voiceOptionRefs.current.set(voice.speakerId, node);
                              else voiceOptionRefs.current.delete(voice.speakerId);
                            }}
                            type="button"
                            role="option"
                            aria-selected={selected}
                            onClick={() => selectVoice(voice.speakerId)}
                          >
                            <span className="voice-name">{voice.name}</span>
                            <span className="voice-meta">
                              {genderLabel(voice.gender)} · {voice.category}
                            </span>
                            <code title={voice.speakerId}>{voice.speakerId}</code>
                          </button>
                          <button
                            className={`icon-button ${favorite ? "favorite" : ""}`}
                            type="button"
                            onClick={() => toggleFavorite(voice.speakerId)}
                            aria-label={favorite ? `取消收藏 ${voice.name}` : `收藏 ${voice.name}`}
                            aria-pressed={favorite}
                          >
                            {favorite ? <Star size={17} fill="currentColor" aria-hidden="true" /> : <Heart size={17} aria-hidden="true" />}
                          </button>
                          <button
                            className="preview-button"
                            type="button"
                            onClick={() => void previewVoice(voice.speakerId, false)}
                            disabled={previewing}
                          >
                            {!previewing && !unavailable && <Play size={12} aria-hidden="true" />}
                            {previewing ? "试听中…" : unavailable ? "当前不可用" : "试听"}
                          </button>
                        </div>
                      );
                    })}
                  </div>
                )}
              </div>

              <div className="rate-grid">
                <RangeControl
                  id="speech-rate"
                  label="语速"
                  value={settings.speechRate}
                  onChange={(speechRate) => queueSettings({ speechRate })}
                />
                <RangeControl
                  id="loudness-rate"
                  label="音量"
                  value={settings.loudnessRate}
                  onChange={(loudnessRate) => queueSettings({ loudnessRate })}
                />
                <RangeControl
                  id="pitch"
                  label="音调"
                  value={settings.pitch}
                  min={-12}
                  max={12}
                  step={1}
                  formatValue={(pitch) => `${pitch > 0 ? "+" : ""}${pitch}`}
                  icon="pitch"
                  onChange={(pitch) => queueSettings({ pitch })}
                />
              </div>

              <section className="expression-panel" aria-labelledby="expression-title">
                <div className="expression-header">
                  <div className="expression-heading">
                    <span className="expression-icon"><MessageSquareText size={15} aria-hidden="true" /></span>
                    <span>
                      <strong id="expression-title">表达方式</strong>
                      <small>{expressionName} · {expressionDescription}</small>
                    </span>
                  </div>
                  <div className="expression-preview-actions">
                    <button
                      className="secondary-button button-with-icon"
                      type="button"
                      onClick={() => void previewCurrentVoice(false)}
                      disabled={usingCustomVoice || previewingSpeakerId !== null}
                    >
                      <Play size={12} aria-hidden="true" />原声
                    </button>
                    <button
                      className="primary-button button-with-icon"
                      type="button"
                      onClick={() => void previewCurrentVoice(Boolean(voiceInstruction))}
                      disabled={usingCustomVoice || previewingSpeakerId !== null}
                    >
                      <AudioLines size={13} aria-hidden="true" />{previewingSpeakerId ? "试听中…" : "试听当前"}
                    </button>
                  </div>
                </div>
                <div className="expression-presets" aria-label="表达方式预设">
                  {VOICE_INSTRUCTION_PRESETS.map((preset) => (
                    <button
                      className={voiceInstruction === preset.value ? "active" : ""}
                      type="button"
                      key={preset.id}
                      title={preset.description}
                      aria-pressed={voiceInstruction === preset.value}
                      onClick={() => queueSettings({ voiceInstruction: preset.value }, 0)}
                      disabled={usingCustomVoice}
                    >
                      {preset.label}
                      {voiceInstruction === preset.value && <CheckCircle2 size={12} aria-hidden="true" />}
                    </button>
                  ))}
                </div>
                <details className="custom-expression">
                  <summary>
                    <span>自定义表达要求</span>
                    <span>{!selectedExpression && voiceInstruction ? "已启用" : "高级"}<ChevronDown size={13} aria-hidden="true" /></span>
                  </summary>
                  <div className="custom-expression-body">
                  {usingCustomVoice && (
                    <p className="instruction-warning" role="note">
                      自定义 Speaker ID 不发送表达指令。请切换到官方 Seed TTS 2.0 音色。
                    </p>
                  )}
                  <label className="instruction-field">
                    <span>用一句话描述想要的语气</span>
                    <textarea
                      value={settings.voiceInstruction ?? ""}
                      onChange={(event) => queueSettings({ voiceInstruction: event.target.value })}
                      placeholder="例如：请用温柔、放松的语气说话。"
                      maxLength={300}
                      disabled={usingCustomVoice}
                      rows={3}
                    />
                    <span>{voiceInstruction.length}/300</span>
                  </label>
                  <div className="custom-expression-footer">
                    <p><Info size={12} aria-hidden="true" />表达效果会受正文语义、标点和音色影响。</p>
                    <button
                      className="secondary-button button-with-icon"
                      type="button"
                      onClick={() => queueSettings({ voiceInstruction: "" }, 0)}
                      disabled={!voiceInstruction}
                    >
                      恢复自然
                    </button>
                  </div>
                  </div>
                </details>
              </section>
            </section>
          )}

          {tab === "advanced" && (
            <section className="advanced-panel" aria-label="高级设置">
              <article className="api-key-card">
                <div>
                  <p className="card-kicker button-with-icon"><KeyRound size={13} aria-hidden="true" />火山引擎 API Key</p>
                  <h2>{status.hasApiKey ? "密钥已保存在系统钥匙串" : "连接你的语音服务"}</h2>
                  <p>只在 Rust 层写入 Keychain / Credential Manager；不会写进本机配置或日志。</p>
                </div>
                <label className="api-key-field">
                  <span className="sr-only">火山引擎 API Key</span>
                  <input
                    value={apiKey}
                    onChange={(event) => setApiKey(event.target.value)}
                    placeholder={status.hasApiKey ? "输入新 Key 以替换现有密钥" : "粘贴 API Key"}
                    type="password"
                    autoComplete="off"
                    spellCheck={false}
                  />
                  <button className="primary-button button-with-icon" type="button" onClick={() => void saveApiKey()} disabled={keyBusy}>
                    <LockKeyhole size={14} aria-hidden="true" />
                    {keyBusy ? "正在保存…" : status.hasApiKey ? "替换 Key" : "保存 Key"}
                  </button>
                </label>
                {status.hasApiKey && (
                  <button className="danger-button button-with-icon" type="button" onClick={() => void clearApiKey()}>
                    <Trash2 size={13} aria-hidden="true" /> 清除 API Key
                  </button>
                )}
              </article>

              <details className="custom-speaker">
                <summary>自定义 Speaker ID</summary>
                <p>只在你的账号已开通私有或指定音色时填写；留空会恢复使用当前官方目录音色。</p>
                <label className="field">
                  <span>Speaker ID</span>
                  <input
                    value={settings.customSpeakerId ?? ""}
                    onChange={(event) => queueSettings({ customSpeakerId: event.target.value })}
                    placeholder="例如：your-speaker-id"
                    spellCheck={false}
                  />
                </label>
              </details>

              <article className="privacy-note">
                <p className="card-kicker">隐私边界</p>
                <p>不读取剪贴板，不保存选中文字，不保留朗读历史；文本只用于当前一次语音请求。</p>
                <button className="secondary-button button-with-icon" type="button" onClick={() => void exportDiagnostics()}>
                  <FileDown size={14} aria-hidden="true" />导出诊断信息
                </button>
                {diagnosticsPath && <code className="diagnostics-path" title={diagnosticsPath}>{diagnosticsPath}</code>}
              </article>
            </section>
          )}
        </div>

        <footer className={`status-bar ${visibleNotice?.kind ?? "idle"}`}>
          {visibleNotice ? (
            <div
              className="status-message"
              role={visibleNotice.kind === "error" ? "alert" : "status"}
              aria-live={visibleNotice.kind === "error" ? "assertive" : "polite"}
            >
              <StatusIcon kind={visibleNotice.kind} />
              <span title={visibleNotice.message}>{visibleNotice.message}</span>
              <button type="button" onClick={dismissVisibleNotice} aria-label="关闭提示" title="关闭提示">
                <X size={13} aria-hidden="true" />
              </button>
            </div>
          ) : (
            <>
              <span>鼠标选中 2–5000 个字后自动朗读</span>
              <span>关闭窗口后仍在托盘运行</span>
            </>
          )}
        </footer>
      </section>
    </main>
  );
}

function StatusIcon({ kind }: { kind: NoticeKind }) {
  if (kind === "success") return <CheckCircle2 size={13} aria-hidden="true" />;
  if (kind === "warning") return <TriangleAlert size={13} aria-hidden="true" />;
  if (kind === "error") return <CircleAlert size={13} aria-hidden="true" />;
  return <Info size={13} aria-hidden="true" />;
}

function TabButton({
  active,
  id,
  onClick,
  children,
}: {
  active: boolean;
  id: Tab;
  onClick: () => void;
  children: React.ReactNode;
}) {
  return (
    <button
      className={active ? "active" : ""}
      id={`tab-${id}`}
      type="button"
      role="tab"
      aria-selected={active}
      aria-controls={`panel-${id}`}
      onClick={onClick}
    >
      {children}
    </button>
  );
}

function ScopeButton({
  active,
  onClick,
  children,
}: {
  active: boolean;
  onClick: () => void;
  children: React.ReactNode;
}) {
  return (
    <button className={active ? "active" : ""} type="button" onClick={onClick} aria-pressed={active}>
      {children}
    </button>
  );
}

function RangeControl({
  id,
  label,
  value,
  onChange,
  min = RATE_MIN,
  max = RATE_MAX,
  step = 5,
  formatValue = rateMultiplier,
  icon = "volume",
}: {
  id: string;
  label: string;
  value: number;
  onChange: (value: number) => void;
  min?: number;
  max?: number;
  step?: number;
  formatValue?: (value: number) => string;
  icon?: "volume" | "pitch";
}) {
  const Icon = icon === "pitch" ? Music2 : Volume2;
  const displayValue = formatValue(value);
  return (
    <div className="range-control">
      <label className="button-with-icon" htmlFor={id}><Icon size={13} aria-hidden="true" />{label}</label>
      <div>
        <input
          id={id}
          type="range"
          min={min}
          max={max}
          step={step}
          value={value}
          onChange={(event) => onChange(Number(event.target.value))}
          aria-valuetext={displayValue}
        />
        <output htmlFor={id}>{displayValue}</output>
      </div>
    </div>
  );
}
