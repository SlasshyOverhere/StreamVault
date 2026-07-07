import { DownloadJob } from "@/services/api";
import { formatFileSize } from "@/utils/format";
import { Button } from "@/components/ui/button";
import { cn } from "@/lib/utils";
import {
  AlertTriangle,
  CheckCircle2,
  Download,
  FolderOpen,
  Loader2,
  PauseCircle,
  Trash2,
  CheckSquare,
  ArrowDownToLine,
  HardDrive,
  Cloud,
  Globe,
  Search,
  ChevronRight,
  X,
  MoreHorizontal,
} from "lucide-react";
import { LazyMotion, m, AnimatePresence, domAnimation } from "framer-motion";
import { useState, useMemo, memo, useDeferredValue } from "react";

interface DownloadsViewProps {
  jobs: DownloadJob[];
  onCancel: (job: DownloadJob) => void | Promise<void>;
  onOpen: (job: DownloadJob) => void | Promise<void>;
  onDeleteJob: (jobId: string) => void | Promise<void>;
  onClearHistory: () => void | Promise<void>;
}

const formatSpeed = (bytesPerSecond?: number | null) => {
  if (!bytesPerSecond) return "0 B/s";
  return `${formatFileSize(bytesPerSecond)}/s`;
};

const formatTimeRemaining = (job: DownloadJob): string | null => {
  if (job.status !== "downloading" || !job.speedBytesPerSecond || job.speedBytesPerSecond <= 0) return null;
  const remaining = job.totalBytes - job.downloadedBytes;
  if (remaining <= 0) return null;
  const seconds = remaining / job.speedBytesPerSecond;
  if (seconds < 60) return `${Math.ceil(seconds)}s`;
  if (seconds < 3600) return `${Math.ceil(seconds / 60)}m`;
  const h = Math.floor(seconds / 3600);
  const m = Math.ceil((seconds % 3600) / 60);
  return `${h}h ${m}m`;
};

const formatRelativeDate = (iso: string) => {
  const date = new Date(iso);
  const now = Date.now();
  const diff = (now - date.getTime()) / 1000;
  if (diff < 60) return "just now";
  if (diff < 3600) return `${Math.floor(diff / 60)}m ago`;
  if (diff < 86400) return `${Math.floor(diff / 3600)}h ago`;
  if (diff < 604800) return `${Math.floor(diff / 86400)}d ago`;
  return date.toLocaleDateString(undefined, { month: "short", day: "numeric" });
};

type DownloadJobStatus = "queued" | "preparing" | "downloading" | "completed" | "failed" | "cancelled";

const isActiveStatus = (status: DownloadJobStatus): status is "queued" | "preparing" | "downloading" =>
  status === "queued" || status === "preparing" || status === "downloading";

const isArchivedStatus = (status: DownloadJobStatus) =>
  status === "completed" || status === "failed" || status === "cancelled";

const statusMeta = (status: DownloadJobStatus) => {
  switch (status) {
    case "completed":
      return { label: "Completed", tone: "ok" as const };
    case "failed":
      return { label: "Failed", tone: "danger" as const };
    case "cancelled":
      return { label: "Cancelled", tone: "muted" as const };
    case "preparing":
      return { label: "Preparing", tone: "warn" as const };
    case "queued":
      return { label: "Queued", tone: "muted" as const };
    default:
      return { label: "Downloading", tone: "live" as const };
  }
};

const toneClass = (tone: ReturnType<typeof statusMeta>["tone"]) => {
  switch (tone) {
    case "ok":
      return "text-emerald-300";
    case "danger":
      return "text-red-300";
    case "warn":
      return "text-amber-300";
    case "live":
      return "text-white";
    case "muted":
    default:
      return "text-white/40";
  }
};

const sourceIconKey = (kind: string): "gdrive" | "direct" | "other" => {
  if (kind === "gdrive") return "gdrive";
  if (kind === "direct") return "direct";
  return "other";
};

const SourceIconByKey = ({ kind }: { kind: "gdrive" | "direct" | "other" }) => {
  if (kind === "gdrive") return <Cloud className="size-4" />;
  if (kind === "direct") return <Globe className="size-4" />;
  return <HardDrive className="size-4" />;
};

type TabFilter = "all" | "active" | "completed" | "failed";

const TABS: Array<{ id: TabFilter; label: string }> = [
  { id: "all", label: "All" },
  { id: "active", label: "Active" },
  { id: "completed", label: "Completed" },
  { id: "failed", label: "Failed" },
];

const VISIBLE_LIMIT = 12;

export function DownloadsView({
  jobs,
  onCancel,
  onOpen,
  onDeleteJob,
  onClearHistory,
}: DownloadsViewProps) {
  const [selectedIds, setSelectedIds] = useState<Set<string>>(new Set());
  const [selectionMode, setSelectionMode] = useState(false);
  const [activeTab, setActiveTab] = useState<TabFilter>("all");
  const [query, setQuery] = useState("");
  const [showFullArchive, setShowFullArchive] = useState(false);
  const [activeMenuId, setActiveMenuId] = useState<string | null>(null);

  const deferredQuery = useDeferredValue(query);

  const activeJobs = useMemo(
    () => jobs.filter((job) => isActiveStatus(job.status)),
    [jobs],
  );
  const archivedJobs = useMemo(
    () =>
      [...jobs]
        .filter((job) => isArchivedStatus(job.status))
        .sort((a, b) => new Date(b.updatedAt).getTime() - new Date(a.updatedAt).getTime()),
    [jobs],
  );
  const failedArchivedJobs = useMemo(
    () => archivedJobs.filter((j) => j.status === "failed" || j.status === "cancelled"),
    [archivedJobs],
  );

  const counts = useMemo(
    () => ({
      all: jobs.length,
      active: activeJobs.length,
      completed: archivedJobs.filter((j) => j.status === "completed").length,
      failed: failedArchivedJobs.length,
    }),
    [jobs, activeJobs, archivedJobs, failedArchivedJobs],
  );

  const matchesQuery = (job: DownloadJob) => {
    if (!deferredQuery.trim()) return true;
    const q = deferredQuery.toLowerCase();
    return (
      job.title.toLowerCase().includes(q) ||
      job.fileName.toLowerCase().includes(q) ||
      job.sourceKind.toLowerCase().includes(q)
    );
  };

  const filteredActive = useMemo(
    () =>
      (activeTab === "all" || activeTab === "active" ? activeJobs : []).filter(matchesQuery),
    // matchesQuery is recreated each render but only depends on deferredQuery; including it directly
    // trips React Compiler. The deferredQuery dep captures the same value.
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [activeJobs, activeTab, deferredQuery],
  );
  const filteredArchived = useMemo(() => {
    if (activeTab === "active") return [];
    if (activeTab === "completed")
      return archivedJobs.filter((j) => j.status === "completed").filter(matchesQuery);
    if (activeTab === "failed") return failedArchivedJobs.filter(matchesQuery);
    return archivedJobs.filter(matchesQuery);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [archivedJobs, failedArchivedJobs, activeTab, deferredQuery]);

  const visibleActive = filteredActive;
  const visibleArchived = showFullArchive
    ? filteredArchived
    : filteredArchived.slice(0, VISIBLE_LIMIT);
  const archivedOverflow = Math.max(0, filteredArchived.length - VISIBLE_LIMIT);

  const toggleSelect = (id: string) => {
    setSelectedIds((prev) => {
      const next = new Set(prev);
      if (next.has(id)) next.delete(id);
      else next.add(id);
      return next;
    });
  };

  const exitSelection = () => {
    setSelectionMode(false);
    setSelectedIds(new Set());
  };

  const handleDeleteSelected = async () => {
    if (selectedIds.size === 0) return;
    if (!window.confirm(`Delete ${selectedIds.size} selected item${selectedIds.size !== 1 ? "s" : ""}?`)) return;
    const ids = Array.from(selectedIds);
    const results = await Promise.allSettled(ids.map((id) => onDeleteJob(id)));
    for (const result of results) {
      if (result.status === "rejected") console.error("Failed to delete job:", result.reason);
    }
    exitSelection();
  };

  const handleClearHistory = async () => {
    if (archivedJobs.length === 0) return;
    if (!window.confirm(`Clear ${archivedJobs.length} finished item${archivedJobs.length !== 1 ? "s" : ""}?`)) return;
    await onClearHistory();
  };

  const isFilterEmpty =
    filteredActive.length === 0 && filteredArchived.length === 0 && jobs.length > 0;

  return (
    <LazyMotion features={domAnimation}>
      <div className="relative h-full overflow-y-auto">
        <div className="mx-auto flex w-full max-w-6xl flex-col gap-8 px-4 pb-16 pt-8 sm:px-6 sm:pt-10 lg:px-10">
          {/* Page header */}
          <header className="flex flex-col gap-6">
            <div className="flex flex-col gap-4 sm:flex-row sm:items-end sm:justify-between">
              <div className="space-y-1.5">
                <h1 className="text-[2rem] font-bold leading-none tracking-tight text-white sm:text-[2.25rem]">
                  Downloads
                </h1>
                <p className="text-sm text-white/45">
                  {counts.active > 0
                    ? `${counts.active} active · ${counts.completed} completed · ${counts.failed} failed`
                    : "No active transfers. Parallel engine idle."}
                </p>
              </div>

              <div className="flex items-center gap-2">
                <label className="relative flex items-center">
                  <span className="sr-only">Search downloads</span>
                  <Search className="pointer-events-none absolute left-3 size-4 text-white/30" />
                  <input
                    type="search"
                    value={query}
                    onChange={(e) => setQuery(e.target.value)}
                    placeholder="Search title or file"
                    className="h-10 w-56 rounded-xl border border-white/10 bg-white/[0.04] pl-9 pr-3 text-sm text-white placeholder:text-white/30 transition-colors hover:border-white/20 focus:border-white/30 focus:bg-white/[0.06] focus:outline-none"
                  />
                </label>
              </div>
            </div>

            {/* Filter bar — single affordance replacing chips + tab pills */}
            <div className="flex flex-col gap-3 sm:flex-row sm:items-center sm:justify-between">
              <nav
                role="tablist"
                aria-label="Filter downloads"
                className="inline-flex w-fit items-center gap-1 rounded-2xl border border-white/10 bg-white/[0.03] p-1"
              >
                {TABS.map((tab) => {
                  const active = activeTab === tab.id;
                  return (
                    <button
                      key={tab.id}
                      type="button"
                      role="tab"
                      aria-selected={active}
                      onClick={() => setActiveTab(tab.id)}
                      className={cn(
                        "flex items-center gap-2 rounded-xl px-3.5 py-1.5 text-sm font-medium transition-colors",
                        active
                          ? "bg-white text-black"
                          : "text-white/55 hover:bg-white/[0.05] hover:text-white",
                      )}
                    >
                      <span>{tab.label}</span>
                      <span
                        className={cn(
                          "tabular-nums rounded-md px-1.5 py-0.5 text-[11px] font-semibold leading-none",
                          active ? "bg-black/10 text-black/70" : "bg-white/[0.06] text-white/45",
                        )}
                      >
                        {counts[tab.id]}
                      </span>
                    </button>
                  );
                })}
              </nav>

              <div className="flex items-center gap-2">
                {selectionMode ? (
                  <>
                    <span className="text-sm text-white/55">
                      {selectedIds.size} selected
                    </span>
                    <Button
                      variant="ghost"
                      size="sm"
                      onClick={exitSelection}
                      className="h-9 rounded-xl text-white/65 hover:bg-white/[0.06] hover:text-white"
                    >
                      Cancel
                    </Button>
                    <Button
                      variant="ghost"
                      size="sm"
                      onClick={handleDeleteSelected}
                      disabled={selectedIds.size === 0}
                      className="h-9 rounded-xl text-red-300 hover:bg-red-500/10 hover:text-red-200 disabled:opacity-40"
                    >
                      <Trash2 className="mr-2 size-3.5" />
                      Delete
                    </Button>
                  </>
                ) : (
                  <>
                    {jobs.length > 0 && (
                      <Button
                        variant="ghost"
                        size="sm"
                        onClick={() => setSelectionMode(true)}
                        className="h-9 rounded-xl text-white/65 hover:bg-white/[0.06] hover:text-white"
                      >
                        Select
                      </Button>
                    )}
                    {archivedJobs.length > 0 && (
                      <Button
                        variant="ghost"
                        size="sm"
                        onClick={handleClearHistory}
                        className="h-9 rounded-xl text-white/45 hover:bg-white/[0.06] hover:text-white"
                      >
                        Clear history
                      </Button>
                    )}
                  </>
                )}
              </div>
            </div>
          </header>

          {/* Whole-tab empty state */}
          {jobs.length === 0 ? (
            <EmptyState />
          ) : (
            <div className="space-y-10">
              {/* Active section */}
              {(activeTab === "all" || activeTab === "active") && (
                <Section
                  title="Active"
                  description="Transfers in progress or waiting in the queue."
                  count={filteredActive.length}
                  total={activeJobs.length}
                >
                  {filteredActive.length === 0 ? (
                    <NoMatches label="No active downloads match this filter." />
                  ) : (
                    <div className="space-y-2">
                      <AnimatePresence initial={false}>
                        {visibleActive.map((job) => (
                          <m.div
                            key={job.id}
                            layout
                            initial={{ opacity: 0, y: 8 }}
                            animate={{ opacity: 1, y: 0 }}
                            exit={{ opacity: 0, y: -8 }}
                            transition={{ duration: 0.2, ease: [0.19, 1, 0.22, 1] }}
                          >
                            <DownloadRow
                              job={job}
                              onCancel={onCancel}
                              onOpen={onOpen}
                              onDelete={onDeleteJob}
                              isSelectionMode={selectionMode}
                              isSelected={selectedIds.has(job.id)}
                              onToggleSelect={() => toggleSelect(job.id)}
                              menuOpen={activeMenuId === job.id}
                              onMenuToggle={() =>
                                setActiveMenuId((prev) => (prev === job.id ? null : job.id))
                              }
                              onMenuClose={() => setActiveMenuId(null)}
                            />
                          </m.div>
                        ))}
                      </AnimatePresence>
                    </div>
                  )}
                </Section>
              )}

              {/* Archive section */}
              {(activeTab === "all" ||
                activeTab === "completed" ||
                activeTab === "failed") && (
                <Section
                  title="Archive"
                  description="Finished, failed, and cancelled transfers. Newest first."
                  count={filteredArchived.length}
                  total={archivedJobs.length}
                >
                  {filteredArchived.length === 0 ? (
                    <NoMatches label="Nothing in the archive matches this filter." />
                  ) : (
                    <div className="space-y-2">
                      <AnimatePresence initial={false}>
                        {visibleArchived.map((job) => (
                          <m.div
                            key={job.id}
                            layout
                            initial={{ opacity: 0, y: 8 }}
                            animate={{ opacity: 1, y: 0 }}
                            exit={{ opacity: 0, y: -8 }}
                            transition={{ duration: 0.2, ease: [0.19, 1, 0.22, 1] }}
                          >
                            <DownloadRow
                              job={job}
                              onCancel={onCancel}
                              onOpen={onOpen}
                              onDelete={onDeleteJob}
                              isSelectionMode={selectionMode}
                              isSelected={selectedIds.has(job.id)}
                              onToggleSelect={() => toggleSelect(job.id)}
                              menuOpen={activeMenuId === job.id}
                              onMenuToggle={() =>
                                setActiveMenuId((prev) => (prev === job.id ? null : job.id))
                              }
                              onMenuClose={() => setActiveMenuId(null)}
                            />
                          </m.div>
                        ))}
                      </AnimatePresence>

                      {archivedOverflow > 0 && !showFullArchive && (
                        <button
                          type="button"
                          onClick={() => setShowFullArchive(true)}
                          className="group flex w-full items-center justify-between rounded-2xl border border-white/5 bg-transparent px-4 py-3 text-left transition-colors hover:border-white/15 hover:bg-white/[0.03]"
                        >
                          <span className="text-sm text-white/55 group-hover:text-white">
                            Show {archivedOverflow} more archived item{archivedOverflow !== 1 ? "s" : ""}
                          </span>
                          <ChevronRight className="size-4 text-white/30 group-hover:translate-x-0.5 group-hover:text-white" />
                        </button>
                      )}
                      {showFullArchive && archivedOverflow > 0 && (
                        <button
                          type="button"
                          onClick={() => setShowFullArchive(false)}
                          className="flex w-full items-center justify-center rounded-2xl px-4 py-3 text-sm text-white/45 transition-colors hover:bg-white/[0.03] hover:text-white"
                        >
                          Show less
                        </button>
                      )}
                    </div>
                  )}
                </Section>
              )}

              {isFilterEmpty && (
                <NoMatches label="No downloads match this filter." />
              )}
            </div>
          )}
        </div>
      </div>
    </LazyMotion>
  );
}

function Section({
  title,
  description,
  count,
  total,
  children,
}: {
  title: string;
  description: string;
  count: number;
  total: number;
  children: React.ReactNode;
}) {
  return (
    <section className="space-y-3">
      <div className="flex items-baseline justify-between gap-4 border-b border-white/[0.06] pb-3">
        <div className="space-y-0.5">
          <div className="flex items-baseline gap-2">
            <h2 className="text-base font-semibold text-white">{title}</h2>
            <span className="text-sm tabular-nums text-white/35">
              {count}
              {count !== total ? ` of ${total}` : ""}
            </span>
          </div>
          <p className="text-sm text-white/40">{description}</p>
        </div>
      </div>
      {children}
    </section>
  );
}

function NoMatches({ label }: { label: string }) {
  return (
    <div className="rounded-2xl border border-dashed border-white/[0.08] px-6 py-10 text-center">
      <p className="text-sm text-white/45">{label}</p>
    </div>
  );
}

function EmptyState() {
  return (
    <section
      aria-label="Empty downloads state"
      className="overflow-hidden rounded-3xl border border-white/[0.08] bg-white/[0.015]"
    >
      <div className="flex items-center justify-between border-b border-white/[0.06] px-5 py-3">
        <span className="text-xs font-medium text-white/45">Examples</span>
        <span className="text-xs text-white/30">How rows will appear</span>
      </div>
      <div className="grid gap-px bg-white/[0.04] sm:grid-cols-3">
        <GhostRow
          icon={Loader2}
          tone="live"
          status="Downloading"
          title="Sample — 2.1 GB / 4.8 GB"
          meta="Speed 18.4 MB/s · ETA 2m"
          progress={44}
          spinIcon
        />
        <GhostRow
          icon={CheckCircle2}
          tone="ok"
          status="Completed"
          title="Sample — finished 1.2 GB"
          meta="Saved to ~/Movies"
          progress={100}
        />
        <GhostRow
          icon={AlertTriangle}
          tone="danger"
          status="Failed"
          title="Sample — network error"
          meta="Tap retry from the row menu"
          progress={62}
        />
      </div>

      <div className="flex flex-col items-start gap-4 px-6 py-8 sm:flex-row sm:items-center sm:justify-between sm:px-8">
        <div className="space-y-1">
          <h3 className="text-lg font-semibold text-white">No downloads yet</h3>
          <p className="max-w-md text-sm text-white/50">
            Pick a movie or episode from the Cloud tab and start a transfer. Active
            downloads, the queue, and your archive will all live here.
          </p>
        </div>
        <div className="flex items-center gap-2 rounded-2xl border border-white/10 bg-white/[0.03] px-4 py-3 text-sm text-white/55">
          <ArrowDownToLine className="size-4 text-white/50" />
          <span>From the Cloud tab, tap a poster and choose Download</span>
        </div>
      </div>
    </section>
  );
}

function GhostRow({
  icon: Icon,
  tone,
  status,
  title,
  meta,
  progress,
  spinIcon,
}: {
  icon: typeof Download;
  tone: ReturnType<typeof statusMeta>["tone"];
  status: string;
  title: string;
  meta: string;
  progress: number;
  spinIcon?: boolean;
}) {
  return (
    <div className="relative bg-[hsl(0_0%_5%)] p-5">
      <div className="flex items-center gap-2 text-xs font-medium">
        <Icon
          className={cn(
            "size-3.5",
            toneClass(tone),
            spinIcon && "animate-spin [animation-duration:2.4s]",
          )}
        />
        <span className={cn("uppercase tracking-wide", toneClass(tone))}>{status}</span>
      </div>
      <p className="mt-3 text-sm font-medium text-white/75">{title}</p>
      <p className="mt-0.5 text-xs text-white/35">{meta}</p>
      <div className="mt-4 h-1 w-full overflow-hidden rounded-full bg-white/[0.06]">
        <div
          className={cn(
            "h-full rounded-full",
            tone === "ok"
              ? "bg-emerald-400/70"
              : tone === "danger"
                ? "bg-red-400/70"
                : "bg-white/70",
          )}
          style={{ width: `${progress}%` }}
        />
      </div>
    </div>
  );
}

const DownloadRow = memo(function DownloadRow({
  job,
  onCancel,
  onOpen,
  onDelete,
  isSelectionMode,
  isSelected,
  onToggleSelect,
  menuOpen,
  onMenuToggle,
  onMenuClose,
}: {
  job: DownloadJob;
  onCancel: (job: DownloadJob) => void | Promise<void>;
  onOpen: (job: DownloadJob) => void | Promise<void>;
  onDelete: (jobId: string) => void | Promise<void>;
  isSelectionMode?: boolean;
  isSelected?: boolean;
  onToggleSelect?: () => void;
  menuOpen?: boolean;
  onMenuToggle?: () => void;
  onMenuClose?: () => void;
}) {
  const active = isActiveStatus(job.status);
  const timeRemaining = formatTimeRemaining(job);
  const sourceKey = sourceIconKey(job.sourceKind);

  const handleRowClick = () => {
    if (isSelectionMode) onToggleSelect?.();
  };

  const handleKey = (e: React.KeyboardEvent<HTMLDivElement>) => {
    if (!isSelectionMode) return;
    if (e.key === " " || e.key === "Enter") {
      e.preventDefault();
      onToggleSelect?.();
    }
  };

  return (
    <div
      role="button"
      tabIndex={isSelectionMode ? 0 : -1}
      onClick={handleRowClick}
      onKeyDown={handleKey}
      className={cn(
        "group relative flex w-full items-start gap-4 rounded-2xl border bg-white/[0.015] p-4 text-left transition-colors sm:gap-5 sm:p-5",
        isSelected
          ? "border-white/30 bg-white/[0.06]"
          : "border-white/[0.07] hover:border-white/[0.16] hover:bg-white/[0.03]",
        isSelectionMode && "cursor-pointer",
      )}
    >
      {/* Selection checkbox or source icon */}
      {isSelectionMode ? (
        <div
          className={cn(
            "mt-0.5 flex size-5 shrink-0 items-center justify-center rounded-md border transition-colors",
            isSelected ? "border-white bg-white text-black" : "border-white/25 text-transparent",
          )}
        >
          <CheckSquare className="size-3.5" strokeWidth={3} />
        </div>
      ) : (
        <div className="mt-0.5 flex size-9 shrink-0 items-center justify-center rounded-xl border border-white/[0.08] bg-white/[0.03] text-white/55">
          <SourceIconByKey kind={sourceKey} />
        </div>
      )}

      {/* Center column */}
      <div className="min-w-0 flex-1 space-y-2">
        <div className="flex flex-wrap items-center gap-2 text-xs">
          <StatusBadge status={job.status} />
          <span className="text-white/30">·</span>
          <span className="text-white/40">
            {job.sourceKind.replace(/-/g, " ")}
          </span>
          {active ? (
            <span className="text-white/30">·</span>
          ) : null}
          {active && job.speedBytesPerSecond ? (
            <span className="text-white/65 tabular-nums">
              {formatSpeed(job.speedBytesPerSecond)}
            </span>
          ) : null}
          {active && timeRemaining ? (
            <>
              <span className="text-white/30">·</span>
              <span className="text-white/65 tabular-nums">{timeRemaining} left</span>
            </>
          ) : null}
        </div>

        <div className="space-y-0.5">
          <h3 className="truncate text-[15px] font-semibold text-white">{job.title}</h3>
          <p className="truncate text-xs text-white/35 font-mono">{job.fileName}</p>
        </div>

        <div className="flex items-center gap-3 text-xs text-white/45">
          <span className="tabular-nums">
            {formatFileSize(job.downloadedBytes)} of {formatFileSize(job.totalBytes)}
          </span>
          <span className="text-white/15">·</span>
          <span>{formatRelativeDate(job.updatedAt)}</span>
          {job.error && (
            <>
              <span className="text-white/15">·</span>
              <span className="truncate text-red-300/85">{job.error}</span>
            </>
          )}
        </div>

        {/* Progress */}
        {active && (
          <div className="flex items-center gap-3 pt-1">
            <div className="h-1 flex-1 overflow-hidden rounded-full bg-white/[0.06]">
              <m.div
                className="h-full rounded-full bg-white"
                initial={false}
                animate={{ width: `${Math.max(0, Math.min(100, job.progress))}%` }}
                transition={{ duration: 0.5, ease: [0.19, 1, 0.22, 1] }}
              />
            </div>
            <span className="w-9 text-right text-xs tabular-nums text-white/60">
              {Math.round(job.progress)}%
            </span>
          </div>
        )}
      </div>

      {/* Right column — actions */}
      {!isSelectionMode && (
        <div className="flex shrink-0 items-center gap-1.5">
          {active ? (
            <Button
              type="button"
              onClick={(e) => {
                e.stopPropagation();
                void onCancel(job);
              }}
              variant="ghost"
              size="sm"
              className="h-9 rounded-xl text-white/65 hover:bg-white/[0.06] hover:text-white"
            >
              {job.status === "downloading" ? (
                <Loader2 className="mr-1.5 size-3.5 animate-spin" />
              ) : (
                <PauseCircle className="mr-1.5 size-3.5" />
              )}
              {job.status === "downloading" ? "Stop" : "Cancel"}
            </Button>
          ) : job.targetExists ? (
            <Button
              type="button"
              onClick={(e) => {
                e.stopPropagation();
                void onOpen(job);
              }}
              variant="ghost"
              size="sm"
              className="h-9 rounded-xl text-white/65 hover:bg-white/[0.06] hover:text-white"
            >
              <FolderOpen className="mr-1.5 size-3.5" />
              Open
            </Button>
          ) : (
            <span className="inline-flex h-9 items-center rounded-xl border border-white/[0.07] bg-white/[0.02] px-3 text-xs text-white/30">
              File removed
            </span>
          )}

          <div className="relative">
            <Button
              type="button"
              aria-haspopup="menu"
              aria-expanded={menuOpen}
              onClick={(e) => {
                e.stopPropagation();
                onMenuToggle?.();
              }}
              variant="ghost"
              size="icon"
              className="size-9 rounded-xl text-white/45 hover:bg-white/[0.06] hover:text-white"
            >
              <MoreHorizontal className="size-4" />
            </Button>
            <AnimatePresence>
              {menuOpen && (
                <>
                  <div
                    className="fixed inset-0 z-10"
                    onClick={(e) => {
                      e.stopPropagation();
                      onMenuClose?.();
                    }}
                  />
                  <m.div
                    initial={{ opacity: 0, y: -4, scale: 0.98 }}
                    animate={{ opacity: 1, y: 0, scale: 1 }}
                    exit={{ opacity: 0, y: -4, scale: 0.98 }}
                    transition={{ duration: 0.12, ease: [0.19, 1, 0.22, 1] }}
                    role="menu"
                    className="absolute right-0 top-11 z-20 w-44 overflow-hidden rounded-xl border border-white/10 bg-[hsl(0_0%_9%)] p-1 shadow-2xl"
                  >
                    {job.targetExists && (
                      <button
                        type="button"
                        role="menuitem"
                        onClick={(e) => {
                          e.stopPropagation();
                          onMenuClose?.();
                          void onOpen(job);
                        }}
                        className="flex w-full items-center gap-2 rounded-lg px-3 py-2 text-sm text-white/80 hover:bg-white/[0.06]"
                      >
                        <FolderOpen className="size-3.5" />
                        Open file
                      </button>
                    )}
                    {active && (
                      <button
                        type="button"
                        role="menuitem"
                        onClick={(e) => {
                          e.stopPropagation();
                          onMenuClose?.();
                          void onCancel(job);
                        }}
                        className="flex w-full items-center gap-2 rounded-lg px-3 py-2 text-sm text-white/80 hover:bg-white/[0.06]"
                      >
                        <X className="size-3.5" />
                        {job.status === "downloading" ? "Stop" : "Cancel transfer"}
                      </button>
                    )}
                    <button
                      type="button"
                      role="menuitem"
                      onClick={(e) => {
                        e.stopPropagation();
                        onMenuClose?.();
                        void onDelete(job.id);
                      }}
                      className="flex w-full items-center gap-2 rounded-lg px-3 py-2 text-sm text-red-300 hover:bg-red-500/10"
                    >
                      <Trash2 className="size-3.5" />
                      Remove from list
                    </button>
                  </m.div>
                </>
              )}
            </AnimatePresence>
          </div>
        </div>
      )}
    </div>
  );
});

function StatusBadge({ status }: { status: DownloadJobStatus }) {
  const meta = statusMeta(status);

  const dot =
    meta.tone === "ok"
      ? "bg-emerald-400"
      : meta.tone === "danger"
        ? "bg-red-400"
        : meta.tone === "warn"
          ? "bg-amber-300"
          : meta.tone === "live"
            ? "bg-white"
            : "bg-white/35";

  const isLive = meta.tone === "live";

  return (
    <span
      className={cn(
        "inline-flex items-center gap-1.5 rounded-full px-2.5 py-0.5",
        meta.tone === "live" ? "bg-white/[0.08]" : "bg-transparent",
        meta.tone === "live" ? "text-white" : toneClass(meta.tone),
      )}
    >
      <span className="relative flex size-2">
        {isLive && (
          <span className="absolute inline-flex h-full w-full animate-ping rounded-full bg-white/60 opacity-60" />
        )}
        <span className={cn("relative inline-flex size-2 rounded-full", dot)} />
      </span>
      <span className="font-medium">{meta.label}</span>
    </span>
  );
}
