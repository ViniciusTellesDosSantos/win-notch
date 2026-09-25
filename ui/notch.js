(() => {
  const { invoke } = window.__TAURI__.core;
  const { listen } = window.__TAURI__.event;
  const { getCurrentWindow, LogicalPosition, LogicalSize, currentMonitor } = window.__TAURI__.window;

  const appWindow = getCurrentWindow();

  // Sizes and all the position/drag math below are in *logical* pixels throughout, so the
  // notch looks the same real-world size on a 100% and a 150%/200% scaled Windows display.
  // Monitor geometry from Tauri is physical, so it's converted via the monitor's
  // scaleFactor in logicalMonitor() — everything past that boundary stays logical.
  //
  // The tab's collapsed sizes (body + a 14px concave corner on each side along the edge)
  // are kept in sync by hand with collapsed_size() in src-tauri/src/config.rs, and all of
  // these with the geometry in notch.css.
  const TAB_VERTICAL = { width: 64, height: 128 };
  const TAB_HORIZONTAL = { width: 136, height: 56 };
  const POPOVER = { width: 280, height: 188 };
  const POPOVER_GAP = 12;
  // Keeps the popover's pointer clear of its rounded corners when the tab sits near the
  // end of a monitor edge and the popover gets clamped off-center from it.
  const TAIL_INSET = 24;

  const HOVER_EXPAND_DELAY = 120;
  const HOVER_COLLAPSE_DELAY = 350;
  // Matches the popover's fade transition in notch.css — used to time the real OS window
  // resize on collapse (see collapse()'s comment for why this can't be transitionend).
  const COLLAPSE_RESIZE_DELAY_MS = 170;
  const SNAP_MARGIN = 48; // logical px
  const USAGE_POLL_MS = 8000;
  const USAGE_WINDOW_MS = 5 * 60 * 60 * 1000;
  const ALWAYS_ON_TOP_REASSERT_MS = 3000;
  const CAPTURE_NOTE_MS = 3000;
  const RING_CIRCUMFERENCE = 2 * Math.PI * 18;

  const WEEKLY_RESET_FORMAT = new Intl.DateTimeFormat("pt-BR", {
    weekday: "short",
    hour: "2-digit",
    minute: "2-digit",
  });

  const notchEl = document.getElementById("notch");
  const tabRing = document.getElementById("tab-ring");
  const tabRingProgress = document.getElementById("tab-ring-progress");
  const tabPercent = document.getElementById("tab-percent");
  const sessionReset = document.getElementById("session-reset");
  const sessionFill = document.getElementById("session-fill");
  const sessionUsed = document.getElementById("session-used");
  const weeklyLine = document.getElementById("weekly-line");
  const weeklyReset = document.getElementById("weekly-reset");
  const weeklyFill = document.getElementById("weekly-fill");
  const weeklyUsed = document.getElementById("weekly-used");
  const usageFootnote = document.getElementById("usage-footnote");
  const captureBtn = document.getElementById("capture-btn");

  let edge = "top";
  let offsetCenter = 0;
  let isExpanded = false;
  let isDragging = false;
  let expandTimer = null;
  let collapseTimer = null;
  let collapseResizeTimer = null;
  let lastUsageDto = null;
  let captureNote = null;

  // Manual, pointer-capture-driven drag state (see the pointerdown handler below for why
  // this replaces Tauri's native startDragging()).
  let dragOrigin = null;
  let dragCurrentPos = null;
  let dragStartScreen = null;
  let dragSize = null;
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

  function isVertical(e) {
    return e === "left" || e === "right";
  }

  // The tab stands upright on the side edges and lies flat on the top/bottom ones, and the
  // popover opens on its inner side — so both window sizes depend on the current edge.
  function sizesFor(e) {
    if (isVertical(e)) {
      return {
        collapsed: TAB_VERTICAL,
        expanded: {
          width: TAB_VERTICAL.width + POPOVER_GAP + POPOVER.width,
          height: Math.max(TAB_VERTICAL.height, POPOVER.height),
        },
      };
    }
    return {
      collapsed: TAB_HORIZONTAL,
      expanded: {
        width: Math.max(TAB_HORIZONTAL.width, POPOVER.width),
        height: TAB_HORIZONTAL.height + POPOVER_GAP + POPOVER.height,
      },
    };
  }

  function applyEdgeClass() {
    notchEl.classList.remove("edge-top", "edge-bottom", "edge-left", "edge-right");
    notchEl.classList.add(`edge-${edge}`);
  }

  // --tab-offset shifts the tab inside a (bigger) expanded window so it stays exactly where
  // it was on screen while collapsed; --tail-offset aims the popover's pointer at it.
  function setOffsets(tabOffset, tailOffset) {
    notchEl.style.setProperty("--tab-offset", `${tabOffset}px`);
    notchEl.style.setProperty("--tail-offset", `${tailOffset}px`);
  }

  function clamp(value, min, max) {
    return Math.min(Math.max(value, min), max);
  }

  function clampCenter(total, center, sizeAlong) {
    const available = Math.max(total - sizeAlong, 0);
    return Math.min(Math.max(center - sizeAlong / 2, 0), available);
  }

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
    // collapse()) — cancel it, or it'd shrink the window out from under the popover a
    // moment after the user re-entered.
    clearTimeout(collapseResizeTimer);
    collapseResizeTimer = null;
    if (isExpanded) return;
    isExpanded = true;

    const monitor = await logicalMonitor();
    if (!monitor) {
      isExpanded = false;
      return;
    }
    const { collapsed, expanded } = sizesFor(edge);
    const vertical = isVertical(edge);
    const from = computeWindowRect(monitor, collapsed);
    const to = computeWindowRect(monitor, expanded);
    const tabOffset = vertical ? from.y - to.y : from.x - to.x;
    const tabAlong = vertical ? collapsed.height : collapsed.width;
    const popoverAlong = vertical ? POPOVER.height : POPOVER.width;
    const tailOffset = clamp(tabOffset + tabAlong / 2, TAIL_INSET, popoverAlong - TAIL_INSET);

    await moveAndResize(expanded);
    setOffsets(tabOffset, tailOffset);
    requestAnimationFrame(() => notchEl.classList.add("expanded"));
    refreshUsage();
  }

  // Shrinking the real OS window back to the collapsed size is what stops it from
  // swallowing clicks meant for whatever's underneath, so it can't depend on an event that
  // isn't guaranteed to fire: if the mouse re-enters before the CSS fade-out finishes, that
  // transition gets cancelled/reversed and "transitionend" never fires for it (or fires for
  // the wrong direction) — the window would stay stuck at its expanded size, invisibly
  // blocking clicks near the notch. A plain timer matching the transition's duration always
  // fires, same as every other timer in this file.
  function collapse() {
    if (!isExpanded || isDragging) return;
    isExpanded = false;
    notchEl.classList.remove("expanded");
    clearTimeout(collapseResizeTimer);
    collapseResizeTimer = setTimeout(() => {
      collapseResizeTimer = null;
      setOffsets(0, 0);
      moveAndResize(sizesFor(edge).collapsed);
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
  // Listens on the whole notch (tab and popover alike), so a drag can start from either;
  // only buttons opt out, so they can still be clicked normally.
  notchEl.addEventListener("pointerdown", async (event) => {
    if (event.button !== 0 || event.target.closest("button")) return;
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

    // Drag math (here and in onDragSettled) assumes the window is just the collapsed tab
    // throughout — force that *before* the drag starts rather than only after, so a drag
    // begun from the expanded popover doesn't run with a mismatched size.
    const { collapsed } = sizesFor(edge);
    if (isExpanded) {
      isExpanded = false;
      notchEl.classList.remove("expanded");
      setOffsets(0, 0);
      await moveAndResize(collapsed);
    }

    dragSize = collapsed;
    dragOrigin = computeWindowRect(monitor, collapsed);
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

    // The size that was actually being dragged — the tab's orientation for the *old* edge.
    const size = dragSize;

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

    // Resize to the *new* edge's collapsed size: snapping from a side edge to the top one
    // (or vice versa) flips the tab between upright and flat.
    applyEdgeClass();
    await moveAndResize(sizesFor(edge).collapsed);
    invoke("save_position", { edge, offset: offsetCenter }).catch(() => {});
  }

  // --- Usage ---------------------------------------------------------------------------

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

  function levelFor(percent) {
    if (percent >= 80) return "high";
    if (percent >= 50) return "mid";
    return "ok";
  }

  function setFill(el, fraction, level) {
    el.style.width = `${clamp(fraction, 0, 1) * 100}%`;
    el.dataset.level = level;
  }

  function renderUsage(dto) {
    lastUsageDto = dto;

    // Fraction (0–1) and color level shared by the tab's ring and the session bar.
    let fraction = 0;
    let level = "ok";
    let tabText = "…";
    let footnote = "";

    sessionReset.textContent = "";
    weeklyLine.hidden = true;

    switch (dto.status) {
      case "loading":
        sessionUsed.textContent = "Carregando…";
        break;
      case "unavailable":
        tabText = "—";
        sessionUsed.textContent = "Sem dados locais";
        footnote = dto.reason ?? "";
        break;
      case "idle":
        tabText = "0%";
        sessionUsed.textContent = "Sem sessão ativa nas últimas 5h";
        break;
      case "active_official": {
        const percent = Math.round(dto.percent);
        fraction = dto.percent / 100;
        level = levelFor(dto.percent);
        tabText = `${percent}%`;
        sessionUsed.textContent = `${percent}% usado`;
        if (dto.resets_at) {
          sessionReset.textContent = formatSessionReset(new Date(dto.resets_at).getTime() - Date.now());
        }

        if (dto.weekly_percent != null) {
          const weeklyPercent = Math.round(dto.weekly_percent);
          weeklyLine.hidden = false;
          weeklyUsed.textContent = `${weeklyPercent}% usado`;
          weeklyReset.textContent = dto.weekly_resets_at
            ? `Reinicia ${WEEKLY_RESET_FORMAT.format(new Date(dto.weekly_resets_at))}`
            : "";
          setFill(weeklyFill, dto.weekly_percent / 100, levelFor(dto.weekly_percent));
        }
        break;
      }
      case "active": {
        // No official percentage here — the fill is how far into the 5h window we are,
        // not plan usage, so it gets the neutral "est" color instead of a usage level.
        const remainingMs = new Date(dto.resets_at).getTime() - Date.now();
        fraction = 1 - remainingMs / USAGE_WINDOW_MS;
        level = "est";
        tabText = formatTokensShort(dto.tokens);
        sessionUsed.textContent = `${formatTokens(dto.tokens)} tokens`;
        sessionReset.textContent = formatSessionReset(remainingMs);
        footnote = "Estimativa pelos logs locais, não é o limite oficial do plano.";
        if (dto.reason) {
          // Why the official percentage wasn't used instead — there's no console in a
          // release build, so this is the only place a failure here is ever visible.
          footnote += ` (debug: ${dto.reason})`;
        }
        break;
      }
    }

    fraction = clamp(fraction, 0, 1);
    tabPercent.textContent = tabText;
    tabRing.dataset.level = level;
    tabRingProgress.style.strokeDashoffset = String(RING_CIRCUMFERENCE * (1 - fraction));
    setFill(sessionFill, fraction, level);

    if (captureNote && Date.now() < captureNote.until) {
      footnote = captureNote.text;
    }
    usageFootnote.textContent = footnote;
    usageFootnote.title = footnote;
  }

  async function refreshUsage() {
    try {
      const dto = await invoke("get_usage");
      renderUsage(dto);
    } catch (err) {
      console.error("failed to refresh usage", err);
    }
  }

  // Keep the "Reinicia em" countdown (and a temporary capture note's expiry) ticking
  // between polls without re-fetching from the backend every second.
  setInterval(() => {
    if (lastUsageDto) renderUsage(lastUsageDto);
  }, 1000);
  setInterval(refreshUsage, USAGE_POLL_MS);

  // --- Screenshot ------------------------------------------------------------------------

  function showCaptureNote(text) {
    captureNote = text ? { text, until: Date.now() + CAPTURE_NOTE_MS } : null;
    if (lastUsageDto) renderUsage(lastUsageDto);
  }

  captureBtn.addEventListener("click", async () => {
    showCaptureNote(null);
    try {
      // Just opens the selection overlay window — the actual result (copied/cancelled/
      // error) arrives later via the "screenshot-result" event below, since the capture
      // itself finishes in that other window, not this one.
      await invoke("capture_region");
    } catch (err) {
      showCaptureNote(`Erro: ${err}`);
    }
  });

  listen("screenshot-result", (event) => {
    const { ok, message } = event.payload;
    if (ok) {
      showCaptureNote("Copiado para a área de transferência");
    } else if (message) {
      showCaptureNote(`Erro: ${message}`);
    } else {
      showCaptureNote(null);
    }
  });

  // --- Boot --------------------------------------------------------------------------

  function applySettings(settings) {
    edge = settings.edge;
    offsetCenter = settings.offset_along_edge;
    applyEdgeClass();
  }

  // Fired by the tray's "Redefinir posição" action: the Rust side already moved (and
  // resized) the real window, this just keeps our local edge/offsetCenter from going
  // stale, since they drive the next hover-expand or drag — without this, the notch would
  // silently "snap back" to the old position the next time either of those runs.
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
