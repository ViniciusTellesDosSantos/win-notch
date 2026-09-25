(() => {
  const { invoke } = window.__TAURI__.core;
  const { listen, emit } = window.__TAURI__.event;
  const { getCurrentWindow } = window.__TAURI__.window;
  const U = window.UsageFormat;

  // This window is positioned, opened and parked entirely by notch.js (the tab); it only
  // renders, fades in/out on request, and reports hover back so the tab knows not to close
  // it while the pointer is over it.
  const appWindow = getCurrentWindow();

  const USAGE_POLL_MS = 8000;
  const ALWAYS_ON_TOP_REASSERT_MS = 3000;
  const CAPTURE_NOTE_MS = 3000;

  const rootEl = document.getElementById("root");
  const sessionReset = document.getElementById("session-reset");
  const sessionFill = document.getElementById("session-fill");
  const sessionUsed = document.getElementById("session-used");
  const weeklyLine = document.getElementById("weekly-line");
  const weeklyReset = document.getElementById("weekly-reset");
  const weeklyFill = document.getElementById("weekly-fill");
  const weeklyUsed = document.getElementById("weekly-used");
  const usageFootnote = document.getElementById("usage-footnote");
  const captureBtn = document.getElementById("capture-btn");
  const captureList = document.getElementById("capture-list");
  const openFolderBtn = document.getElementById("open-folder");
  const updateBtn = document.getElementById("update-btn");

  let lastUsageDto = null;
  let captureNote = null;

  // See the matching comment in notch.js: topmost has to be re-asserted, not set once.
  setInterval(() => {
    appWindow.setAlwaysOnTop(true).catch(() => {});
  }, ALWAYS_ON_TOP_REASSERT_MS);

  // --- Open / close / hover --------------------------------------------------------------

  listen("popover-open", (event) => {
    const { edge, tailOffset } = event.payload;
    rootEl.classList.remove("edge-top", "edge-bottom", "edge-left", "edge-right");
    rootEl.classList.add(`edge-${edge}`);
    rootEl.style.setProperty("--tail-offset", `${tailOffset}px`);
    if (lastUsageDto) renderUsage(lastUsageDto);
    refreshUsage();
    refreshCaptures();
    requestAnimationFrame(() => rootEl.classList.add("open"));
  });

  listen("popover-close", () => {
    rootEl.classList.remove("open");
  });

  rootEl.addEventListener("mouseenter", () => {
    emit("popover-hover", { inside: true });
  });
  rootEl.addEventListener("mouseleave", () => {
    emit("popover-hover", { inside: false });
  });

  // --- Usage ---------------------------------------------------------------------------

  function setFill(el, fraction, level) {
    el.style.width = `${U.clamp(fraction, 0, 1) * 100}%`;
    el.dataset.level = level;
  }

  function renderUsage(dto) {
    lastUsageDto = dto;
    const session = U.sessionSummary(dto);
    let footnote = "";

    sessionReset.textContent = "";
    weeklyLine.hidden = true;

    switch (dto.status) {
      case "loading":
        sessionUsed.textContent = "Carregando…";
        break;
      case "unavailable":
        sessionUsed.textContent = "Sem dados locais";
        footnote = dto.reason ?? "";
        break;
      case "idle":
        sessionUsed.textContent = "Sem sessão ativa nas últimas 5h";
        break;
      case "active_official":
        sessionUsed.textContent = `${Math.round(dto.percent)}% usado`;
        if (dto.resets_at) {
          sessionReset.textContent = U.formatSessionReset(new Date(dto.resets_at).getTime() - Date.now());
        }
        if (dto.weekly_percent != null) {
          weeklyLine.hidden = false;
          weeklyUsed.textContent = `${Math.round(dto.weekly_percent)}% usado`;
          weeklyReset.textContent = dto.weekly_resets_at ? U.formatWeeklyReset(dto.weekly_resets_at) : "";
          setFill(weeklyFill, dto.weekly_percent / 100, U.levelFor(dto.weekly_percent));
        }
        break;
      case "active":
        sessionUsed.textContent = `${U.formatTokens(dto.tokens)} tokens`;
        sessionReset.textContent = U.formatSessionReset(session.remainingMs);
        footnote = "Estimativa pelos logs locais, não é o limite oficial do plano.";
        if (dto.reason) {
          // Why the official percentage wasn't used instead — there's no console in a
          // release build, so this is the only place a failure here is ever visible.
          footnote += ` (debug: ${dto.reason})`;
        }
        break;
    }

    setFill(sessionFill, session.fraction, session.level);

    if (captureNote && Date.now() < captureNote.until) {
      footnote = captureNote.text;
    }
    usageFootnote.textContent = footnote;
    usageFootnote.title = footnote;
  }

  async function refreshUsage() {
    try {
      renderUsage(await invoke("get_usage"));
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
    const { ok, message, saved } = event.payload;
    if (ok && saved) {
      showCaptureNote("Copiado e salvo na pasta de capturas");
    } else if (ok) {
      // Still on the clipboard; only writing the file failed.
      showCaptureNote(`Copiado, mas não salvo: ${message}`);
    } else if (message) {
      showCaptureNote(`Erro: ${message}`);
    } else {
      showCaptureNote(null);
    }
    if (ok) refreshCaptures();
  });

  // --- Recent captures -------------------------------------------------------------------

  function renderCaptures(entries) {
    captureList.replaceChildren();
    if (entries.length === 0) {
      const empty = document.createElement("p");
      empty.className = "captures-empty";
      empty.textContent = "Nenhuma captura ainda";
      captureList.append(empty);
      return;
    }
    for (const entry of entries) {
      const thumb = document.createElement("button");
      thumb.type = "button";
      thumb.className = "capture-thumb";
      thumb.title = `${entry.name} — clique pra copiar de novo`;
      if (entry.thumb) {
        const img = document.createElement("img");
        img.src = entry.thumb;
        img.alt = "";
        thumb.append(img);
      }
      thumb.addEventListener("click", async () => {
        try {
          await invoke("copy_capture", { path: entry.path });
          showCaptureNote("Copiado para a área de transferência");
        } catch (err) {
          showCaptureNote(`Erro: ${err}`);
        }
      });
      captureList.append(thumb);
    }
  }

  async function refreshCaptures() {
    try {
      renderCaptures(await invoke("list_captures"));
    } catch (err) {
      console.error("failed to list captures", err);
    }
  }

  openFolderBtn.addEventListener("click", () => {
    invoke("open_captures_folder").catch((err) => showCaptureNote(`Erro: ${err}`));
  });

  // --- Updates -----------------------------------------------------------------------

  function showUpdate(version) {
    if (!version) return;
    updateBtn.hidden = false;
    updateBtn.disabled = false;
    updateBtn.title = `Atualizar para a versão ${version}`;
  }

  updateBtn.addEventListener("click", async () => {
    updateBtn.disabled = true;
    updateBtn.title = "Baixando atualização…";
    showCaptureNote("Baixando atualização…");
    try {
      // On success the app exits and the installer reopens the new version, so this
      // only comes back if something went wrong.
      await invoke("install_update");
    } catch (err) {
      updateBtn.disabled = false;
      updateBtn.title = "Tentar atualizar de novo";
      showCaptureNote(`Erro ao atualizar: ${err}`);
    }
  });

  listen("update-available", (event) => showUpdate(event.payload?.version));
  invoke("update_status").then(showUpdate, () => {});

  refreshUsage();
  refreshCaptures();
})();
