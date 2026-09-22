(() => {
  const { invoke } = window.__TAURI__.core;
  const { getCurrentWindow } = window.__TAURI__.window;

  const selfWindow = getCurrentWindow();
  const rectEl = document.getElementById("selection-rect");
  const labelEl = document.getElementById("dimension-label");
  const hintEl = document.getElementById("hint");

  // The Rust side captured monitors and expects the final coordinates in absolute screen
  // *physical* pixels, matching how it positioned this window. Mouse events here report
  // CSS (logical) pixels relative to this window's own top-left, so both an origin offset
  // and a devicePixelRatio scale are needed to convert.
  let originX = 0;
  let originY = 0;
  selfWindow.outerPosition().then((pos) => {
    originX = pos.x;
    originY = pos.y;
  });

  let startX = null;
  let startY = null;
  let dragging = false;

  function rectFrom(x1, y1, x2, y2) {
    return {
      left: Math.min(x1, x2),
      top: Math.min(y1, y2),
      width: Math.abs(x2 - x1),
      height: Math.abs(y2 - y1),
    };
  }

  function updateRect(curX, curY) {
    const r = rectFrom(startX, startY, curX, curY);
    rectEl.style.left = `${r.left}px`;
    rectEl.style.top = `${r.top}px`;
    rectEl.style.width = `${r.width}px`;
    rectEl.style.height = `${r.height}px`;

    labelEl.textContent = `${Math.round(r.width)} × ${Math.round(r.height)}`;
    labelEl.style.left = `${r.left + r.width + 8}px`;
    labelEl.style.top = `${Math.max(r.top - 24, 4)}px`;
  }

  window.addEventListener("mousedown", (event) => {
    if (event.button !== 0) return;
    dragging = true;
    startX = event.clientX;
    startY = event.clientY;
    hintEl.style.display = "none";
    rectEl.style.display = "block";
    labelEl.style.display = "block";
    updateRect(event.clientX, event.clientY);
  });

  window.addEventListener("mousemove", (event) => {
    if (!dragging) return;
    updateRect(event.clientX, event.clientY);
  });

  window.addEventListener("mouseup", async (event) => {
    if (!dragging) return;
    dragging = false;

    const r = rectFrom(startX, startY, event.clientX, event.clientY);
    if (r.width < 3 || r.height < 3) {
      await invoke("cancel_selection");
      return;
    }

    const scale = window.devicePixelRatio || 1;
    await invoke("finish_selection", {
      x: Math.round(originX + r.left * scale),
      y: Math.round(originY + r.top * scale),
      width: Math.round(r.width * scale),
      height: Math.round(r.height * scale),
    });
  });

  window.addEventListener("keydown", async (event) => {
    if (event.key === "Escape") {
      await invoke("cancel_selection");
    }
  });
})();
