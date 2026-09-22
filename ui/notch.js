(() => {
  const { invoke } = window.__TAURI__.core;
  const { listen } = window.__TAURI__.event;
  const { getCurrentWindow, LogicalPosition, LogicalSize, currentMonitor } = window.__TAURI__.window;

  const appWindow = getCurrentWindow();

  // Sizes and all the position/drag math below are in *logical* pixels throughout, so the
  // notch looks the same real-world size on a 100% and a 150%/200% scaled Windows display.
  // Monitor geometry and window-move events from Tauri are physical, so they're converted
  // via the monitor's scaleFactor at the one place each is read (toLogicalMonitor,
  // onDragSettled) — everything past that boundary stays logical.
  // COLLAPSED_SIZE is kept in sync by hand with COLLAPSED_SIZE in src-tauri/src/config.rs.
  const COLLAPSED_SIZE = { width: 120, height: 28 };
  const EXPANDED_SIZE = { width: 340, height: 210 };
  const HOVER_EXPAND_DELAY = 120;
  const HOVER_COLLAPSE_DELAY = 350;
  const SNAP_MARGIN = 48; // logical px
  const USAGE_POLL_MS = 8000;
  const USAGE_WINDOW_MS = 5 * 60 * 60 * 1000;
  const RING_CIRCUMFERENCE = 2 * Math.PI * 27;

  const notchEl = document.getElementById("notch");
  const statusDot = document.getElementById("status-dot");
  const headerDot = document.getElementById("header-dot");
  const usagePrimary = document.getElementById("usage-primary");
  const usageSecondary = document.getElementById("usage-secondary");
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
  let dragSettleTimer = null;
  let lastUsageDto = null;

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
    if (!monitor) return;
    const pos = computeWindowRect(monitor, size);
    await appWindow.setSize(new LogicalSize(size.width, size.height));
    await appWindow.setPosition(new LogicalPosition(pos.x, pos.y));
  }

  async function expand() {
    expandTimer = null;
    if (isExpanded) return;
    isExpanded = true;
    await moveAndResize(EXPANDED_SIZE);
    requestAnimationFrame(() => notchEl.classList.add("expanded"));
    refreshUsage();
  }

  function collapse() {
    if (!isExpanded || isDragging) return;
    isExpanded = false;
    notchEl.classList.remove("expanded");
    notchEl.addEventListener("transitionend", onCollapseTransitionEnd, { once: true });
  }

  async function onCollapseTransitionEnd(event) {
    if (event.propertyName !== "width") return;
    await moveAndResize(COLLAPSED_SIZE);
  }

  notchEl.addEventListener("mouseenter", () => {
    clearTimeout(collapseTimer);
    collapseTimer = null;
    if (isExpanded || expandTimer) return;
    expandTimer = setTimeout(expand, HOVER_EXPAND_DELAY);
  });

  notchEl.addEventListener("mouseleave", () => {
    clearTimeout(expandTimer);
    expandTimer = null;
    if (!isExpanded || isDragging) return;
    collapseTimer = setTimeout(collapse, HOVER_COLLAPSE_DELAY);
  });

  // --- Drag to reposition + edge snap -------------------------------------------------

  // Listens on the whole notch (not just #pill): hovering for HOVER_EXPAND_DELAY (120ms)
  // before the user manages to press the button is the common case, not the exception, so
  // by the time mousedown would fire on #pill it's usually already hidden behind the
  // expanded panel (pointer-events: none). Dragging from the panel background works too;
  // only the capture button opts out, so it can still be clicked normally.
  notchEl.addEventListener("mousedown", async (event) => {
    if (event.button !== 0 || event.target.closest("#capture-btn")) return;
    clearTimeout(expandTimer);
    expandTimer = null;
    clearTimeout(collapseTimer);
    collapseTimer = null;

    // Drag math (here and in onDragSettled) assumes the window's physical footprint is
    // COLLAPSED_SIZE throughout — force that *before* the native drag starts rather than
    // only after, so a drag begun from the expanded panel doesn't run with mismatched size.
    if (isExpanded) {
      isExpanded = false;
      notchEl.classList.remove("expanded");
      await moveAndResize(COLLAPSED_SIZE);
    }

    isDragging = true;
    await appWindow.startDragging();
  });

  appWindow.onMoved(({ payload: position }) => {
    if (!isDragging) return;
    clearTimeout(dragSettleTimer);
    dragSettleTimer = setTimeout(() => onDragSettled(position), 150);
  });

  async function onDragSettled(physicalPosition) {
    isDragging = false;
    const monitor = await logicalMonitor();
    if (!monitor) return;

    // `onMoved` reports physical pixels (it mirrors the OS window manager); convert to
    // logical so it lines up with `monitor` and every other value in this file.
    const position = { x: physicalPosition.x / monitor.scale, y: physicalPosition.y / monitor.scale };
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

  function setDotState(state) {
    statusDot.dataset.state = state;
    headerDot.dataset.state = state;
  }

  function renderUsage(dto) {
    lastUsageDto = dto;
    setDotState(dto.status);

    switch (dto.status) {
      case "loading":
        usagePrimary.textContent = "Carregando…";
        usageSecondary.textContent = "";
        usageFootnote.textContent = "";
        ringProgress.style.strokeDashoffset = RING_CIRCUMFERENCE;
        break;
      case "unavailable":
        usagePrimary.textContent = "Sem dados locais";
        usageSecondary.textContent = dto.reason ?? "";
        usageFootnote.textContent = "";
        ringProgress.style.strokeDashoffset = RING_CIRCUMFERENCE;
        break;
      case "idle":
        usagePrimary.textContent = "Sem sessão ativa";
        usageSecondary.textContent = "nas últimas 5h";
        usageFootnote.textContent = "";
        ringProgress.style.strokeDashoffset = RING_CIRCUMFERENCE;
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

        const fraction = Math.min(Math.max(dto.percent / 100, 0), 1);
        ringProgress.style.strokeDashoffset = String(RING_CIRCUMFERENCE * (1 - fraction));
        break;
      }
      case "active": {
        usagePrimary.textContent = `${formatTokens(dto.tokens)} tokens`;
        const remainingMs = new Date(dto.resets_at).getTime() - Date.now();
        usageSecondary.textContent = `reinicia em ${formatDuration(remainingMs)}`;
        usageFootnote.textContent = "Estimativa derivada dos logs locais, não é o limite oficial do plano.";

        const elapsedFraction = Math.min(Math.max(1 - remainingMs / USAGE_WINDOW_MS, 0), 1);
        ringProgress.style.strokeDashoffset = String(RING_CIRCUMFERENCE * (1 - elapsedFraction));
        break;
      }
    }

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

  (async () => {
    try {
      const settings = await invoke("get_settings");
      edge = settings.edge;
      offsetCenter = settings.offset_along_edge;
    } catch (err) {
      console.error("failed to load settings", err);
    }
    applyEdgeClass();
    refreshUsage();
  })();
})();
