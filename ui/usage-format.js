// Formatting shared by the tab (notch.js) and the popover (popover.js), which live in two
// separate windows — sessionSummary() in particular is what keeps the tab's ring and the
// popover's session bar showing the same number in the same color.
window.UsageFormat = (() => {
  const USAGE_WINDOW_MS = 5 * 60 * 60 * 1000;
  const WEEKLY_RESET_FORMAT = new Intl.DateTimeFormat("pt-BR", {
    weekday: "short",
    hour: "2-digit",
    minute: "2-digit",
  });

  function clamp(value, min, max) {
    return Math.min(Math.max(value, min), max);
  }

  function levelFor(percent) {
    if (percent >= 80) return "high";
    if (percent >= 50) return "mid";
    return "ok";
  }

  function formatTokens(n) {
    return n.toLocaleString("pt-BR");
  }

  function formatTokensShort(n) {
    if (n >= 1_000_000) return `${(n / 1_000_000).toFixed(1).replace(".", ",")}M`;
    if (n >= 1_000) return `${Math.round(n / 1_000)}k`;
    return String(n);
  }

  function formatSessionReset(ms) {
    const totalMinutes = Math.max(Math.round(ms / 60000), 0);
    const hours = Math.floor(totalMinutes / 60);
    const minutes = totalMinutes % 60;
    if (hours === 0) return `Reinicia em ${minutes} min`;
    return `Reinicia em ${hours}h${String(minutes).padStart(2, "0")}`;
  }

  function formatWeeklyReset(iso) {
    return `Reinicia ${WEEKLY_RESET_FORMAT.format(new Date(iso))}`;
  }

  // Session fraction (0–1), color level and the tab's short label for a usage DTO. In the
  // token-count fallback there's no official percentage, so the fraction is how far into
  // the 5h window we are and the level is the neutral "est" rather than a usage level.
  function sessionSummary(dto) {
    switch (dto.status) {
      case "active_official":
        return {
          fraction: clamp(dto.percent / 100, 0, 1),
          level: levelFor(dto.percent),
          tabText: `${Math.round(dto.percent)}%`,
        };
      case "active": {
        const remainingMs = new Date(dto.resets_at).getTime() - Date.now();
        return {
          fraction: clamp(1 - remainingMs / USAGE_WINDOW_MS, 0, 1),
          level: "est",
          tabText: formatTokensShort(dto.tokens),
          remainingMs,
        };
      }
      case "idle":
        return { fraction: 0, level: "ok", tabText: "0%" };
      case "unavailable":
        return { fraction: 0, level: "ok", tabText: "—" };
      default:
        return { fraction: 0, level: "ok", tabText: "…" };
    }
  }

  return {
    clamp,
    levelFor,
    formatTokens,
    formatSessionReset,
    formatWeeklyReset,
    sessionSummary,
  };
})();
