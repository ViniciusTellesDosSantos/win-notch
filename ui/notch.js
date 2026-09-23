(() => {
  const { invoke } = window.__TAURI__.core;
  const { listen } = window.__TAURI__.event;
  const { getCurrentWindow, LogicalPosition, LogicalSize, currentMonitor } = window.__TAURI__.window;

  const appWindow = getCurrentWindow();

  // Sizes and all the position/drag math below are in *logical* pixels throughout, so the
  // notch looks the same real-world size on a 100% and a 150%/200% scaled Windows display.
  // Monitor geometry from Tauri is physical, so it's converted via the monitor's
  // scaleFactor in logicalMonitor() — everything past that boundary stays logical.
  // COLLAPSED_SIZE is kept in sync by hand with COLLAPSED_SIZE in src-tauri/src/config.rs.
  const COLLAPSED_SIZE = { width: 32, height: 32 };
  const EXPANDED_SIZE = { width: 340, height: 232 };
  const HOVER_EXPAND_DELAY = 120;
  const HOVER_COLLAPSE_DELAY = 350;
  // Matches the width/height transition duration in notch.css — used to time the real OS
  // window resize on collapse (see collapse()'s comment for why this can't be transitionend).
  const COLLAPSE_RESIZE_DELAY_MS = 170;
  const SNAP_MARGIN = 48; // logical px
  const USAGE_POLL_MS = 8000;
  const USAGE_WINDOW_MS = 5 * 60 * 60 * 1000;
  const ALWAYS_ON_TOP_REASSERT_MS = 3000;
  const RING_CIRCUMFERENCE = 2 * Math.PI * 27;
  const PILL_RING_CIRCUMFERENCE = 2 * Math.PI * 12;

  const notchEl = document.getElementById("notch");
  const pillRing = document.getElementById("pill-ring");
  const pillRingProgress = document.getElementById("pill-ring-progress");
  const headerDot = document.getElementById("header-dot");
  const usagePrimary = document.getElementById("usage-primary");
  const usageSecondary = document.getElementById("usage-secondary");
  const usageWeekly = document.getElementById("usage-weekly");
  const usageFootnote = document.getElementById("usage-footnote");
  const ringProgress = document.getElementById("ring-progress");
  const captureBtn = document.getElementById("capture-btn");
  const captureMessage = document.getElementById("capture-message");
  const freshnessEl = document.getElementById("freshness");

  let edge = "top";
  let offsetCenter = 0;
  let isExpanded = false;
  let isDragging = false;
  let expandTimer = null;
  let collapseTimer = null;
  let collapseResizeTimer = null;
  let lastUsageDto = null;

  // Manual, pointer-capture-driven drag state (see the pointerdown handler below for why
  // this replaces Tauri's native startDragging()).
  let dragOrigin = null;
  let dragCurrentPos = null;
  let dragStartScreen = null;
  let dragFrame = null;

  // tauri.conf.json's alwaysOnTop only sets Windows' topmost flag once, at window
  // creation. That flag isn't a single fixed layer, though — it's a band shared with every
  // other topmost window, and whichever of those gets activated most recently ends up
  // nearer the top *within* that band. Any other app that also marks itself topmost (tray
  // flyouts, overlays, other widgets) can end up drawing over the notch over time. The
  // standard fix for exactly this is what this does: keep re-asserting topmost instead of
  // only setting it once.
  setInterval(() => {
    appWindow.setAlwaysOnTop(true).catch(() => {});
  }, ALWAYS_ON_TOP_REASSERT_MS);

  function applyEdgeClass() {
    notchEl.classList.remove("edge-top", "edge-bottom", "edge-left", "edge-right");
    notchEl.classList.add(`edge-${edge}`);
  }

  function clampCenter(total, center, sizeAlong) {
    const available = Math.max(total - sizeAlong, 0);
    return Math.min(Math.max(center - sizeAlong / 2, 0), available);
  }

  // Converts Tauri's physical monitor geometry into logical pixels, matching the unit
  // every size/position value in this file is expressed in.
  async function logicalMonitor() {
    const monitor = await currentMonitor();
    if (!monitor) return null;
    const scale = monitor.scaleFactor || 1;
    return {
      x: monitor.position.x / scale,
      y: monitor.position.y / scale,
      width: monitor.size.width / scale,
      height: monitor.size.height / scale,
      scale,
    };
  }

  function computeWindowRect(monitor, size) {
    const { x: mx, y: my, width: mw, height: mh } = monitor;
    switch (edge) {
      case "top":
        return { x: mx + clampCenter(mw, offsetCenter, size.width), y: my };
      case "bottom":
        return { x: mx + clampCenter(mw, offsetCenter, size.width), y: my + mh - size.height };
      case "left":
        return { x: mx, y: my + clampCenter(mh, offsetCenter, size.height) };
      case "right":
        return { x: mx + mw - size.width, y: my + clampCenter(mh, offsetCenter, size.height) };
      default:
        return { x: mx, y: my };
    }
  }

  async function moveAndResize(size) {
    const monitor = await logicalMonitor();
    if (!monitor) return null;
    const pos = computeWindowRect(monitor, size);
    await appWindow.setSize(new LogicalSize(size.width, size.height));
    await appWindow.setPosition(new LogicalPosition(pos.x, pos.y));
    return pos;
  }

  async function expand() {
    expandTimer = null;
    // A collapse may still be waiting to shrink the real OS window back down (see
    // collapse()) — cancel it, or it'd shrink the window out from under the panel a
    // moment after the user re-entered.
    clearTimeout(collapseResizeTimer);
    collapseResizeTimer = null;
    if (isExpanded) return;
    isExpanded = true;
    await moveAndResize(EXPANDED_SIZE);
    requestAnimationFrame(() => notchEl.classList.add("expanded"));
    refreshUsage();
  }

  // Shrinking the real OS window back to COLLAPSED_SIZE is what stops it from swallowing
  // clicks meant for whatever's underneath, so it can't depend on an event that isn't
  // guaranteed to fire: if the mouse re-enters before the CSS shrink transition finishes,
  // that transition gets cancelled/reversed and "transitionend" never fires for it (or
  // fires for the wrong direction) — the window would stay stuck at EXPANDED_SIZE,
  // invisibly blocking clicks near the notch. A plain timer matching the transition's
  // duration always fires, same as every other timer in this file.
  function collapse() {
    if (!isExpanded || isDragging) return;
    isExpanded = false;
    notchEl.classList.remove("expanded");
    clearTimeout(collapseResizeTimer);
    collapseResizeTimer = setTimeout(() => {
      collapseResizeTimer = null;
      moveAndResize(COLLAPSED_SIZE);
    }, COLLAPSE_RESIZE_DELAY_MS);
  }

  notchEl.addEventListener("mouseenter", () => {
    clearTimeout(collapseTimer);
    collapseTimer = null;
    // Pointer capture (see the drag handlers below) should already keep the pointer
    // "inside" notchEl for the whole drag per spec, so this shouldn't normally be
    // reachable while dragging — kept as a defensive guard since that behavior can't be
    // verified without a real Windows/WebView2 install.
    if (isExpanded || expandTimer || isDragging) return;
    expandTimer = setTimeout(expand, HOVER_EXPAND_DELAY);
  });

  notchEl.addEventListener("mouseleave", () => {
    clearTimeout(expandTimer);
    expandTimer = null;
    if (!isExpanded || isDragging) return;
    collapseTimer = setTimeout(collapse, HOVER_COLLAPSE_DELAY);
  });

  // --- Drag to reposition + edge snap -------------------------------------------------

  // This drags the window manually (Pointer Events + setPointerCapture) instead of using
  // Tauri's native appWindow.startDragging(). On Windows, startDragging() just posts
  // WM_NCLBUTTONDOWN and returns immediately — the actual move happens entirely inside
  // Windows' own modal SC_MOVE loop, which only ends when the mouse is released, and
  // there's no JS-exposed event for that ("onMoved" only fires per actual movement, so
  // pausing mid-drag while still holding the button — completely normal — looks
  // indistinguishable from "drag finished" to anything watching for silence on it). Driving
  // the drag ourselves trades a little native smoothness for a real, unambiguous end
  // signal: pointerup. setPointerCapture keeps pointermove/pointerup targeted at notchEl
  // for the rest of the gesture even as the window moves out from under the cursor between
  // frames — the same mechanism used by web drag-and-drop for exactly this problem.
  //
  // Listens on the whole notch (not just #pill): hovering for HOVER_EXPAND_DELAY (120ms)
  // before the user manages to press the button is the common case, not the exception, so
  // by the time pointerdown would fire on #pill it's usually already hidden behind the
  // expanded panel (pointer-events: none). Dragging from the panel background works too;
  // only the capture button opts out, so it can still be clicked normally.
  notchEl.addEventListener("pointerdown", async (event) => {
    if (event.button !== 0 || event.target.closest("#capture-btn")) return;
    clearTimeout(expandTimer);
    expandTimer = null;
    clearTimeout(collapseTimer);
    collapseTimer = null;
    // A stale resize from a still-pending collapse must not land mid-drag or right after
    // settling — it would reposition the window using an outdated edge/offsetCenter.
    clearTimeout(collapseResizeTimer);
    collapseResizeTimer = null;

    const monitor = await logicalMonitor();
    if (!monitor) return;

    // Drag math (here and in onDragSettled) assumes the window's logical footprint is
    // COLLAPSED_SIZE throughout — force that *before* the drag starts rather than only
    // after, so a drag begun from the expanded panel doesn't run with mismatched size.
    if (isExpanded) {
      isExpanded = false;
      notchEl.classList.remove("expanded");
      await moveAndResize(COLLAPSED_SIZE);
    }

    dragOrigin = computeWindowRect(monitor, COLLAPSED_SIZE);
    dragCurrentPos = { ...dragOrigin };
    dragStartScreen = { x: event.screenX, y: event.screenY };
    isDragging = true;
    notchEl.setPointerCapture(event.pointerId);
  });

  notchEl.addEventListener("pointermove", (event) => {
    if (!isDragging) return;
    dragCurrentPos = {
      x: dragOrigin.x + (event.screenX - dragStartScreen.x),
      y: dragOrigin.y + (event.screenY - dragStartScreen.y),
    };
    // Coalesce to at most one setPosition call per frame instead of one per pointermove
    // (which can fire far faster than the window can actually move via IPC).
    if (dragFrame) return;
    dragFrame = requestAnimationFrame(() => {
      dragFrame = null;
      if (!isDragging || !dragCurrentPos) return;
      appWindow.setPosition(new LogicalPosition(dragCurrentPos.x, dragCurrentPos.y)).catch(() => {});
    });
  });

  async function endDrag(event) {
    if (!isDragging) return;
    isDragging = false;
    notchEl.releasePointerCapture(event.pointerId);
    if (dragFrame) {
      cancelAnimationFrame(dragFrame);
      dragFrame = null;
    }
    await onDragSettled(dragCurrentPos ?? dragOrigin);
  }
  notchEl.addEventListener("pointerup", endDrag);
  notchEl.addEventListener("pointercancel", endDrag);

  async function onDragSettled(position) {
    const monitor = await logicalMonitor();
    if (!monitor) return;

    const size = COLLAPSED_SIZE;

    const distTop = Math.abs(position.y - monitor.y);
    const distBottom = Math.abs(position.y + size.height - (monitor.y + monitor.height));
    const distLeft = Math.abs(position.x - monitor.x);
    const distRight = Math.abs(position.x + size.width - (monitor.x + monitor.width));

    const candidates = [
      ["top", distTop],
      ["bottom", distBottom],
      ["left", distLeft],
      ["right", distRight],
    ].sort((a, b) => a[1] - b[1]);

    if (candidates[0][1] <= SNAP_MARGIN) {
      edge = candidates[0][0];
    }

    const centerX = position.x + size.width / 2 - monitor.x;
    const centerY = position.y + size.height / 2 - monitor.y;
    offsetCenter = edge === "top" || edge === "bottom" ? centerX : centerY;

    applyEdgeClass();
    await moveAndResize(COLLAPSED_SIZE);
    invoke("save_position", { edge, offset: offsetCenter }).catch(() => {});
  }

  // --- Usage ---------------------------------------------------------------------------

  function formatTokens(n) {
    return n.toLocaleString("pt-BR");
  }

  function formatDuration(ms) {
    const totalMinutes = Math.max(Math.round(ms / 60000), 0);
    const hours = Math.floor(totalMinutes / 60);
    const minutes = totalMinutes % 60;
    return `${hours}h${String(minutes).padStart(2, "0")}min`;
  }

  // The weekly window can reset up to 7 days out — "172h34min" is technically correct but
  // unreadable, so once it's past a day this switches to a coarser "3d 4h" instead.
  function formatLongDuration(ms) {
    const totalHours = Math.max(Math.round(ms / 3_600_000), 0);
    if (totalHours < 24) {
      return formatDuration(ms);
    }
    const days = Math.floor(totalHours / 24);
    const hours = totalHours % 24;
    return `${days}d ${hours}h`;
  }

  function renderUsage(dto) {
    lastUsageDto = dto;
    headerDot.dataset.state = dto.status;
    pillRing.dataset.state = dto.status;

    // Fraction (0–1) driving both rings — the mini one at rest and the big one expanded
    // show the exact same number, just at different sizes.
    let fraction = 0;

    usageWeekly.textContent = "";

    switch (dto.status) {
      case "loading":
        usagePrimary.textContent = "Carregando…";
        usageSecondary.textContent = "";
        usageFootnote.textContent = "";
        break;
      case "unavailable":
        usagePrimary.textContent = "Sem dados locais";
        usageSecondary.textContent = dto.reason ?? "";
        usageFootnote.textContent = "";
        break;
      case "idle":
        usagePrimary.textContent = "Sem sessão ativa";
        usageSecondary.textContent = "nas últimas 5h";
        usageFootnote.textContent = "";
        break;
      case "active_official": {
        const percent = Math.round(dto.percent);
        usagePrimary.textContent = `${percent}%`;
        if (dto.resets_at) {
          const remainingMs = new Date(dto.resets_at).getTime() - Date.now();
          usageSecondary.textContent = `reinicia em ${formatDuration(remainingMs)}`;
        } else {
          usageSecondary.textContent = "janela de 5h";
        }
        usageFootnote.textContent = "";

        if (dto.weekly_percent != null) {
          const weeklyPercent = Math.round(dto.weekly_percent);
          if (dto.weekly_resets_at) {
            const weeklyRemainingMs = new Date(dto.weekly_resets_at).getTime() - Date.now();
            usageWeekly.textContent = `Semana: ${weeklyPercent}% · reinicia em ${formatLongDuration(weeklyRemainingMs)}`;
          } else {
            usageWeekly.textContent = `Semana: ${weeklyPercent}%`;
          }
        }

        fraction = Math.min(Math.max(dto.percent / 100, 0), 1);
        break;
      }
      case "active": {
        usagePrimary.textContent = `${formatTokens(dto.tokens)} tokens`;
        const remainingMs = new Date(dto.resets_at).getTime() - Date.now();
        usageSecondary.textContent = `reinicia em ${formatDuration(remainingMs)}`;
        usageFootnote.textContent = "Estimativa derivada dos logs locais, não é o limite oficial do plano.";
        if (dto.reason) {
          // Why the official percentage wasn't used instead — there's no console in a
          // release build, so this is the only place a failure here is ever visible.
          usageFootnote.textContent += ` (debug: ${dto.reason})`;
        }
        fraction = Math.min(Math.max(1 - remainingMs / USAGE_WINDOW_MS, 0), 1);
        break;
      }
    }

    ringProgress.style.strokeDashoffset = String(RING_CIRCUMFERENCE * (1 - fraction));
    pillRingProgress.style.strokeDashoffset = String(PILL_RING_CIRCUMFERENCE * (1 - fraction));

    const ageSeconds = Math.max(Math.round((Date.now() - new Date(dto.last_updated).getTime()) / 1000), 0);
    freshnessEl.textContent = `atualizado há ${ageSeconds}s`;
  }

  async function refreshUsage() {
    try {
      const dto = await invoke("get_usage");
      renderUsage(dto);
    } catch (err) {
      console.error("failed to refresh usage", err);
    }
  }

  // Keep the "reinicia em"/"atualizado há" countdowns ticking between polls without
  // re-fetching from the backend every second.
  setInterval(() => {
    if (lastUsageDto) renderUsage(lastUsageDto);
  }, 1000);
  setInterval(refreshUsage, USAGE_POLL_MS);

  // --- Screenshot ------------------------------------------------------------------------

  captureBtn.addEventListener("click", async () => {
    captureMessage.textContent = "";
    try {
      // Just opens the selection overlay window — the actual result (copied/cancelled/
      // error) arrives later via the "screenshot-result" event below, since the capture
      // itself finishes in that other window, not this one.
      await invoke("capture_region");
    } catch (err) {
      captureMessage.textContent = `Erro: ${err}`;
    }
  });

  listen("screenshot-result", (event) => {
    const { ok, message } = event.payload;
    if (ok) {
      captureMessage.textContent = "Copiado para a área de transferência";
    } else if (message) {
      captureMessage.textContent = `Erro: ${message}`;
    } else {
      captureMessage.textContent = "";
    }
  });

  // --- Boot --------------------------------------------------------------------------

  function applySettings(settings) {
    edge = settings.edge;
    offsetCenter = settings.offset_along_edge;
    applyEdgeClass();
  }

  // Fired by the tray's "Redefinir posição" action: the Rust side already moved the real
  // window, this just keeps our local edge/offsetCenter from going stale, since they drive
  // the next hover-expand or drag — without this, the notch would silently "snap back" to
  // the old position the next time either of those runs.
  listen("notch-position-reset", (event) => applySettings(event.payload));

  (async () => {
    try {
      applySettings(await invoke("get_settings"));
    } catch (err) {
      console.error("failed to load settings", err);
    }
    refreshUsage();
  })();
})();
