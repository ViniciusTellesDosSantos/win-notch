(() => {
  const { invoke } = window.__TAURI__.core;
  const { listen, emit } = window.__TAURI__.event;
  const {
    getCurrentWindow,
    Window,
    LogicalSize,
    PhysicalPosition,
    currentMonitor,
    cursorPosition,
    monitorFromPoint,
  } = window.__TAURI__.window;
  const { sessionSummary } = window.UsageFormat;

  const appWindow = getCurrentWindow();

  // Sizes and edge math are in the *logical* pixels of the monitor the tab is on, so the
  // notch looks the same real-world size on a 100% and a 150%/200% scaled display. Actual
  // window positions are always set in *physical* pixels, converted with that monitor's
  // own scale (toPhysical): a logical position would be converted by Tauri with the scale
  // of whichever monitor the window happens to be on at that moment, which is wrong
  // mid-way between two monitors scaled differently.
  //
  // The tab and the popover are two separate windows. An earlier version grew a single
  // window on hover, which meant moving and resizing it and re-offsetting the tab inside it
  // across several async steps — the tab visibly jumped in between. Now this window is
  // always exactly the tab (only resized when a drag changes its orientation), and hovering
  // just moves the popover's window next to it.
  //
  // Collapsed sizes kept in sync by hand with collapsed_size() in src-tauri/src/config.rs,
  // and with notch.css; the popover window's with tauri.conf.json and popover.css.
  const TAB_VERTICAL = { width: 64, height: 128 };
  const TAB_HORIZONTAL = { width: 136, height: 56 };
  const POPOVER_WINDOW = { width: 300, height: 292 };
  const POPOVER_MARGIN = 10;
  const POPOVER_GAP = 12;
  // Where the popover window waits while closed. Moving it (SWP_NOACTIVATE) instead of
  // show()/hide() matters: show() on Windows activates the window, stealing focus from
  // whatever app the user is typing in every time they hover the notch.
  const PARKED = { x: -10000, y: -10000 };
  // Keeps the popover's pointer clear of its rounded corners when the tab sits near the
  // end of a monitor edge and the popover gets clamped off-center from it.
  const TAIL_INSET = 24;

  const HOVER_EXPAND_DELAY = 120;
  const HOVER_COLLAPSE_DELAY = 350;
  // Matches the popover's fade-out in popover.css: parking it any sooner would cut the
  // animation off. A plain timer rather than transitionend, since that one isn't
  // guaranteed to fire (a reversed transition never does).
  const PARK_DELAY_MS = 170;
  // Below this much pointer travel (logical px) a press is a click, not a drag, and leaves
  // the tab exactly where it is.
  const DRAG_THRESHOLD = 4;
  const USAGE_POLL_MS = 8000;
  const ALWAYS_ON_TOP_REASSERT_MS = 3000;
  const RING_CIRCUMFERENCE = 2 * Math.PI * 18;

  const notchEl = document.getElementById("notch");
  const tabRing = document.getElementById("tab-ring");
  const tabRingProgress = document.getElementById("tab-ring-progress");
  const tabPercent = document.getElementById("tab-percent");

  let edge = "top";
  let offsetCenter = 0;
  let isExpanded = false;
  let isDragging = false;
  let hoverTab = false;
  let hoverPopover = false;
  let expandTimer = null;
  let collapseTimer = null;
  let parkTimer = null;
  let lastUsageDto = null;
  let popoverWindowPromise = null;

  // Manual, pointer-capture-driven drag state (see the pointerdown handler below for why
  // this replaces Tauri's native startDragging()).
  let dragGrab = null; // cursor position relative to the window's corner, physical px
  let dragStartCursor = null;
  let dragScale = 1;
  let dragMoved = false;
  let dragFrame = null;

  // tauri.conf.json's alwaysOnTop only sets Windows' topmost flag once, at window
  // creation. That flag isn't a single fixed layer, though — it's a band shared with every
  // other topmost window, and whichever of those gets activated most recently ends up
  // nearer the top *within* that band. Any other app that also marks itself topmost (tray
  // flyouts, overlays, other widgets) can end up drawing over the notch over time. The
  // standard fix for exactly this is what this does: keep re-asserting topmost instead of
  // only setting it once. (popover.js does the same for its own window.)
  setInterval(() => {
    appWindow.setAlwaysOnTop(true).catch(() => {});
  }, ALWAYS_ON_TOP_REASSERT_MS);

  function isVertical(e) {
    return e === "left" || e === "right";
  }

  // The tab stands upright on the side edges and lies flat on the top/bottom ones.
  function collapsedSize(e) {
    return isVertical(e) ? TAB_VERTICAL : TAB_HORIZONTAL;
  }

  function popoverWindow() {
    popoverWindowPromise ??= Window.getByLabel("popover");
    return popoverWindowPromise;
  }

  function applyEdgeClass() {
    notchEl.classList.remove("edge-top", "edge-bottom", "edge-left", "edge-right");
    notchEl.classList.add(`edge-${edge}`);
  }

  function clamp(value, min, max) {
    return Math.min(Math.max(value, min), max);
  }

  function clampCenter(total, center, sizeAlong) {
    const available = Math.max(total - sizeAlong, 0);
    return Math.min(Math.max(center - sizeAlong / 2, 0), available);
  }

  // A Tauri monitor (physical geometry) in its own logical pixels, plus what's needed to
  // convert positions back to physical ones on it.
  function toLogicalMonitor(monitor) {
    const scale = monitor.scaleFactor || 1;
    return {
      x: monitor.position.x / scale,
      y: monitor.position.y / scale,
      width: monitor.size.width / scale,
      height: monitor.size.height / scale,
      scale,
      physicalX: monitor.position.x,
      physicalY: monitor.position.y,
      name: monitor.name ?? null,
    };
  }

  async function logicalMonitor() {
    const monitor = await currentMonitor();
    return monitor ? toLogicalMonitor(monitor) : null;
  }

  function toPhysical(monitor, pos) {
    return new PhysicalPosition(
      Math.round(monitor.physicalX + (pos.x - monitor.x) * monitor.scale),
      Math.round(monitor.physicalY + (pos.y - monitor.y) * monitor.scale),
    );
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

  // Puts the tab on `monitor` (passed in, not looked up: right after a drag the window may
  // still be straddling two monitors, and currentMonitor() would pick by the window rather
  // than by where the user dropped it). Size first, then position, so the window arrives
  // on the target monitor already at its final size — if that monitor's scale differs,
  // Windows keeps the window's logical size as it crosses over.
  async function placeTab(monitor) {
    const size = collapsedSize(edge);
    await appWindow.setSize(new LogicalSize(size.width, size.height));
    await appWindow.setPosition(toPhysical(monitor, computeWindowRect(monitor, size)));
  }

  // Where the popover window goes for the tab's current position: on the tab's inner side,
  // POPOVER_GAP away, centered on the tab along the edge but kept inside the monitor. The
  // pointer (tailOffset, relative to the balloon's own start) then aims at the tab's center.
  function popoverPlacement(monitor) {
    const tabSize = collapsedSize(edge);
    const tab = computeWindowRect(monitor, tabSize);
    const bodyW = POPOVER_WINDOW.width - 2 * POPOVER_MARGIN;
    const bodyH = POPOVER_WINDOW.height - 2 * POPOVER_MARGIN;

    let bodyX;
    let bodyY;
    let tailOffset;
    if (isVertical(edge)) {
      const centerY = tab.y + tabSize.height / 2;
      bodyY = clamp(centerY - bodyH / 2, monitor.y, monitor.y + monitor.height - bodyH);
      bodyX = edge === "right" ? tab.x - POPOVER_GAP - bodyW : tab.x + tabSize.width + POPOVER_GAP;
      tailOffset = clamp(centerY - bodyY, TAIL_INSET, bodyH - TAIL_INSET);
    } else {
      const centerX = tab.x + tabSize.width / 2;
      bodyX = clamp(centerX - bodyW / 2, monitor.x, monitor.x + monitor.width - bodyW);
      bodyY = edge === "bottom" ? tab.y - POPOVER_GAP - bodyH : tab.y + tabSize.height + POPOVER_GAP;
      tailOffset = clamp(centerX - bodyX, TAIL_INSET, bodyW - TAIL_INSET);
    }
    return { pos: { x: bodyX - POPOVER_MARGIN, y: bodyY - POPOVER_MARGIN }, tailOffset };
  }

  async function parkPopover() {
    const popover = await popoverWindow();
    if (popover) await popover.setPosition(new PhysicalPosition(PARKED.x, PARKED.y)).catch(() => {});
  }

  async function expand() {
    expandTimer = null;
    // A collapse may still be waiting to park the popover (see collapse()) — cancel it, or
    // it'd yank the popover away a moment after the user re-entered.
    clearTimeout(parkTimer);
    parkTimer = null;
    if (isExpanded) return;
    isExpanded = true;

    const [monitor, popover] = await Promise.all([logicalMonitor(), popoverWindow()]);
    if (!monitor || !popover) {
      isExpanded = false;
      return;
    }
    const place = popoverPlacement(monitor);
    await popover.setPosition(toPhysical(monitor, place.pos));
    // The popover's content stays transparent until this arrives, so moving its window
    // into place above never shows a stale frame.
    emit("popover-open", { edge, tailOffset: place.tailOffset });
  }

  function collapse() {
    collapseTimer = null;
    if (!isExpanded || isDragging) return;
    isExpanded = false;
    emit("popover-close");
    clearTimeout(parkTimer);
    parkTimer = setTimeout(() => {
      parkTimer = null;
      parkPopover();
    }, PARK_DELAY_MS);
  }

  // Hover spans two windows: the tab reports through mouseenter/mouseleave here, the
  // popover through its "popover-hover" event. The popover only closes once the pointer is
  // in neither, so crossing the gap between them doesn't close it.
  function onHoverChange() {
    if (hoverTab || hoverPopover) {
      clearTimeout(collapseTimer);
      collapseTimer = null;
      // Pointer capture (see the drag handlers below) should already keep hover events
      // from firing mid-drag per spec — the isDragging check is a defensive guard, since
      // that behavior can't be verified without a real Windows/WebView2 install.
      if (!isExpanded && !expandTimer && !isDragging) {
        // Re-entering the popover while it's fading out reopens it right away.
        expandTimer = setTimeout(expand, hoverTab ? HOVER_EXPAND_DELAY : 0);
      }
    } else {
      clearTimeout(expandTimer);
      expandTimer = null;
      if (isExpanded && !isDragging && !collapseTimer) {
        collapseTimer = setTimeout(collapse, HOVER_COLLAPSE_DELAY);
      }
    }
  }

  notchEl.addEventListener("mouseenter", () => {
    hoverTab = true;
    onHoverChange();
  });
  notchEl.addEventListener("mouseleave", () => {
    hoverTab = false;
    onHoverChange();
  });
  listen("popover-hover", (event) => {
    hoverPopover = Boolean(event.payload?.inside);
    onHoverChange();
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
  notchEl.addEventListener("pointerdown", async (event) => {
    if (event.button !== 0) return;
    clearTimeout(expandTimer);
    expandTimer = null;
    clearTimeout(collapseTimer);
    collapseTimer = null;
    clearTimeout(parkTimer);
    parkTimer = null;

    // The popover is placed relative to the tab, so it can't stay open while the tab moves.
    if (isExpanded) {
      isExpanded = false;
      emit("popover-close");
    }
    parkPopover();

    const [cursor, windowPos] = await Promise.all([cursorPosition(), appWindow.outerPosition()]);
    dragGrab = { x: cursor.x - windowPos.x, y: cursor.y - windowPos.y };
    dragStartCursor = { x: cursor.x, y: cursor.y };
    dragScale = window.devicePixelRatio || 1;
    dragMoved = false;
    isDragging = true;
    notchEl.setPointerCapture(event.pointerId);
  });

  // The window follows the real cursor (physical, global) rather than pointer-event
  // deltas: those are in the starting monitor's logical pixels and drift once the tab
  // crosses onto a monitor with a different scale. At most one move in flight per frame.
  notchEl.addEventListener("pointermove", () => {
    if (!isDragging || dragFrame) return;
    dragFrame = requestAnimationFrame(async () => {
      const cursor = await cursorPosition().catch(() => null);
      dragFrame = null;
      if (!isDragging || !cursor) return;
      if (!dragMoved) {
        const travel = Math.hypot(cursor.x - dragStartCursor.x, cursor.y - dragStartCursor.y);
        if (travel < DRAG_THRESHOLD * dragScale) return;
        dragMoved = true;
      }
      appWindow.setPosition(new PhysicalPosition(cursor.x - dragGrab.x, cursor.y - dragGrab.y)).catch(() => {});
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
    if (dragMoved) await onDragSettled();
  }
  notchEl.addEventListener("pointerup", endDrag);
  notchEl.addEventListener("pointercancel", endDrag);

  function nearestEdge(x, y, width, height) {
    const distances = [
      ["top", y],
      ["bottom", height - y],
      ["left", x],
      ["right", width - x],
    ];
    distances.sort((a, b) => a[1] - b[1]);
    return distances[0][0];
  }

  // Snaps to the edge nearest the *cursor*, on the monitor *under the cursor*. Measuring
  // from the tab's own edges instead (as an earlier version did) broke once the tab got
  // wider than twice the snap margin — with the cursor at the screen's side, the flat tab's
  // far edge could never get close enough — and with two monitors the window straddles the
  // boundary, so "the window's monitor" is a coin toss. The notch always hugs some edge, so
  // there's no margin at all: wherever it's dropped, it goes to the closest one.
  async function onDragSettled() {
    const cursor = await cursorPosition().catch(() => null);
    const raw =
      (cursor && (await monitorFromPoint(cursor.x, cursor.y).catch(() => null))) || (await currentMonitor());
    if (!raw) return;
    const monitor = toLogicalMonitor(raw);

    const x = cursor ? clamp((cursor.x - raw.position.x) / monitor.scale, 0, monitor.width) : monitor.width / 2;
    const y = cursor ? clamp((cursor.y - raw.position.y) / monitor.scale, 0, monitor.height) : 0;
    const previousEdge = edge;
    edge = nearestEdge(x, y, monitor.width, monitor.height);
    offsetCenter = isVertical(edge) ? y : x;
    if (edge === previousEdge) {
      // Same edge: keep the spot the tab was grabbed by under the cursor rather than
      // re-centering on it. (After switching edges the tab turns, so there's no such spot.)
      const size = collapsedSize(edge);
      const alongSize = isVertical(edge) ? size.height : size.width;
      const grabAlong = (isVertical(edge) ? dragGrab.y : dragGrab.x) / dragScale;
      offsetCenter += alongSize / 2 - grabAlong;
    }

    applyEdgeClass();
    await placeTab(monitor);
    invoke("save_position", { edge, offset: offsetCenter, monitor: monitor.name }).catch(() => {});
  }

  // --- Usage ---------------------------------------------------------------------------

  function renderUsage(dto) {
    lastUsageDto = dto;
    const { fraction, level, tabText } = sessionSummary(dto);
    tabPercent.textContent = tabText;
    tabRing.dataset.level = level;
    tabRingProgress.style.strokeDashoffset = String(RING_CIRCUMFERENCE * (1 - fraction));
  }

  async function refreshUsage() {
    try {
      renderUsage(await invoke("get_usage"));
    } catch (err) {
      console.error("failed to refresh usage", err);
    }
  }

  // The token-count fallback's ring tracks elapsed time in the 5h window, so it keeps
  // moving between polls.
  setInterval(() => {
    if (lastUsageDto) renderUsage(lastUsageDto);
  }, 1000);
  setInterval(refreshUsage, USAGE_POLL_MS);

  // --- Boot --------------------------------------------------------------------------

  function applySettings(settings) {
    edge = settings.edge;
    offsetCenter = settings.offset_along_edge;
    applyEdgeClass();
  }

  // Fired by the tray's "Redefinir posição" action: the Rust side already moved (and
  // resized) the real window, this just keeps our local edge/offsetCenter from going
  // stale, since they drive the next hover or drag — without this, the notch would
  // silently "snap back" to the old position the next time either of those runs.
  listen("notch-position-reset", (event) => {
    applySettings(event.payload);
    if (isExpanded) collapse();
  });

  (async () => {
    try {
      applySettings(await invoke("get_settings"));
    } catch (err) {
      console.error("failed to load settings", err);
    }
    refreshUsage();
  })();
})();
