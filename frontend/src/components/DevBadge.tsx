import React, { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { DevFlags, formatFlags } from "./DeveloperSettings";

interface DevBadgeProps {
  /** When true, render a compact dot suitable for the collapsed sidebar. */
  isCollapsed?: boolean;
}

/**
 * Small "DEV" indicator shown only in the development build so the dev app is
 * visually distinguishable from the production Nixon.app when both run side by
 * side. Next inlines `process.env.NODE_ENV` at build time, so in the static
 * production export this component compiles down to `null`.
 */
const DevBadge: React.FC<DevBadgeProps> = ({ isCollapsed = false }) => {
  // specs/0059 — hooks run unconditionally, before the production early return below.
  const [flags, setFlags] = useState<DevFlags | null>(null);

  useEffect(() => {
    if (process.env.NODE_ENV === "production") return;
    invoke<DevFlags>("dev_get_flags").then(setFlags).catch(() => setFlags(null));
  }, []);

  if (process.env.NODE_ENV === "production") return null;

  const label = flags
    ? `Development build — separate data from the production app\nDev flags: ${formatFlags(flags)}`
    : "Development build — separate data from the production app";

  if (isCollapsed) {
    return (
      <span
        title={label}
        aria-label={label}
        className="inline-flex items-center justify-center px-1.5 py-0.5 rounded-[2px] text-[9px] font-bold uppercase tracking-[0.12em] bg-brand/15 text-brand border border-brand/40"
      >
        DEV
      </span>
    );
  }

  return (
    <span
      title={label}
      aria-label={label}
      className="inline-flex items-center justify-center px-1.5 py-0.5 rounded-[2px] text-[9px] font-bold uppercase tracking-[0.12em] bg-brand/15 text-brand border border-brand/40"
    >
      DEV
    </span>
  );
};

export default DevBadge;
