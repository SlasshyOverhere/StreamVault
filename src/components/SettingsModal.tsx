import { useState, useEffect, useCallback } from "react";
import { Dialog, DialogContent } from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Button } from "@/components/ui/button";
import {
  Trash2,
  MonitorPlay,
  FolderOpen,
  AlertTriangle,
  Settings,
  Key,
  Zap,
  Power,
  X,
  Save,
  Cloud,
  Download,
  RefreshCw,
  FlaskConical,
  Radio,
  Shield,
  Archive,
  HardDrive,
  Loader2,
  Bug,
  Wifi,
} from "lucide-react";
import {
  Config,
  getConfig,
  saveConfig,
  clearAllAppData,
  deleteAllMediaFiles,
  TabVisibility,
  checkForUpdates,
  downloadUpdate,
  installUpdate,
  getAppVersion,
  UpdateInfo,
  autoDetectMpv,
  getBundledMpvInfo,
  downloadBundledMpv,
  BundledMpvInfo,
  getTopSpaceConsumers,
  getGdriveAccountInfo,
  getCloudCacheInfo,
  cleanupCloudCache,
  clearCloudCache,
  getDownloadJobs,
  MediaItem,
  DriveAccountInfo,
  CloudCacheInfo,
  DownloadJob,
} from "@/services/api";

import { useToast } from "@/components/ui/use-toast";
import { open as openDialog } from "@tauri-apps/plugin-dialog";
import { invoke } from "@tauri-apps/api/core";
import { emit } from "@tauri-apps/api/event";
import { Switch } from "@/components/ui/switch";
import { LazyMotion, domAnimation, m, AnimatePresence } from "framer-motion";
import { cn } from "@/lib/utils";
import { GoogleDriveSettings } from "@/components/GoogleDriveSettings";
import { ZipGuideModal } from "@/components/ZipGuideModal";
import { SelectiveDeleteModal } from "@/components/SelectiveDeleteModal";
import { BetaConfirmDialog } from "@/components/BetaConfirmDialog";
import { DebridServicesPanel } from "@/components/DebridServicesPanel";

interface SettingsModalProps {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  initialTab?: SettingsSection;
  tabVisibility?: TabVisibility;
  onTabVisibilityChange?: (visibility: TabVisibility) => void;
  onLogout?: () => void;
  betaEnabled?: boolean;
  onBetaToggle?: (enabled: boolean) => void;
  autoCheckUpdate?: boolean;
  onSimulateUpdate?: () => void;
}

type SettingsSection =
  | "general"
  | "account"
  | "beta"
  | "updates"
  | "cloud"
  | "storage"
  | "api"
  | "external"
  | "danger"
  | "dev"
  | "nightly"
  | "relay";

const sections: {
  id: SettingsSection;
  label: string;
  icon: React.ReactNode;
}[] = [
  { id: "general", label: "General", icon: <Settings className="size-4" /> },
  {
    id: "account",
    label: "Account",
    icon: <Power className="size-4" />,
  },
  {
    id: "updates",
    label: "Updates",
    icon: <Shield className="size-4" />,
  },
  {
    id: "cloud",
    label: "Cache & Storage",
    icon: <Cloud className="size-4" />,
  },
  {
    id: "storage",
    label: "Storage & Bandwidth",
    icon: <Archive className="size-4" />,
  },
  { id: "api", label: "API Keys", icon: <Key className="size-4" /> },
  { id: "external", label: "External", icon: <Radio className="size-4" /> },
  { id: "relay", label: "Watch Together", icon: <Wifi className="size-4" /> },
  {
    id: "danger",
    label: "Factory Reset",
    icon: <AlertTriangle className="size-4" />,
  },
  { id: "beta", label: "Beta", icon: <FlaskConical className="size-4" /> },
  ...(import.meta.env.DEV
    ? []
    : []),
  ...(import.meta.env.VITE_IS_NIGHTLY === 'true'
    ? [{ id: "nightly" as SettingsSection, label: "Nightly", icon: <Bug className="size-4" /> }]
    : []),
];

// Addon Source interface matching Rust backend
interface AddonSource {
  id: string;
  name: string;
  url: string;
  enabled: boolean;
  is_default: boolean;
  binary_path?: string;
}

function AddonSourcesManager() {
  const { toast } = useToast();
  const [sources, setSources] = useState<AddonSource[]>([]);
  const [loading, setLoading] = useState(true);
  const [newName, setNewName] = useState("");
  const [newUrl, setNewUrl] = useState("");
  const [adding, setAdding] = useState(false);
  const [sourceStatus, setSourceStatus] = useState<Record<string, { online: boolean; version: string | null; checking: boolean }>>({});

  // Load sources immediately — no blocking on version checks
  const loadSources = useCallback(async () => {
    try {
      const data = await invoke<AddonSource[]>("get_addon_sources");
      setSources(data);
      setLoading(false);
      // Fire-and-forget health checks per source
      for (const src of data) {
        checkSourceHealth(src.id, src.url);
      }
    } catch (e) {
      console.error("[AddonSourcesManager] loadSources:", e);
      setLoading(false);
    }
  }, []);

  // Check a single source's health + version (with timeout)
  const checkSourceHealth = useCallback(async (id: string, url: string) => {
    setSourceStatus(prev => ({ ...prev, [id]: { ...prev[id], online: false, version: null, checking: true } }));
    try {
      // Check online with 2s timeout
      const online = await Promise.race([
        invoke<boolean>("check_addon_server", { url }),
        new Promise<boolean>((_, rej) => setTimeout(() => rej(new Error("timeout")), 2000))
      ]);
      let version: string | null = null;
      if (online) {
        try {
          version = await Promise.race([
            invoke<string | null>("get_addon_version", { url }),
            new Promise<null>((resolve) => setTimeout(() => resolve(null), 2000))
          ]);
        } catch {
          console.debug('[Settings] Addon version check failed for', id)
        }
      }
      setSourceStatus(prev => ({ ...prev, [id]: { online, version, checking: false } }));
    } catch (e) {
      console.warn('[Settings] Source status check failed:', e)
      setSourceStatus(prev => ({ ...prev, [id]: { online: false, version: null, checking: false } }));
    }
  }, []);

  useEffect(() => { loadSources(); }, [loadSources]);

  const handleAdd = useCallback(async () => {
    if (!newUrl.trim()) return;
    setAdding(true);
    try {
      await invoke("add_addon_source", { name: newName, url: newUrl });
      setNewName("");
      setNewUrl("");
      await loadSources();
      window.dispatchEvent(new CustomEvent("config-saved"));
      toast({ title: "Source added" });
    } catch (e: any) {
      toast({ title: "Failed to add source", description: e?.message || String(e), variant: "destructive" });
    } finally {
      setAdding(false);
    }
  }, [newName, newUrl, loadSources, toast]);

  const handleRemove = useCallback(async (id: string) => {
    try {
      await invoke("remove_addon_source", { id });
      setSources(prev => prev.filter(s => s.id !== id));
      setSourceStatus(prev => { const next = { ...prev }; delete next[id]; return next; });
      window.dispatchEvent(new CustomEvent("config-saved"));
      toast({ title: "Source removed" });
    } catch (e: any) {
      toast({ title: "Failed to remove", description: e?.message || String(e), variant: "destructive" });
    }
  }, [toast]);

  const handleSetActive = useCallback(async (id: string) => {
    try {
      await invoke("set_active_source", { id });
      await loadSources();
      window.dispatchEvent(new CustomEvent("config-saved"));
    } catch (e: any) {
      toast({ title: "Failed to set active", description: e?.message || String(e), variant: "destructive" });
    }
  }, [loadSources, toast]);

  if (loading) {
    return (
      <div className="flex items-center gap-2 text-sm text-muted-foreground">
        <Loader2 className="size-4 animate-spin" /> Loading sources...
      </div>
    );
  }

  return (
    <div className="space-y-4">
      {/* Existing sources */}
      {sources.length > 0 && (
        <div className="space-y-2">
          {sources.map((src) => {
            const status = sourceStatus[src.id];
            const isOnline = status?.online ?? false;
            const version = status?.version;
            const checking = status?.checking ?? false;
            return (
              <div
                key={src.id}
                className={`p-3 rounded-xl border flex items-center gap-3 ${
                  src.is_default
                    ? "bg-white/5 border-white/10"
                    : "bg-card border-border"
                }`}
              >
                <div className="flex-1 min-w-0">
                  <div className="flex items-center gap-2">
                    {/* Online/offline indicator */}
                    <div className={`size-2 rounded-full shrink-0 ${checking ? 'bg-yellow-500 animate-pulse' : isOnline ? 'bg-emerald-500' : 'bg-red-500/60'}`}
                      title={checking ? 'Checking...' : isOnline ? 'Online' : 'Offline'} />
                    <span className="text-sm font-medium truncate">{src.name}</span>
                    {version && (
                      <span className="text-[10px] px-1.5 py-0.5 rounded bg-white/5 text-white/40">v{version}</span>
                    )}
                    {src.is_default && (
                      <span className="text-[10px] px-1.5 py-0.5 rounded bg-white/10 text-white/60 uppercase tracking-wider">
                        Active
                      </span>
                    )}
                  </div>
                  <p className="text-xs text-muted-foreground truncate mt-0.5">
                    {src.url}{src.binary_path ? ' · binary' : ''}
                    {!checking && !isOnline && <span className="text-red-400/60 ml-2">· not responding</span>}
                  </p>
                </div>
                <div className="flex items-center gap-1.5">
                  <button
                    onClick={() => checkSourceHealth(src.id, src.url)}
                    disabled={checking}
                    className="p-1.5 rounded-lg hover:bg-white/5 text-muted-foreground hover:text-foreground transition-colors"
                    title="Re-check"
                  >
                    <Loader2 className={`size-3.5 ${checking ? 'animate-spin' : ''}`} />
                  </button>
                  {!src.is_default && (
                    <button
                      onClick={() => handleSetActive(src.id)}
                      className="text-xs px-2 py-1 rounded-lg bg-white/5 hover:bg-white/10 text-muted-foreground hover:text-foreground transition-colors"
                    >
                      Set Active
                    </button>
                  )}
                  <button
                    onClick={() => handleRemove(src.id)}
                    className="p-1.5 rounded-lg hover:bg-destructive/10 text-muted-foreground hover:text-destructive transition-colors"
                  >
                    <Trash2 className="size-3.5" />
                  </button>
                </div>
              </div>
            );
          })}
        </div>
      )}

      {sources.length === 0 && (
        <div className="text-center py-6">
          <p className="text-sm text-muted-foreground">No addon sources configured.</p>
        </div>
      )}

      {/* Add new source manually */}
      <div className="p-4 rounded-xl bg-card border border-border space-y-3">
        <Label className="text-sm font-medium">Add Source</Label>
        <Input
          placeholder="Source name (optional)"
          value={newName}
          onChange={(e) => setNewName(e.target.value)}
          className="h-9"
        />
        <Input
          type="url"
          placeholder="http://127.0.0.1:11470"
          value={newUrl}
          onChange={(e) => setNewUrl(e.target.value)}
          onKeyDown={(e) => { if (e.key === "Enter") handleAdd(); }}
          className="h-9"
        />
        <button
          onClick={handleAdd}
          disabled={!newUrl.trim() || adding}
          className="w-full h-9 rounded-xl bg-white/5 hover:bg-white/10 disabled:opacity-40 disabled:cursor-not-allowed text-sm font-medium transition-colors"
        >
          {adding ? "Adding..." : "Add Source"}
        </button>
      </div>

    </div>
  );
}

export function SettingsModal({
  open,
  onOpenChange,
  initialTab,
  tabVisibility: _tabVisibility,
  onTabVisibilityChange: _onTabVisibilityChange,
  onLogout,
  betaEnabled = false,
  onBetaToggle,
  autoCheckUpdate = false,
  onSimulateUpdate: _onSimulateUpdate,
}: SettingsModalProps) {
  const [showBetaConfirm, setShowBetaConfirm] = useState(false);
  const [config, setConfig] = useState<Config>({
    mpv_path: "",
    vlc_path: "",
    ffprobe_path: "",
    ffmpeg_path: "",
    tmdb_api_key: "",
    omdb_api_key: "",
    cloud_cache_enabled: false,
    cloud_cache_dir: "",
    cloud_cache_max_mb: 1024,
    cloud_cache_expiry_hours: 24,
    zip_indexing_enabled: true,
    zip_cache_dir: "",
    zip_cache_max_gb: 20,
    zip_cache_expiry_days: 7,
    player_mode: "external",
    addon_url: "",
  });
  const [loading, setLoading] = useState(false);
  const [autoStart, setAutoStart] = useState(false);
  const [showResetConfirm, setShowResetConfirm] = useState(false);
  const [resetting, setResetting] = useState(false);
  const [deleteAllStep, setDeleteAllStep] = useState<0 | 1 | 2>(0);
  const [deleteAllConfirmText, setDeleteAllConfirmText] = useState("");
  const [deletingAll, setDeletingAll] = useState(false);
  const [showSelectiveDelete, setShowSelectiveDelete] = useState(false);
  const [showLogoutConfirm, setShowLogoutConfirm] = useState(false);
  const [loggingOut, setLoggingOut] = useState(false);
  const [activeSection, setActiveSection] =
    useState<SettingsSection>("general");
  const [appVersion, setAppVersion] = useState<string>("");
  const [updateInfo, setUpdateInfo] = useState<UpdateInfo | null>(null);
  const [checkingUpdate, setCheckingUpdate] = useState(false);
  const [downloadingUpdate, setDownloadingUpdate] = useState(false);
  const [downloadProgress, setDownloadProgress] = useState(0);
  const [detectingMpv, setDetectingMpv] = useState(false);
  const [bundledMpvInfo, setBundledMpvInfo] = useState<BundledMpvInfo | null>(null);
  const [downloadingBundledMpv, setDownloadingBundledMpv] = useState(false);
  const [bundledMpvProgress, setBundledMpvProgress] = useState(0);
  const [showCustomMpv, setShowCustomMpv] = useState(false);
  // ponytail: api key section simplified — no useOwnApiKey toggle needed
  const [showZipGuide, setShowZipGuide] = useState(false);
  const [driveInfo, setDriveInfo] = useState<DriveAccountInfo | null>(null);
  const [cacheInfo, setCacheInfo] = useState<CloudCacheInfo | null>(null);
  const [topMedia, setTopMedia] = useState<MediaItem[]>([]);
  const [recentDownloads, setRecentDownloads] = useState<DownloadJob[]>([]);
  const [storageLoading, setStorageLoading] = useState(false);
  const [pathValidation, setPathValidation] = useState<Record<string, string>>({});
  const [showDevConsole, setShowDevConsole] = useState(() => {
    return localStorage.getItem("slasshyvault_show_dev_console") === "true";
  });
  const { toast } = useToast();
  const isWindows = /windows/i.test(navigator.userAgent);

  const validatePath = useCallback((path: string, label: string) => {
    if (!path) {
      setPathValidation(prev => ({ ...prev, [label]: "" }));
      return;
    }
    if (path.includes("..") || path.includes("~")) {
      setPathValidation(prev => ({ ...prev, [label]: "Path contains relative segments" }));
    } else if (path.length > 260) {
      setPathValidation(prev => ({ ...prev, [label]: "Path too long" }));
    } else {
      setPathValidation(prev => ({ ...prev, [label]: "" }));
    }
  }, []);

  useEffect(() => {
    if (open) {
      loadConfig();
      checkAutoStart();
      loadAppVersion();
      loadBundledMpvInfo();
      setActiveSection(initialTab || "general");
      setShowResetConfirm(false);
    }
  }, [open, initialTab]);

  // Auto-trigger update check when navigated from update notification
  useEffect(() => {
    if (open && autoCheckUpdate && activeSection === "updates") {
      handleCheckUpdate();
    }
  }, [open, autoCheckUpdate, activeSection]);

  // Fetch storage & bandwidth data when section is opened
  useEffect(() => {
    if (open && activeSection === "storage") {
      setStorageLoading(true);
      Promise.all([
        getGdriveAccountInfo(),
        getCloudCacheInfo(),
        getTopSpaceConsumers(10),
        getDownloadJobs(),
      ]).then(([drive, cache, media, downloads]) => {
        setDriveInfo(drive);
        setCacheInfo(cache);
        setTopMedia(media);
        setRecentDownloads(downloads.filter(d => d.status === "completed" || d.status === "downloading").slice(0, 5));
      }).finally(() => setStorageLoading(false));
    }
  }, [open, activeSection]);

  const loadAppVersion = async () => {
    try {
      const version = await getAppVersion();
      setAppVersion(version);
    } catch (error) {
      console.error("Failed to load app version", error);
    }
  };

  const handleCheckUpdate = async () => {
    setCheckingUpdate(true);
    setUpdateInfo(null);
    try {
      const info = await checkForUpdates();
      setUpdateInfo(info);
      if (!info.available) {
        toast({
          title: "Up to Date",
          description: `You're running the latest version (${info.current_version})`,
        });
      }
    } catch (error) {
      console.error("Failed to check for updates", error);
      const description =
        typeof error === "string"
          ? error
          : error instanceof Error
            ? error.message
            : String(error);
      toast({
        title: "Error",
        description,
        variant: "destructive",
      });
    } finally {
      setCheckingUpdate(false);
    }
  };

  const handleDownloadAndInstall = async () => {
    if (!updateInfo?.download_url) return;

    setDownloadingUpdate(true);
    setDownloadProgress(0);
    try {
      // Listen for download progress events
      const { listen } = await import("@tauri-apps/api/event");
      const unlisten = await listen<{ progress: number }>(
        "update-download-progress",
        (event) => {
          setDownloadProgress(event.payload.progress);
        },
      );

      const installerPath = await downloadUpdate(updateInfo.download_url, updateInfo.published_at ?? undefined);
      unlisten();

      toast({
        title: "Download Complete",
        description: "Installing update and restarting…",
      });

      // Small delay to show the toast
      await new Promise((resolve) => setTimeout(resolve, 1000));

      await installUpdate(installerPath);
    } catch (error) {
      console.error("Failed to download/install update", error);
      const description =
        typeof error === "string"
          ? error
          : error instanceof Error
            ? error.message
            : String(error);
      toast({
        title: "Error",
        description,
        variant: "destructive",
      });
    } finally {
      setDownloadingUpdate(false);
      setDownloadProgress(0);
    }
  };

  const checkAutoStart = async () => {
    try {
      const enabled = await invoke<boolean>("plugin:autostart|is_enabled");
      setAutoStart(enabled);
    } catch (error) {
      console.error("Failed to check autostart", error);
    }
  };

  const toggleAutoStart = async (checked: boolean) => {
    try {
      if (checked) {
        await invoke("plugin:autostart|enable");
        toast({
          title: "Auto Startup Enabled",
          description: "SlasshyVault will now start automatically.",
        });
      } else {
        await invoke("plugin:autostart|disable");
        toast({
          title: "Auto Startup Disabled",
          description: "SlasshyVault will not start automatically.",
        });
      }
      setAutoStart(checked);
    } catch (error) {
      console.error("Failed to toggle autostart", error);
      toast({
        title: "Error",
        description: "Failed to update startup settings",
        variant: "destructive",
      });
    }
  };

  const loadConfig = async () => {
    try {
      const data = await getConfig();
      setConfig({
        mpv_path: data.mpv_path || "",
        vlc_path: data.vlc_path || "",
        ffprobe_path: data.ffprobe_path || "",
        ffmpeg_path: data.ffmpeg_path || "",
        tmdb_api_key: data.tmdb_api_key || "",
        omdb_api_key: data.omdb_api_key || "",
        cloud_cache_enabled: data.cloud_cache_enabled ?? false,
        cloud_cache_dir: data.cloud_cache_dir || "",
        cloud_cache_max_mb: data.cloud_cache_max_mb ?? 1024,
        cloud_cache_expiry_hours: data.cloud_cache_expiry_hours ?? 24,
        zip_indexing_enabled: data.zip_indexing_enabled ?? true,
        zip_cache_dir: data.zip_cache_dir || "",
        zip_cache_max_gb: data.zip_cache_max_gb ?? 20,
        zip_cache_expiry_days: data.zip_cache_expiry_days ?? 7,
        player_mode: data.player_mode || "external",
        addon_url: data.addon_url || "",
      });
      // If user already has a custom API key saved, show the custom input
      // ponytail: api key toggle removed
    } catch (error) {
      console.error("Failed to load config", error);
      toast({
        title: "Error",
        description: "Failed to load configuration",
        variant: "destructive",
      });
    }
  };

  const handleSave = async () => {
    setLoading(true);
    try {
      await saveConfig(config);
      toast({ title: "Success", description: "Settings saved successfully" });
      onOpenChange(false);
    } catch (error) {
      console.error("Failed to save config", error);
      toast({
        title: "Error",
        description: "Failed to save settings",
        variant: "destructive",
      });
    } finally {
      setLoading(false);
    }
  };

  const handleResetApp = async () => {
    setResetting(true);
    try {
      await clearAllAppData();
      setShowResetConfirm(false);
      onOpenChange(false);
      invoke('restart_app');
    } catch (error) {
      console.error("Failed to reset app", error);
      toast({
        title: "Error",
        description: "Failed to reset app data",
        variant: "destructive",
      });
    } finally {
      setResetting(false);
    }
  };

  const handleDeleteAllMedia = async () => {
    setDeletingAll(true);
    try {
      const result = await deleteAllMediaFiles();
      setDeleteAllStep(0);
      setDeleteAllConfirmText("");
      toast({
        title: "All Media Deleted",
        description: result.message,
      });
      emit("library-updated");
    } catch (error) {
      console.error("Failed to delete all media", error);
      toast({
        title: "Error",
        description: "Failed to delete all media files",
        variant: "destructive",
      });
    } finally {
      setDeletingAll(false);
    }
  };

  const browseMpvPath = async () => {
    try {
      const selected = await openDialog({
        multiple: false,
        ...(isWindows ? { filters: [{ name: "Executable", extensions: ["exe"] }] } : {}),
        title: "Select MPV Executable",
      });
      if (selected && typeof selected === "string") {
        setConfig({ ...config, mpv_path: selected });
      }
    } catch (error) {
      console.error("Failed to open file dialog", error);
    }
  };

  const browseZipCacheDir = async () => {
    try {
      const selected = await openDialog({
        directory: true,
        multiple: false,
        title: "Select ZIP Cache Directory",
      });
      if (selected && typeof selected === "string") {
        setConfig({ ...config, zip_cache_dir: selected });
      }
    } catch (error) {
      console.error("Failed to open directory dialog", error);
    }
  };

  const loadBundledMpvInfo = async () => {
    const info = await getBundledMpvInfo();
    setBundledMpvInfo(info);
  };

  const handleDownloadBundledMpv = async () => {
    setDownloadingBundledMpv(true);
    setBundledMpvProgress(0);
    try {
      const { listen } = await import("@tauri-apps/api/event");
      const unlisten = await listen<{ progress: number }>(
        "mpv-download-progress",
        (event) => {
          setBundledMpvProgress(event.payload.progress);
        },
      );

      const path = await downloadBundledMpv();
      unlisten();

      setConfig({ ...config, mpv_path: path });

      // Refresh bundled MPV info
      await loadBundledMpvInfo();

      toast({
        title: "MPV Installed",
        description: "Bundled MPV player has been installed successfully.",
      });
    } catch (error) {
      console.error("Failed to download bundled MPV:", error);
      const description =
        typeof error === "string"
          ? error
          : error instanceof Error
            ? error.message
            : String(error);
      toast({
        title: "Download Failed",
        description,
        variant: "destructive",
      });
    } finally {
      setDownloadingBundledMpv(false);
      setBundledMpvProgress(0);
    }
  };

  const handleAutoDetectMpv = async () => {
    setDetectingMpv(true);
    try {
      const foundPath = await autoDetectMpv();
      if (foundPath) {
        setConfig({ ...config, mpv_path: foundPath });
        toast({
          title: "MPV Found",
          description: `Detected at: ${foundPath}`,
        });
      } else {
        toast({
          title: "MPV Not Found",
          description:
            "Could not find mpv on your system. Please install MPV or set the path manually.",
          variant: "destructive",
        });
      }
    } catch (error) {
      console.error("Failed to auto-detect MPV:", error);
      toast({
        title: "Detection Failed",
        description: "An error occurred while searching for MPV.",
        variant: "destructive",
      });
    } finally {
      setDetectingMpv(false);
    }
  };

  return (
    <>
    <Dialog open={open} onOpenChange={onOpenChange}>
      <LazyMotion features={domAnimation}>
        <DialogContent className="!flex max-w-4xl max-h-[85vh] p-0 gap-0 flex-col overflow-hidden">
          <div className="flex flex-1 min-h-0 pr-14">
            {/* Sidebar */}
            <div className="w-40 sm:w-48 md:w-56 flex-shrink-0 bg-card/50 border-r border-border p-3 sm:p-4 overflow-y-auto">
              <div className="flex items-center justify-between mb-6">
                <h2 className="text-lg font-semibold text-foreground">
                  Settings
                </h2>
                <button
                  type="button"
                  onClick={() => onOpenChange(false)}
                  className="p-1.5 rounded-lg text-muted-foreground hover:text-foreground hover:bg-muted transition-colors"
                  aria-label="Close settings"
                >
                  <X className="size-4" />
                </button>
              </div>

              <nav className="space-y-1">
                {sections.map((section) => (
                  <button
                    type="button"
                    key={section.id}
                    onClick={() => setActiveSection(section.id)}
                    className={cn(
                      "w-full flex items-center gap-2 sm:gap-3 px-2 sm:px-3 py-2 sm:py-2.5 rounded-xl transition-all duration-200 text-left",
                      activeSection === section.id
                        ? "bg-white/10 text-white"
                        : "text-muted-foreground hover:text-foreground hover:bg-muted/50",
                    )}
                    aria-label={`${section.label} settings section`}
                  >
                    {section.icon}
                    <span className="text-xs sm:text-sm font-medium truncate">
                      {section.label}
                    </span>
                  </button>
                ))}
              </nav>
            </div>

            {/* Content */}
            <div className="flex-1 flex flex-col min-h-0 min-w-0">
              {/* Content Area */}
              <div className="flex-1 overflow-y-auto p-4 sm:p-6 min-h-0">
                <AnimatePresence mode="wait">
                  {/* General Section */}
                  {/* ===== General Settings ===== */}
                  {activeSection === "general" && (
                    <m.div
                      key="general"
                      initial={{ opacity: 0, y: 10 }}
                      animate={{ opacity: 1, y: 0 }}
                      exit={{ opacity: 0, y: -10 }}
                      className="space-y-6"
                    >
                      <div>
                        <h3 className="text-lg font-semibold text-foreground mb-1">
                          General Settings
                        </h3>
                        <p className="text-sm text-muted-foreground">
                          Configure general app behavior
                        </p>
                      </div>

                      {/* Auto Start */}
                      <div className="p-4 rounded-xl bg-card border border-border">
                        <div className="flex items-center justify-between">
                          <div className="flex items-center gap-3">
                            <div className="p-2 rounded-lg bg-white/10">
                              <Power className="size-5 text-white" />
                            </div>
                            <div>
                              <Label className="text-base font-medium">
                                Run on Startup
                              </Label>
                              <p className="text-sm text-muted-foreground">
                                Automatically start SlasshyVault when you log in
                              </p>
                            </div>
                          </div>
                          <Switch
                            checked={autoStart}
                            onCheckedChange={toggleAutoStart}
                          />
                        </div>
                      </div>

                      {/* MPV Player */}
                      <div className="p-4 rounded-xl bg-card border border-border space-y-3">
                        <div className="flex items-center gap-3">
                          <div className="p-2 rounded-lg bg-white/8">
                            <MonitorPlay className="size-5 text-foreground" />
                          </div>
                          <div>
                            <Label className="text-base font-medium">
                              MPV Player
                            </Label>
                            <p className="text-sm text-muted-foreground">
                              External MPV player (default)
                            </p>
                          </div>
                        </div>

                        <div className="space-y-3">
                        {/* Bundled Player — the hero */}
                        <div className={cn(
                          "rounded-xl border transition-all overflow-hidden",
                          config.mpv_path && bundledMpvInfo?.exists && config.mpv_path === bundledMpvInfo.path
                            ? "border-white/10 bg-white/5"
                            : "border-border/50 bg-muted/30"
                        )}>
                          <div className="p-4">
                            <div className="flex items-center justify-between">
                              <div className="flex items-center gap-3">
                                <div className={cn(
                                  "p-2 rounded-lg",
                                  bundledMpvInfo?.exists ? "bg-white/10" : "bg-muted"
                                )}>
                                  <Wifi className={cn(
                                    "size-5",
                                    bundledMpvInfo?.exists ? "text-foreground" : "text-muted-foreground"
                                  )} />
                                </div>
                                <div>
                                  <div className="flex items-center gap-2">
                                    <p className="text-sm font-medium">
                                      Bundled Player
                                    </p>
                                    <span className="px-1.5 py-0.5 text-[10px] font-semibold bg-white/10 text-foreground rounded-full tracking-wide">
                                      RECOMMENDED
                                    </span>
                                  </div>
                                  <p className={cn(
                                    "text-xs",
                                    bundledMpvInfo?.exists ? "text-foreground/70" : "text-muted-foreground"
                                  )}>
                                    {bundledMpvInfo?.exists
                                      ? "✓ Installed and ready to use"
                                      : isWindows
                                        ? "Not installed — click to set up"
                                        : "Install MPV with your Linux package manager"}
                                  </p>
                                </div>
                              </div>
                              <Button
                                variant={bundledMpvInfo?.exists ? "ghost" : "default"}
                                size="sm"
                                onClick={handleDownloadBundledMpv}
                                disabled={downloadingBundledMpv || !isWindows}
                                className={cn(
                                  "gap-1.5 text-xs h-8 shrink-0",
                                  bundledMpvInfo?.exists && "text-muted-foreground hover:text-foreground"
                                )}
                              >
                                {downloadingBundledMpv ? (
                                  <>
                                    <Loader2 className="size-3 animate-spin" />
                                    {bundledMpvProgress > 0
                                      ? `${Math.round(bundledMpvProgress)}%`
                                      : "Installing…"}
                                  </>
                                ) : bundledMpvInfo?.exists ? (
                                  "Reinstall"
                                ) : (
                                  isWindows ? "Install" : "System MPV"
                                )}
                              </Button>
                            </div>

                            {/* Warning when bundled not actively used */}
                            {bundledMpvInfo?.exists && config.mpv_path && config.mpv_path !== bundledMpvInfo.path && (
                              <div className="flex items-start gap-2 mt-3 p-2.5 rounded-lg bg-amber-500/10 border border-amber-500/20">
                                <AlertTriangle className="size-4 text-amber-400 shrink-0 mt-0.5" />
                                <p className="text-[11px] text-amber-300/90 leading-relaxed">
                                  You're using a different MPV build. Newer builds can cause
                                  playback errors. Switch back to the bundled player above.
                                </p>
                              </div>
                            )}
                          </div>
                        </div>

                        {/* Custom path — hidden behind a toggle */}
                        <div>
                          <button
                            type="button"
                            onClick={() => setShowCustomMpv(!showCustomMpv)}
                            className="flex items-center gap-1.5 text-xs text-muted-foreground hover:text-foreground transition-colors"
                          >
                            {showCustomMpv ? "▼" : "▶"} {showCustomMpv ? "Hide" : "Use a different player"}
                          </button>

                          {showCustomMpv && (
                            <div className="mt-3 p-3 rounded-xl bg-red-500/5 border border-red-500/20 space-y-3">
                              <div className="flex items-start gap-2">
                                <AlertTriangle className="size-4 text-red-400 shrink-0 mt-0.5" />
                                <div>
                                  <p className="text-xs font-semibold text-red-300">
                                    Not recommended
                                  </p>
                                  <p className="text-[11px] text-red-300/70 leading-relaxed">
                                    Changing the MPV player can break video playback.
                                    Only do this if you're absolutely sure you need
                                    a different build.
                                  </p>
                                </div>
                              </div>
                              <div className="flex gap-2">
                                <div className="flex-1 relative">
                                  <Input
                                    value={config.mpv_path || ""}
                                    onChange={(e) => {
                                      setConfig({ ...config, mpv_path: e.target.value });
                                      validatePath(e.target.value, "mpv_path");
                                    }}
                                    placeholder={isWindows ? "C:\\path\\to\\mpv.exe" : "/usr/bin/mpv"}
                                    className="flex-1 text-xs"
                                    aria-label="Custom MPV executable path"
                                    aria-invalid={!!pathValidation.mpv_path}
                                  />
                                  {pathValidation.mpv_path && (
                                    <p className="text-xs text-destructive mt-1">{pathValidation.mpv_path}</p>
                                  )}
                                </div>
                                <Button
                                  variant="outline"
                                  size="icon"
                                  onClick={browseMpvPath}
                                  title="Browse"
                                  className="shrink-0"
                                  aria-label="Browse for MPV executable"
                                >
                                  <FolderOpen className="size-4" />
                                </Button>
                                <Button
                                  variant="outline"
                                  onClick={handleAutoDetectMpv}
                                  disabled={detectingMpv}
                                  className="gap-2 shrink-0 text-xs"
                                  title="Auto-detect MPV on your PC"
                                >
                                  <RefreshCw
                                    className={cn(
                                      "size-3",
                                      detectingMpv && "animate-spin",
                                    )}
                                  />
                                  {detectingMpv ? "Detecting…" : "Detect"}
                                </Button>
                              </div>
                            </div>
                          )}
                        </div>
                        </div>
                      </div>


                    </m.div>
                   )}

                  {/* ===== Beta Features ===== */}
                  {activeSection === "beta" && (
                    <m.div
                      key="beta"
                      initial={{ opacity: 0, y: 10 }}
                      animate={{ opacity: 1, y: 0 }}
                      exit={{ opacity: 0, y: -10 }}
                      className="space-y-6"
                    >
                      <div>
                        <div className="flex items-center gap-2">
                          <h3 className="text-lg font-semibold text-foreground mb-1">
                            Experimental Features
                          </h3>
                          <span className="px-1.5 py-0.5 text-[10px] font-medium bg-purple-500/20 text-purple-400 rounded-full">
                            EXPERIMENTAL
                          </span>
                        </div>
                        <p className="text-sm text-muted-foreground">
                          Beta features are testable. Unstable features may be incomplete, paused, or not usable yet.
                        </p>
                      </div>

                      {/* Master Beta Toggle */}
                      <div className="p-4 rounded-xl bg-card border border-purple-500/30">
                        <div className="flex items-center justify-between">
                          <div className="flex items-center gap-3">
                            <div className="p-2 rounded-lg bg-purple-500/20">
                              <FlaskConical className="size-5 text-purple-400" />
                            </div>
                            <div>
                              <div className="flex items-center gap-2">
                                <Label className="text-base font-medium">
                                  Enable Beta Features
                                </Label>
                                <span
                                  className={cn(
                                    "px-1.5 py-0.5 text-[10px] font-semibold rounded",
                                    betaEnabled
                                      ? "bg-green-500/20 text-green-400"
                                      : "bg-muted text-muted-foreground",
                                  )}
                                >
                                  {betaEnabled ? "ON" : "OFF"}
                                </span>
                              </div>
                              <p className="text-sm text-muted-foreground">
                                Toggle all beta features on or off
                              </p>
                            </div>
                          </div>
                          <Switch
                            checked={betaEnabled}
                            onCheckedChange={(checked) => {
                              if (checked) {
                                setShowBetaConfirm(true);
                              } else {
                                onBetaToggle?.(false);
                              }
                            }}
                          />
                        </div>
                      </div>

                      {/* Warning Banner */}
                      <div className="p-3 rounded-lg bg-yellow-500/10 border border-yellow-500/20">
                        <div className="flex items-start gap-2">
                          <AlertTriangle className="size-4 text-yellow-500 mt-0.5 flex-shrink-0" />
                          <div className="space-y-1">
                            <p className="text-xs font-medium text-yellow-500">
                              Heads up
                            </p>
                            <p className="text-xs text-yellow-500/70">
                              Beta features are meant for public testing.
                              Unstable features are earlier than beta and may be
                              paused, incomplete, or unavailable at any time.
                            </p>
                          </div>
                        </div>
                      </div>

                      {/* Feature List */}
                      <div className="space-y-3">
                        <Label className="text-sm font-medium text-muted-foreground uppercase tracking-wider">
                          Beta Features
                        </Label>
                        {/* Watch Together */}
                        <div
                          className={cn(
                            "p-4 rounded-xl border transition-colors",
                            betaEnabled
                              ? "bg-card border-purple-500/20"
                              : "bg-card/50 border-border opacity-60",
                          )}
                        >
                          <div className="flex items-start gap-3">
                            <div
                              className={cn(
                                "p-2 rounded-lg flex-shrink-0",
                                betaEnabled ? "bg-purple-500/20" : "bg-muted",
                              )}
                            >
                              <Radio
                                className={cn(
                                  "size-5",
                                  betaEnabled
                                    ? "text-purple-400"
                                    : "text-muted-foreground",
                                )}
                              />
                            </div>
                            <div className="flex-1 min-w-0">
                              <div className="flex items-center gap-2 mb-1">
                                <span className="text-sm font-medium">
                                  Watch Together
                                </span>
                                <span className="px-1.5 py-0.5 text-[10px] font-medium bg-purple-500/20 text-purple-400 rounded">
                                  BETA
                                </span>
                              </div>
                              <p className="text-xs text-muted-foreground">
                                Watch movies and shows in sync with friends.
                                Create or join rooms for synchronized playback.
                              </p>
                            </div>
                          </div>
                        </div>

                      </div>

                    </m.div>
                  )}

                  {/* ===== Account ===== */}
                  {activeSection === "account" && (
                    <m.div
                      key="account"
                      initial={{ opacity: 0, y: 10 }}
                      animate={{ opacity: 1, y: 0 }}
                      exit={{ opacity: 0, y: -10 }}
                      className="space-y-6"
                    >
                      <div>
                        <h3 className="text-lg font-semibold text-foreground mb-1">
                          Account
                        </h3>
                        <p className="text-sm text-muted-foreground">
                          Manage your Google account connection
                        </p>
                      </div>

                      {/* Google Drive connection card */}
                      <GoogleDriveSettings />

                      {/* Logout */}
                      <div className="p-4 rounded-xl bg-card border border-red-500/30 space-y-4">
                        <div className="flex items-center gap-3">
                          <div className="p-2 rounded-lg bg-red-500/20">
                            <Power className="size-5 text-red-400" />
                          </div>
                          <div>
                            <p className="text-base font-medium">
                              Sign Out
                            </p>
                            <p className="text-sm text-muted-foreground">
                              Disconnect your Google account and clear all stored data
                            </p>
                          </div>
                        </div>
                        <p className="text-xs text-muted-foreground">
                          This will sign you out of SlasshyVault, disconnect your Google Drive,
                          and clear all locally stored tokens. You'll need to sign in again
                          to access your library.
                        </p>

                        {!showLogoutConfirm ? (
                          <Button
                            variant="destructive"
                            onClick={() => setShowLogoutConfirm(true)}
                            className="w-full"
                          >
                            <Power className="mr-2 size-4" />
                            Sign Out
                          </Button>
                        ) : (
                          <div className="space-y-3 p-4 rounded-lg bg-destructive/10 border border-destructive/30">
                            <p className="text-sm font-medium text-destructive text-center">
                              Are you sure you want to sign out? This will clear all
                              locally stored credentials.
                            </p>
                            <div className="flex gap-2">
                              <Button
                                variant="outline"
                                onClick={() => setShowLogoutConfirm(false)}
                                className="flex-1"
                                disabled={loggingOut}
                              >
                                Cancel
                              </Button>
                              <Button
                                variant="destructive"
                                onClick={async () => {
                                  setLoggingOut(true)
                                  try {
                                    if (onLogout) {
                                      onLogout()
                                      onOpenChange(false)
                                    }
                                  } finally {
                                    setLoggingOut(false)
                                    setShowLogoutConfirm(false)
                                  }
                                }}
                                className="flex-1"
                                disabled={loggingOut}
                              >
                                {loggingOut ? (
                                  <>
                                    <Loader2 className="size-4 mr-2 animate-spin" />
                                    Signing Out…
                                  </>
                                ) : (
                                  "Yes, Sign Out"
                                )}
                              </Button>
                            </div>
                          </div>
                        )}
                      </div>
                    </m.div>
                  )}

                  {/* ===== Updates & Security ===== */}
                  {activeSection === "updates" && (
                    <m.div
                      key="updates"
                      initial={{ opacity: 0, y: 10 }}
                      animate={{ opacity: 1, y: 0 }}
                      exit={{ opacity: 0, y: -10 }}
                      className="space-y-6"
                    >
                      <div>
                        <h3 className="text-lg font-semibold text-foreground mb-1">
                          Updates & Security
                        </h3>
                        <p className="text-sm text-muted-foreground">
                          App updates, what's new, and version info
                        </p>
                      </div>

                      {/* About & Updates */}
                      <div className="p-4 rounded-xl bg-card border border-border space-y-4">
                        <div className="flex items-center gap-3 mb-2">
                          <div className="p-2 rounded-lg bg-white/10">
                            <Download className="size-5 text-white" />
                          </div>
                          <div>
                            <Label className="text-base font-medium">
                              About & Updates
                            </Label>
                            <p className="text-sm text-muted-foreground">
                              Version {appVersion || "…"}
                            </p>
                          </div>
                        </div>

                        {/* Check for Updates Button */}
                        {!updateInfo?.available && (
                          <Button
                            variant="outline"
                            onClick={handleCheckUpdate}
                            disabled={checkingUpdate}
                            className="w-full gap-2"
                          >
                            <RefreshCw
                              className={cn(
                                "size-4",
                                checkingUpdate && "animate-spin",
                              )}
                            />
                            {checkingUpdate
                              ? "Checking…"
                              : "Check for Updates"}
                          </Button>
                        )}

                        {/* Update Available */}
                        {updateInfo?.available && (
                          <div className="space-y-3 p-3 rounded-lg bg-white/10 border border-white/20">
                            <div className="flex items-center justify-between">
                              <span className="text-sm font-medium text-white">
                                Update Available: v{updateInfo.latest_version}
                              </span>
                              {updateInfo.published_at && (
                                <span className="text-xs text-muted-foreground">
                                  {new Date(
                                    updateInfo.published_at,
                                  ).toLocaleDateString()}
                                </span>
                              )}
                            </div>

                            {updateInfo.release_notes && (
                              <div className="text-xs text-muted-foreground max-h-24 overflow-y-auto">
                                <p className="whitespace-pre-wrap">
                                  {updateInfo.release_notes}
                                </p>
                              </div>
                            )}

                            {downloadingUpdate ? (
                              <div className="space-y-2">
                                <div className="w-full bg-muted rounded-full h-2">
                                  <div
                                    className="bg-white h-2 rounded-full transition-all duration-300"
                                    style={{ width: `${downloadProgress}%` }}
                                  />
                                </div>
                                <p className="text-xs text-center text-muted-foreground">
                                  Downloading… {downloadProgress.toFixed(0)}%
                                </p>
                              </div>
                            ) : (
                              <Button
                                onClick={handleDownloadAndInstall}
                                disabled={!updateInfo.download_url}
                                className="w-full gap-2 bg-white text-black hover:bg-gray-200"
                              >
                                <Download className="size-4" />
                                Download & Install
                              </Button>
                            )}
                          </div>
                        )}
                      </div>
                    </m.div>
                  )}

                  {/* ===== Cache and Storage ===== */}
                  {activeSection === "cloud" && (
                    <m.div
                      key="cloud"
                      initial={{ opacity: 0, y: 10 }}
                      animate={{ opacity: 1, y: 0 }}
                      exit={{ opacity: 0, y: -10 }}
                      className="space-y-6"
                    >
                      <div className="p-4 rounded-xl bg-card border border-border space-y-4">
                        <div className="flex items-start gap-3">
                          <div className="p-2 rounded-lg bg-white/10">
                            <Archive className="size-5 text-white" />
                          </div>
                          <div className="flex-1">
                            <Label className="text-base font-medium">
                              ZIP Archive Support
                            </Label>
                            <p className="text-sm text-muted-foreground">
                              Index TV episodes directly from Google Drive ZIP
                              archives and keep extracted playback cache under
                              control.
                            </p>
                          </div>
                          <Switch
                            checked={config.zip_indexing_enabled ?? true}
                            onCheckedChange={(checked) =>
                              setConfig({
                                ...config,
                                zip_indexing_enabled: checked,
                              })
                            }
                          />
                        </div>

                        <div className="grid gap-4 md:grid-cols-2">
                          <div className="space-y-2 md:col-span-2">
                            <Label>ZIP Cache Directory</Label>
                            <div className="flex gap-2">
                              <Input
                                value={config.zip_cache_dir || ""}
                                onChange={(e) =>
                                  setConfig({
                                    ...config,
                                    zip_cache_dir: e.target.value,
                                  })
                                }
                                placeholder="Default app cache location"
                                className="flex-1"
                              />
                              <Button
                                variant="outline"
                                size="icon"
                                onClick={browseZipCacheDir}
                                title="Browse"
                              >
                                <FolderOpen className="size-4" />
                              </Button>
                            </div>
                            <p className="text-xs text-muted-foreground">
                              Pick a different drive if you want ZIP extraction
                              cache stored outside the default app data folder.
                            </p>
                          </div>

                          <div className="space-y-2">
                            <Label>ZIP Cache Size Limit (GB)</Label>
                            <Input
                              type="number"
                              min={1}
                              max={500}
                              value={config.zip_cache_max_gb ?? 20}
                              onChange={(e) =>
                                setConfig({
                                  ...config,
                                  zip_cache_max_gb: Math.max(
                                    1,
                                    Number(e.target.value) || 1,
                                  ),
                                })
                              }
                            />
                            <p className="text-xs text-muted-foreground">
                              Older ZIP cache files will be replaced first when
                              the limit is reached.
                            </p>
                          </div>

                          <div className="space-y-2">
                            <Label>ZIP Cache Expiry (Days)</Label>
                            <Input
                              type="number"
                              min={1}
                              max={365}
                              value={config.zip_cache_expiry_days ?? 7}
                              onChange={(e) =>
                                setConfig({
                                  ...config,
                                  zip_cache_expiry_days: Math.max(
                                    1,
                                    Number(e.target.value) || 1,
                                  ),
                                })
                              }
                            />
                            <p className="text-xs text-muted-foreground">
                              Unused ZIP cache files older than this will be
                              removed automatically.
                            </p>
                          </div>
                        </div>
                      </div>
                    </m.div>
                  )}

                  {/* ===== Storage & Bandwidth ===== */}
                  {activeSection === "storage" && (
                    <m.div
                      key="storage"
                      initial={{ opacity: 0, y: 10 }}
                      animate={{ opacity: 1, y: 0 }}
                      exit={{ opacity: 0, y: -10 }}
                      className="space-y-6"
                    >
                      <div>
                        <h3 className="text-lg font-semibold text-foreground mb-1">
                          Storage & Bandwidth
                        </h3>
                        <p className="text-sm text-muted-foreground">
                          Monitor storage usage, cache sizes, and download activity
                        </p>
                      </div>

                      {storageLoading ? (
                        <div className="flex items-center justify-center py-12">
                          <Loader2 className="size-6 animate-spin text-muted-foreground" />
                        </div>
                      ) : (
                        <>
                          {/* Google Drive Storage */}
                          <div className="p-4 rounded-xl bg-card border border-border space-y-3">
                            <div className="flex items-center gap-3">
                              <div className="p-2 rounded-lg bg-white/10">
                                <Cloud className="size-5 text-white" />
                              </div>
                              <div>
                                <Label className="text-base font-medium">Google Drive Storage</Label>
                                {driveInfo && (
                                  <p className="text-xs text-muted-foreground">{driveInfo.email}</p>
                                )}
                              </div>
                            </div>
                            {driveInfo?.storage_used != null && driveInfo?.storage_limit != null ? (
                              <div className="space-y-2">
                                <div className="flex justify-between text-sm">
                                  <span className="text-muted-foreground">Used</span>
                                  <span className="font-medium">
                                    {(driveInfo.storage_used / (1024 ** 3)).toFixed(1)} GB / {(driveInfo.storage_limit / (1024 ** 3)).toFixed(0)} GB
                                  </span>
                                </div>
                                <div className="h-2 rounded-full bg-muted overflow-hidden">
                                  <div
                                    className="h-full rounded-full bg-white/70 transition-all"
                                    style={{ width: `${Math.min(100, (driveInfo.storage_used / driveInfo.storage_limit) * 100)}%` }}
                                  />
                                </div>
                                <p className="text-xs text-muted-foreground text-right">
                                  {((driveInfo.storage_used / driveInfo.storage_limit) * 100).toFixed(1)}% used
                                </p>
                              </div>
                            ) : (
                              <p className="text-sm text-muted-foreground">Connect Google Drive to view storage stats.</p>
                            )}
                          </div>

                          {/* Cloud Cache */}
                          <div className="p-4 rounded-xl bg-card border border-border space-y-3">
                            <div className="flex items-center gap-3">
                              <div className="p-2 rounded-lg bg-white/10">
                                <HardDrive className="size-5 text-white" />
                              </div>
                              <div className="flex-1">
                                <Label className="text-base font-medium">Cloud Cache</Label>
                                <p className="text-xs text-muted-foreground">
                                  {cacheInfo?.enabled ? `${cacheInfo.file_count} files, ${cacheInfo.total_size_mb.toFixed(1)} MB used` : "Disabled"}
                                </p>
                              </div>
                              <Button
                                variant="outline"
                                size="sm"
                                onClick={async () => {
                                  try {
                                    const res = await cleanupCloudCache();
                                    toast({ title: "Cache cleaned", description: res.message });
                                    const fresh = await getCloudCacheInfo();
                                    setCacheInfo(fresh);
                                  } catch { toast({ title: "Cleanup failed", variant: "destructive" }); }
                                }}
                              >
                                <RefreshCw className="size-3 mr-1.5" /> Cleanup
                              </Button>
                              <Button
                                variant="destructive"
                                size="sm"
                                onClick={async () => {
                                  try {
                                    const res = await clearCloudCache();
                                    toast({ title: "Cache cleared", description: res.message });
                                    const fresh = await getCloudCacheInfo();
                                    setCacheInfo(fresh);
                                  } catch { toast({ title: "Clear failed", variant: "destructive" }); }
                                }}
                              >
                                <Trash2 className="size-3 mr-1.5" /> Clear
                              </Button>
                            </div>
                            {cacheInfo?.enabled && (
                              <div className="h-2 rounded-full bg-muted overflow-hidden">
                                <div
                                  className="h-full rounded-full bg-white/50 transition-all"
                                  style={{ width: `${Math.min(100, (cacheInfo.total_size_mb / cacheInfo.max_size_mb) * 100)}%` }}
                                />
                              </div>
                            )}
                          </div>

                          {/* Top Space Consumers */}
                          {topMedia.length > 0 && (
                            <div className="p-4 rounded-xl bg-card border border-border space-y-3">
                              <div className="flex items-center gap-3">
                                <div className="p-2 rounded-lg bg-white/10">
                                  <Archive className="size-5 text-white" />
                                </div>
                                <Label className="text-base font-medium">Largest Library Items</Label>
                              </div>
                              <div className="space-y-1">
                                {topMedia.map((item) => (
                                  <div key={item.id} className="flex items-center justify-between py-1.5 px-2 rounded-lg hover:bg-muted/30">
                                    <div className="flex-1 min-w-0">
                                      <p className="text-sm truncate">{item.title}</p>
                                      <p className="text-xs text-muted-foreground">
                                        {item.media_type === "tvshow" ? "TV Show" : "Movie"}{item.year ? ` (${item.year})` : ""}
                                      </p>
                                    </div>
                                    <span className="text-sm font-mono text-muted-foreground ml-3">
                                      {item.file_size_bytes ? `${(item.file_size_bytes / (1024 ** 3)).toFixed(2)} GB` : "—"}
                                    </span>
                                  </div>
                                ))}
                              </div>
                            </div>
                          )}

                          {/* Recent Downloads */}
                          {recentDownloads.length > 0 && (
                            <div className="p-4 rounded-xl bg-card border border-border space-y-3">
                              <div className="flex items-center gap-3">
                                <div className="p-2 rounded-lg bg-white/10">
                                  <Download className="size-5 text-white" />
                                </div>
                                <Label className="text-base font-medium">Recent Downloads</Label>
                              </div>
                              <div className="space-y-1">
                                {recentDownloads.map((job) => (
                                  <div key={job.id} className="flex items-center justify-between py-1.5 px-2 rounded-lg hover:bg-muted/30">
                                    <div className="flex-1 min-w-0">
                                      <p className="text-sm truncate">{job.title}</p>
                                      <p className="text-xs text-muted-foreground">
                                        {job.status === "completed" ? "Completed" : `${(job.progress * 100).toFixed(0)}%`}
                                      </p>
                                    </div>
                                    {job.speedBytesPerSecond != null && job.speedBytesPerSecond > 0 && (
                                      <span className="text-xs font-mono text-muted-foreground ml-3">
                                        {(job.speedBytesPerSecond / (1024 ** 2)).toFixed(1)} MB/s
                                      </span>
                                    )}
                                  </div>
                                ))}
                              </div>
                            </div>
                          )}
                        </>
                      )}
                    </m.div>
                  )}

                  {/* ===== API Configuration ===== */}
                  {activeSection === "api" && (
                    <m.div
                      key="api"
                      initial={{ opacity: 0, y: 10 }}
                      animate={{ opacity: 1, y: 0 }}
                      exit={{ opacity: 0, y: -10 }}
                      className="space-y-6"
                    >
                      <div>
                        <h3 className="text-lg font-semibold text-foreground mb-1">
                          API Configuration
                        </h3>
                        <p className="text-sm text-muted-foreground">
                          Configure external service API keys
                        </p>
                      </div>

                      {/* API Keys */}
                      <div className="p-4 rounded-xl bg-card border border-border space-y-4">
                        <div className="flex items-center gap-3">
                          <div className="p-2 rounded-lg bg-white/10">
                            <Zap className="size-5 text-white" />
                          </div>
                          <div>
                            <Label className="text-base font-medium">
                              API Keys
                            </Label>
                              <p className="text-sm text-muted-foreground">
                                TMDB (metadata, optional) and OMDb (IMDb ratings, optional)
                              </p>
                            </div>
                        </div>

                        {/* API Keys info */}
                        <div className="p-3 rounded-lg bg-blue-500/10 border border-blue-500/20">
                          <p className="text-xs text-blue-300 leading-relaxed">
                            TMDB is <strong>optional</strong>. Without one, metadata queries fall back to
                            Cinemeta (free, Stremio catalog) then to{" "}
                            <a
                              className="underline"
                              href="https://api.balloonerismm.workers.dev"
                              target="_blank"
                              rel="noopener noreferrer"
                            >
                              Balloonerismm
                            </a>{" "}
                            (free, no-key TMDB-shaped mirror) for posters, episode listings, cast,
                            and trending. IMDb ratings also use OMDb by default — OMDb is only needed for
                            dedicated rate limits.
                          </p>
                        </div>

                        {/* Custom API Key Inputs */}
                        <div className="space-y-4">
                          <m.div
                            initial={{ opacity: 0, height: 0 }}
                            animate={{ opacity: 1, height: "auto" }}
                            exit={{ opacity: 0, height: 0 }}
                            className="space-y-4 pt-1"
                          >
                            {/* TMDB Key */}
                            <div>
                              <label htmlFor="tmdb-api-key" className="text-xs font-bold text-white/60 uppercase tracking-wider mb-1.5 block">
                                TMDB API Key <span className="text-white/40 font-normal">(optional)</span>
                              </label>
                              <Input
                                id="tmdb-api-key"
                                type="password"
                                value={config.tmdb_api_key || ""}
                                onChange={(e) =>
                                  setConfig({
                                    ...config,
                                    tmdb_api_key: e.target.value,
                                  })
                                }
                                placeholder="Leave blank to use Cinemeta + Balloonerismm (free)"
                              />
                              <p className="text-xs text-muted-foreground mt-1.5">
                                Recommended for richer metadata and trending suggestions.
                                Without a key, the app falls back to the free{" "}
                                <a
                                  href="https://api.balloonerismm.workers.dev"
                                  target="_blank"
                                  rel="noopener noreferrer"
                                  className="text-white hover:underline"
                                >
                                  Balloonerismm
                                </a>{" "}
                                service. Get yours at{" "}
                                <a
                                  href="https://www.themoviedb.org/settings/api"
                                  target="_blank"
                                  rel="noopener noreferrer"
                                  className="text-white hover:underline"
                                >
                                  themoviedb.org
                                </a>.
                              </p>
                            </div>

                            {/* OMDb Key */}
                            <div>
                              <label htmlFor="omdb-api-key" className="text-xs font-bold text-white/60 uppercase tracking-wider mb-1.5 block">
                                OMDb API Key (IMDb Ratings)
                              </label>
                              <Input
                                id="omdb-api-key"
                                type="password"
                                value={config.omdb_api_key || ""}
                                onChange={(e) =>
                                  setConfig({
                                    ...config,
                                    omdb_api_key: e.target.value,
                                  })
                                }
                                placeholder="Enter your OMDb API key"
                              />
                              <p className="text-xs text-muted-foreground mt-1.5">
                                Used for fetching IMDb ratings for episodes. Get yours at{" "}
                                <a
                                  href="https://www.omdbapi.com/apikey.aspx"
                                  target="_blank"
                                  rel="noopener noreferrer"
                                  className="text-white hover:underline"
                                >
                                  omdbapi.com
                                </a>
                              </p>
                            </div>
                          </m.div>
                        </div>
                      </div>
                    </m.div>
                  )}

                  {/* ===== Hidden section when dev panel selected ===== */}
                  {activeSection === "dev" && import.meta.env.DEV && (
                    <m.div
                      key="dev"
                      initial={{ opacity: 0, y: 10 }}
                      animate={{ opacity: 1, y: 0 }}
                      exit={{ opacity: 0, y: -10 }}
                      className="space-y-6"
                    >
                      {/* Test ZIP notification flow */}
                      <div className="p-4 rounded-xl bg-card border border-border space-y-3">
                        <div className="flex items-center gap-2">
                          <h4 className="text-sm font-medium">Test ZIP Notifications</h4>
                          <span className="px-1.5 py-0.5 text-[10px] font-medium bg-yellow-500/20 text-yellow-400 rounded-full">
                            DEV ONLY
                          </span>
                        </div>
                        <p className="text-xs text-muted-foreground">
                          Simulate ZIP detection and indexing events to test the notification popup.
                        </p>
                        <div className="flex gap-2">
                          <Button
                            variant="outline"
                            size="sm"
                            className="flex-1"
                            onClick={() => {
                              emit('zip-processing-status', {
                                phase: 'detected',
                                archiveCount: 1,
                                archiveName: 'Test Archive.zip',
                                episodesIndexed: null,
                                message: 'Archive detected in Test Folder. Processing episode entries...',
                              })
                            }}
                          >
                            ZIP Detected
                          </Button>
                          <Button
                            variant="outline"
                            size="sm"
                            className="flex-1"
                            onClick={() => {
                              emit('zip-processing-status', {
                                phase: 'complete',
                                archiveCount: 1,
                                archiveName: 'Test Archive.zip',
                                episodesIndexed: 12,
                                message: 'Finished processing Test Archive.zip. Indexed 12 episode(s).',
                              })
                            }}
                          >
                            ZIP Complete
                          </Button>
                          <Button
                            variant="outline"
                            size="sm"
                            className="flex-1"
                            onClick={() => {
                              emit('zip-processing-status', {
                                phase: 'error',
                                archiveCount: 1,
                                archiveName: 'Test Archive.zip',
                                episodesIndexed: null,
                                message: 'ZIP processing failed: Unsupported format',
                              })
                            }}
                          >
                            ZIP Error
                          </Button>
                        </div>
                      </div>

                    </m.div>
                  )}

                  {/* ===== Nightly Section (only shown in nightly builds) ===== */}
                  {import.meta.env.VITE_IS_NIGHTLY === 'true' && activeSection === "nightly" && (
                    <m.div
                      key="nightly"
                      initial={{ opacity: 0, y: 10 }}
                      animate={{ opacity: 1, y: 0 }}
                      exit={{ opacity: 0, y: -10 }}
                      className="space-y-6"
                    >
                      <div>
                        <div className="flex items-center gap-2">
                          <h3 className="text-lg font-semibold text-foreground mb-1">
                            Nightly Build Options
                          </h3>
                          <span className="px-1.5 py-0.5 text-[10px] font-medium bg-orange-500/20 text-orange-400 rounded-full">
                            NIGHTLY
                          </span>
                        </div>
                        <p className="text-sm text-muted-foreground">
                          Developer tools and diagnostics for nightly builds
                        </p>
                      </div>

                      {/* Developer Console Toggle */}
                      <div className="p-4 rounded-xl bg-card border border-border">
                        <div className="flex items-center justify-between">
                          <div className="flex items-center gap-3">
                            <div className="p-2 rounded-lg bg-white/10">
                              <Bug className="size-5 text-white" />
                            </div>
                            <div>
                              <Label className="text-base font-medium">
                                Show Developer Console
                              </Label>
                              <p className="text-sm text-muted-foreground">
                                Opens a floating console overlay showing frontend and backend logs
                              </p>
                            </div>
                          </div>
                          <Switch
                            checked={showDevConsole}
                            onCheckedChange={(checked) => {
                              setShowDevConsole(checked);
                              localStorage.setItem("slasshyvault_show_dev_console", String(checked));
                              if (checked) {
                                toast({
                                  title: "Developer Console Enabled",
                                  description: "Click the 'Console' button at the bottom-right corner to open it.",
                                });
                              }
                            }}
                          />
                        </div>
                      </div>

                      {/* Info card */}
                      <div className="p-3 rounded-lg bg-yellow-500/10 border border-yellow-500/20">
                        <div className="flex items-start gap-2">
                          <AlertTriangle className="size-4 text-yellow-500 mt-0.5 flex-shrink-0" />
                          <div className="space-y-1">
                            <p className="text-xs font-medium text-yellow-500">
                              Nightly Build
                            </p>
                            <p className="text-xs text-yellow-500/70">
                              This is a pre-release build. Logs include verbose debug information
                              from the ZIP proxy cache, MPV playback, cloud scanning, and more.
                            </p>
                          </div>
                        </div>
                      </div>
                    </m.div>
                  )}

                  {/* ===== External Sources ===== */}
                  {activeSection === "external" && (
                    <m.div
                      key="external"
                      initial={{ opacity: 0, y: 8 }}
                      animate={{ opacity: 1, y: 0 }}
                      exit={{ opacity: 0, y: -8 }}
                      className="space-y-6"
                    >
                      <div>
                        <h2 className="text-lg font-semibold">External Sources</h2>
                        <p className="text-sm text-muted-foreground">
                          Manage addon sources for streaming content in the External tab.
                        </p>
                      </div>

                      {/* Addon Sources Manager */}
                      <AddonSourcesManager />

                      {/* Debrid Services */}
                      <DebridServicesPanel />

                      {/* Legacy single URL (collapsed) */}
                      <details className="group">
                        <summary className="text-xs text-muted-foreground cursor-pointer hover:text-foreground transition-colors">
                          Advanced: Manual addon URL override
                        </summary>
                        <div className="mt-3 p-4 rounded-xl bg-card border border-border space-y-3">
                          <div>
                            <Label htmlFor="addon-url" className="text-sm font-medium">
                              Addon URL
                            </Label>
                            <Input
                              id="addon-url"
                              type="url"
                              value={config.addon_url || ""}
                              onChange={(e) => setConfig({ ...config, addon_url: e.target.value })}
                              placeholder="https://your-addon-url.com"
                              className="mt-1"
                            />
                            <p className="text-xs text-muted-foreground mt-1">
                              Direct addon URL. Only used as fallback when no source is configured above.
                            </p>
                          </div>
                        </div>
                      </details>
                    </m.div>
                  )}

                  {/* ===== Watch Together Relay ===== */}
                  {activeSection === "relay" && (
                    <m.div
                      key="relay"
                      initial={{ opacity: 0, y: 10 }}
                      animate={{ opacity: 1, y: 0 }}
                      exit={{ opacity: 0, y: -10 }}
                      className="space-y-6"
                    >
                      <div>
                        <h3 className="text-lg font-semibold text-foreground mb-1">
                          Watch Together
                        </h3>
                        <p className="text-sm text-muted-foreground">
                          Configure your relay server for synchronized playback with friends.
                        </p>
                      </div>

                      {/* What is a relay */}
                      <div className="p-3 rounded-lg bg-muted/50 border border-border">
                        <p className="text-xs text-muted-foreground leading-relaxed">
                          Watch Together uses a Cloudflare Worker as a WebSocket relay to sync
                          playback between participants. You can deploy your own for free,
                          or enter an existing relay URL.
                        </p>
                      </div>

                      {/* Deploy or Manual Input */}
                      <div className="p-4 rounded-xl bg-card border border-border space-y-4">
                        {/* Deploy section */}
                        <div className="space-y-3">
                          <Label className="text-sm font-medium">Deploy to Cloudflare</Label>
                          <p className="text-xs text-muted-foreground">
                            One-click deploy a relay Worker to your Cloudflare account.
                          </p>
                          <div className="flex flex-col gap-2">
                            <Input
                              placeholder="Cloudflare API Token (Workers:Edit permission)"
                              value={config.together_cf_token || ""}
                              onChange={(e) => setConfig({ ...config, together_cf_token: e.target.value })}
                              type="password"
                            />
                            <div className="flex gap-2">
                              <Button
                                variant="outline"
                                size="sm"
                                className="flex-1"
                                onClick={async () => {
                                  if (!config.together_cf_token) return;
                                  try {
                                    const { listAccounts } = await import('@/services/cf-deploy');
                                    const accounts = await listAccounts(config.together_cf_token);
                                    if (accounts.length === 0) {
                                      toast({ title: "No accounts found", variant: "destructive" });
                                      return;
                                    }
                                    // Deploy to first account
                                    const { deployRelay } = await import('@/services/cf-deploy');
                                    const result = await deployRelay(config.together_cf_token, accounts[0].id);
                                    setConfig({
                                      ...config,
                                      together_relay_url: result.url,
                                      together_cf_account_id: accounts[0].id,
                                    });
                                    toast({ title: "Relay deployed!", description: result.url });
                                  } catch (e) {
                                    toast({ title: "Deploy failed", description: String(e), variant: "destructive" });
                                  }
                                }}
                                disabled={!config.together_cf_token}
                              >
                                <Zap className="size-4 mr-1" />
                                Deploy Relay
                              </Button>
                              <Button
                                variant="ghost"
                                size="sm"
                                onClick={async () => {
                                  if (!config.together_cf_token || !config.together_cf_account_id) return;
                                  try {
                                    const { deleteRelay } = await import('@/services/cf-deploy');
                                    await deleteRelay(config.together_cf_token, config.together_cf_account_id);
                                    setConfig({ ...config, together_relay_url: "", together_cf_account_id: "" });
                                    toast({ title: "Relay stopped" });
                                  } catch (e) {
                                    toast({ title: "Failed to stop", description: String(e), variant: "destructive" });
                                  }
                                }}
                                disabled={!config.together_cf_token || !config.together_cf_account_id}
                              >
                                Stop Relay
                              </Button>
                            </div>
                          </div>
                        </div>

                        <div className="border-t border-border pt-4">
                          <Label className="text-sm font-medium">Or enter existing relay URL</Label>
                          <div className="flex gap-2 mt-2">
                            <Input
                              placeholder="wss://your-relay.workers.dev"
                              value={config.together_relay_url || ""}
                              onChange={(e) => setConfig({ ...config, together_relay_url: e.target.value })}
                              className="font-mono text-xs"
                            />
                          </div>
                          <p className="text-xs text-muted-foreground mt-1.5">
                            Cloudflare Workers free tier: 100k req/day. Enough for many Watch Together sessions.
                          </p>
                        </div>
                      </div>
                    </m.div>
                  )}

                  {/* ===== Factory Reset (Danger Zone) ===== */}
                  {activeSection === "danger" && (
                    <m.div
                      key="danger"
                      initial={{ opacity: 0, y: 10 }}
                      animate={{ opacity: 1, y: 0 }}
                      exit={{ opacity: 0, y: -10 }}
                      className="space-y-6"
                    >
                      <div>
                        <h3 className="text-lg font-semibold text-foreground mb-1">
                          Factory Reset
                        </h3>
                        <p className="text-sm text-muted-foreground">
                          Danger zone - proceed with caution
                        </p>
                      </div>


                      {/* Reset App */}
                      <div className="p-4 rounded-xl border border-destructive/30 bg-destructive/5 space-y-4">
                        <div className="flex items-center gap-3">
                          <div className="p-2 rounded-lg bg-destructive/20">
                            <AlertTriangle className="size-5 text-destructive" />
                          </div>
                          <div>
                            <Label className="text-base font-medium text-destructive">
                              Reset Application
                            </Label>
                            <p className="text-sm text-muted-foreground">
                              Delete all data and start fresh
                            </p>
                          </div>
                        </div>
                        <p className="text-sm text-muted-foreground">
                          This will permanently delete your library data and
                          watch history, cached posters, and all settings. This
                          action cannot be undone.
                        </p>

                        {!showResetConfirm ? (
                          <Button
                            variant="destructive"
                            onClick={() => setShowResetConfirm(true)}
                            className="w-full"
                          >
                            <Trash2 className="mr-2 size-4" />
                            Reset App to Factory State
                          </Button>
                        ) : (
                          <div className="space-y-3 p-4 rounded-lg bg-destructive/10 border border-destructive/30">
                            <p className="text-sm font-medium text-destructive text-center">
                              Are you absolutely sure? This will delete
                              everything!
                            </p>
                            <div className="flex gap-2">
                              <Button
                                variant="outline"
                                onClick={() => setShowResetConfirm(false)}
                                className="flex-1"
                                disabled={resetting}
                              >
                                Cancel
                              </Button>
                              <Button
                                variant="destructive"
                                onClick={handleResetApp}
                                className="flex-1"
                                disabled={resetting}
                              >
                                {resetting
                                  ? "Resetting..."
                                  : "Yes, Delete Everything"}
                              </Button>
                            </div>
                          </div>
                        )}
                      </div>

                      {/* Delete All Media Files */}
                      <div className="p-4 rounded-xl border border-destructive/30 bg-destructive/5 space-y-4">
                        <div className="flex items-center gap-3">
                          <div className="p-2 rounded-lg bg-destructive/20">
                            <Trash2 className="size-5 text-destructive" />
                          </div>
                          <div>
                            <Label className="text-base font-medium text-destructive">
                              Delete All Media Files
                            </Label>
                            <p className="text-sm text-muted-foreground">
                              Remove all media from your library and cloud
                            </p>
                          </div>
                        </div>
                        <p className="text-sm text-muted-foreground">
                          This will permanently delete all SlasshyVault-managed
                          files from Google Drive and remove all media entries
                          (local and cloud) from your library. Watch history,
                          settings, and reminders will be preserved. This action
                          cannot be undone.
                        </p>

                        {deleteAllStep === 0 && (
                          <Button
                            variant="destructive"
                            onClick={() => setDeleteAllStep(1)}
                            className="w-full"
                          >
                            <Trash2 className="mr-2 size-4" />
                            Delete All Media Files
                          </Button>
                        )}

                        {deleteAllStep === 1 && (
                          <div className="space-y-3 p-4 rounded-lg bg-destructive/10 border border-destructive/30">
                            <p className="text-sm font-bold text-destructive text-center">
                              ⚠️ WARNING: PERMANENT DELETION
                            </p>
                            <ul className="text-sm text-muted-foreground space-y-1 list-disc list-inside">
                              <li>
                                All cloud files will be <strong>permanently deleted</strong> from Google Drive — they will <strong>NOT</strong> go to Trash
                              </li>
                              <li>
                                All media entries (local + cloud) will be removed from your library
                              </li>
                              <li>
                                Watch history, settings, and reminders will be preserved
                              </li>
                              <li>
                                This action is <strong>irreversible</strong>
                              </li>
                            </ul>
                            <div className="flex gap-2">
                              <Button
                                variant="outline"
                                onClick={() => setDeleteAllStep(0)}
                                className="flex-1"
                              >
                                Cancel
                              </Button>
                              <Button
                                variant="destructive"
                                onClick={() => setDeleteAllStep(2)}
                                className="flex-1"
                              >
                                I Understand, Continue
                              </Button>
                            </div>
                          </div>
                        )}

                        {deleteAllStep === 2 && (
                          <div className="space-y-3 p-4 rounded-lg bg-destructive/10 border border-destructive/30">
                            <p className="text-sm font-bold text-destructive text-center">
                              FINAL CONFIRMATION
                            </p>
                            <p className="text-sm text-muted-foreground text-center">
                              Type <strong>DELETE</strong> below to permanently erase all media files from Google Drive and your library.
                            </p>
                            <Input
                              placeholder='Type "DELETE" to confirm'
                              value={deleteAllConfirmText}
                              onChange={(e) => setDeleteAllConfirmText(e.target.value)}
                              className="text-center font-mono"
                              autoFocus
                            />
                            <div className="flex gap-2">
                              <Button
                                variant="outline"
                                onClick={() => {
                                  setDeleteAllStep(0);
                                  setDeleteAllConfirmText("");
                                }}
                                className="flex-1"
                                disabled={deletingAll}
                              >
                                Cancel
                              </Button>
                              <Button
                                variant="destructive"
                                onClick={handleDeleteAllMedia}
                                className="flex-1"
                                disabled={deletingAll || deleteAllConfirmText !== "DELETE"}
                              >
                                {deletingAll
                                  ? "Deleting..."
                                  : "Permanently Delete Everything"}
                              </Button>
                            </div>
                          </div>
                        )}
                      </div>

                      {/* Selective Delete */}
                      <div className="p-4 rounded-xl border border-orange-500/30 bg-orange-500/5 space-y-4">
                        <div className="flex items-center gap-3">
                          <div className="p-2 rounded-lg bg-orange-500/20">
                            <FolderOpen className="size-5 text-orange-400" />
                          </div>
                          <div>
                            <Label className="text-base font-medium text-orange-400">
                              Selective Delete
                            </Label>
                            <p className="text-sm text-muted-foreground">
                              Browse and choose specific files to delete
                            </p>
                          </div>
                        </div>
                        <p className="text-sm text-muted-foreground">
                          Browse your cloud folders, select individual files or
                          entire folders to permanently delete from Google Drive.
                          Files will NOT go to Trash.
                        </p>
                        <Button
                          variant="outline"
                          onClick={() => setShowSelectiveDelete(true)}
                          className="w-full border-orange-500/30 text-orange-400 hover:bg-orange-500/10"
                        >
                          <FolderOpen className="mr-2 size-4" />
                          Browse & Select Files to Delete
                        </Button>
                      </div>
                    </m.div>
                  )}

                </AnimatePresence>
              </div>
            </div>
          </div>

          {/* Footer - Always visible at bottom */}
          <div className="flex-shrink-0 p-3 sm:p-4 border-t border-border bg-card/50">
            <div className="flex justify-end gap-2 sm:gap-3">
              <Button
                variant="outline"
                size="sm"
                onClick={() => onOpenChange(false)}
              >
                Cancel
              </Button>
              <Button
                size="sm"
                onClick={handleSave}
                disabled={loading}
                className="gap-2"
              >
                <Save className="size-4" />
                {loading ? "Saving…" : "Save"}
              </Button>
            </div>
          </div>
        </DialogContent>
        <ZipGuideModal open={showZipGuide} onOpenChange={setShowZipGuide} />
        <SelectiveDeleteModal open={showSelectiveDelete} onOpenChange={setShowSelectiveDelete} />
      </LazyMotion>
    </Dialog>

    <BetaConfirmDialog
      open={showBetaConfirm}
      onOpenChange={setShowBetaConfirm}
      onConfirm={() => {
        onBetaToggle?.(true);
      }}
    />
    </>
  );
}
