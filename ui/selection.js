(() => {
  const { invoke } = window.__TAURI__.core;
  const { getCurrentWindow } = window.__TAURI__.window;

  const selfWindow = getCurrentWindow();

  // Annotation style: fixed for now. Sizes are in CSS pixels and get scaled to the layer's
  // physical resolution when drawn, so they look the same on a 100% and a 150% display.
  const ANNOTATION_COLOR = "#ff2d2d";
  const RECT_LINE_WIDTH = 3;
  const TEXT_SIZE = 20;
  const TEXT_OUTLINE_WIDTH = 4;
  const TEXT_FONT_FAMILY = '"Segoe UI Variable", "Segoe UI", system-ui, sans-serif';
  const MIN_SIZE = 3;

  const hintEl = document.getElementById("hint");
  const rectEl = document.getElementById("selection-rect");
  const labelEl = document.getElementById("dimension-label");
  const frozenEl = document.getElementById("frozen");
  const layer = document.getElementById("layer");
  const ctx = layer.getContext("2d");
  const textInput = document.getElementById("text-input");
  const toolbar = document.getElementById("toolbar");
  const toolButtons = [...toolbar.querySelectorAll("[data-tool]")];

  // The Rust side captured monitors and expects coordinates in absolute screen *physical*
  // pixels, matching how it positioned this window. Mouse events here report CSS (logical)
  // pixels relative to this window's own top-left, so both an origin offset and a
  // devicePixelRatio scale are needed to convert.
  let originX = 0;
  let originY = 0;
  selfWindow.outerPosition().then((pos) => {
    originX = pos.x;
    originY = pos.y;
  });

  // "select" → drag out a region; "annotate" → draw on it; "busy" → waiting on Rust.
  let mode = "select";
  let startX = null;
  let startY = null;
  let dragging = false;

  let region = null; // selected region, CSS px within this window
  let scaleX = 1; // layer (physical) px per CSS px
  let scaleY = 1;
  let tool = "rect";
  let shapes = [];
  let draft = null; // rectangle being dragged out, layer px
  let textAt = null; // where the text being typed goes, layer px

  function rectFrom(x1, y1, x2, y2) {
    return {
      left: Math.min(x1, x2),
      top: Math.min(y1, y2),
      width: Math.abs(x2 - x1),
      height: Math.abs(y2 - y1),
    };
  }

  function place(el, r) {
    el.style.left = `${r.left}px`;
    el.style.top = `${r.top}px`;
    el.style.width = `${r.width}px`;
    el.style.height = `${r.height}px`;
  }

  // --- Select --------------------------------------------------------------------------

  function updateRect(curX, curY) {
    const r = rectFrom(startX, startY, curX, curY);
    place(rectEl, r);
    labelEl.textContent = `${Math.round(r.width)} × ${Math.round(r.height)}`;
    labelEl.style.left = `${r.left + r.width + 8}px`;
    labelEl.style.top = `${Math.max(r.top - 24, 4)}px`;
  }

  window.addEventListener("mousedown", (event) => {
    if (mode !== "select" || event.button !== 0) return;
    dragging = true;
    startX = event.clientX;
    startY = event.clientY;
    hintEl.style.display = "none";
    rectEl.style.display = "block";
    labelEl.style.display = "block";
    updateRect(event.clientX, event.clientY);
  });

  window.addEventListener("mousemove", (event) => {
    if (mode !== "select" || !dragging) return;
    updateRect(event.clientX, event.clientY);
  });

  window.addEventListener("mouseup", (event) => {
    if (mode !== "select" || !dragging) return;
    dragging = false;

    const r = rectFrom(startX, startY, event.clientX, event.clientY);
    if (r.width < MIN_SIZE || r.height < MIN_SIZE) {
      invoke("cancel_selection");
      return;
    }
    enterAnnotate(r);
  });

  // --- Annotate ------------------------------------------------------------------------

  async function enterAnnotate(r) {
    mode = "busy";
    const scale = window.devicePixelRatio || 1;
    const physical = {
      x: Math.round(originX + r.left * scale),
      y: Math.round(originY + r.top * scale),
      width: Math.round(r.width * scale),
      height: Math.round(r.height * scale),
    };

    let png;
    try {
      png = await invoke("preview_selection", physical);
    } catch (err) {
      hintEl.textContent = `Erro: ${err} · Esc para sair`;
      hintEl.style.display = "block";
      return;
    }

    // The crop is shown frozen exactly where it was, so it doesn't matter if what's live
    // underneath changes while annotating — and what's drawn lines up with what's saved.
    frozenEl.src = URL.createObjectURL(new Blob([png], { type: "image/png" }));
    await frozenEl.decode().catch(() => {});

    region = r;
    layer.width = physical.width;
    layer.height = physical.height;
    scaleX = physical.width / r.width;
    scaleY = physical.height / r.height;
    place(frozenEl, r);
    place(layer, r);
    frozenEl.style.display = "block";
    layer.style.display = "block";
    labelEl.style.display = "none";
    document.body.classList.add("annotating");
    toolbar.style.display = "flex";
    positionToolbar(r);
    setTool("rect");
    mode = "annotate";
  }

  // Under the region, or above it if there's no room, or inside it at the bottom if
  // neither fits (a selection covering the whole screen).
  function positionToolbar(r) {
    const tb = toolbar.getBoundingClientRect();
    const gap = 10;
    let top = r.top + r.height + gap;
    if (top + tb.height > window.innerHeight - 8) top = r.top - tb.height - gap;
    if (top < 8) top = r.top + r.height - tb.height - gap;
    const left = Math.min(Math.max(r.left + r.width - tb.width, 8), window.innerWidth - tb.width - 8);
    toolbar.style.top = `${top}px`;
    toolbar.style.left = `${left}px`;
  }

  function setTool(next) {
    commitText();
    tool = next;
    for (const button of toolButtons) button.classList.toggle("active", button.dataset.tool === next);
    layer.classList.toggle("tool-text", next === "text");
  }

  function toLayer(event) {
    return {
      x: Math.min(Math.max((event.clientX - region.left) * scaleX, 0), layer.width),
      y: Math.min(Math.max((event.clientY - region.top) * scaleY, 0), layer.height),
    };
  }

  function normalized(r) {
    return {
      x: Math.min(r.x, r.x + r.w),
      y: Math.min(r.y, r.y + r.h),
      w: Math.abs(r.w),
      h: Math.abs(r.h),
    };
  }

  function textFont() {
    return `700 ${TEXT_SIZE * scaleY}px ${TEXT_FONT_FAMILY}`;
  }

  function drawShape(shape) {
    if (shape.type === "rect") {
      ctx.lineWidth = RECT_LINE_WIDTH * scaleX;
      ctx.lineJoin = "miter";
      ctx.strokeStyle = ANNOTATION_COLOR;
      ctx.strokeRect(shape.x, shape.y, shape.w, shape.h);
    } else {
      ctx.font = textFont();
      ctx.textBaseline = "top";
      // White outline first so the red text reads on any background.
      ctx.lineJoin = "round";
      ctx.lineWidth = TEXT_OUTLINE_WIDTH * scaleY;
      ctx.strokeStyle = "#fff";
      ctx.strokeText(shape.text, shape.x, shape.y);
      ctx.fillStyle = ANNOTATION_COLOR;
      ctx.fillText(shape.text, shape.x, shape.y);
    }
  }

  function redraw() {
    ctx.clearRect(0, 0, layer.width, layer.height);
    for (const shape of shapes) drawShape(shape);
    if (draft) drawShape({ type: "rect", ...normalized(draft) });
  }

  layer.addEventListener("pointerdown", (event) => {
    if (mode !== "annotate" || event.button !== 0) return;
    const p = toLayer(event);
    if (tool === "rect") {
      commitText();
      draft = { x: p.x, y: p.y, w: 0, h: 0 };
      layer.setPointerCapture(event.pointerId);
    } else {
      commitText();
      openText(p, event.clientX, event.clientY);
      // Otherwise the pointerdown's default focus handling would pull focus straight
      // back off the input that was just opened.
      event.preventDefault();
    }
  });

  layer.addEventListener("pointermove", (event) => {
    if (!draft) return;
    const p = toLayer(event);
    draft.w = p.x - draft.x;
    draft.h = p.y - draft.y;
    redraw();
  });

  function endDraft() {
    if (!draft) return;
    const r = normalized(draft);
    draft = null;
    if (r.w >= MIN_SIZE * scaleX && r.h >= MIN_SIZE * scaleY) {
      shapes.push({ type: "rect", ...r });
    }
    redraw();
  }
  layer.addEventListener("pointerup", endDraft);
  layer.addEventListener("pointercancel", endDraft);

  function openText(p, clientX, clientY) {
    textAt = p;
    textInput.value = "";
    textInput.style.width = "3ch";
    textInput.style.left = `${clientX}px`;
    // The input's line box has half-leading above the glyphs (line-height 1.2); shift it
    // up by that much so the typed text sits where the canvas will draw it (baseline "top").
    textInput.style.top = `${clientY - TEXT_SIZE * 0.1}px`;
    textInput.style.display = "block";
    textInput.focus();
  }

  function commitText() {
    if (!textAt) return;
    const text = textInput.value.trim();
    if (text) shapes.push({ type: "text", x: textAt.x, y: textAt.y, text });
    textAt = null;
    textInput.style.display = "none";
    redraw();
  }

  function cancelText() {
    textAt = null;
    textInput.style.display = "none";
  }

  textInput.addEventListener("input", () => {
    textInput.style.width = `${Math.max(textInput.value.length + 1, 3)}ch`;
  });

  textInput.addEventListener("keydown", (event) => {
    event.stopPropagation();
    if (event.key === "Enter") commitText();
    else if (event.key === "Escape") cancelText();
  });
  textInput.addEventListener("blur", commitText);

  function undo() {
    shapes.pop();
    redraw();
  }

  async function finish() {
    if (mode !== "annotate") return;
    commitText();
    mode = "busy";
    // Only the annotation layer goes back, as raw PNG bytes — Rust lays it over the crop
    // it kept, so the screenshot's own pixels never round-trip through the webview.
    let body = new Uint8Array(0);
    if (shapes.length > 0) {
      const blob = await new Promise((resolve) => layer.toBlob(resolve, "image/png"));
      body = new Uint8Array(await blob.arrayBuffer());
    }
    await invoke("finish_annotated", body);
  }

  for (const button of toolButtons) {
    button.addEventListener("click", () => setTool(button.dataset.tool));
  }
  document.getElementById("undo-btn").addEventListener("click", undo);
  document.getElementById("done-btn").addEventListener("click", finish);
  document.getElementById("cancel-btn").addEventListener("click", () => invoke("cancel_selection"));
  // Keeps a click on the toolbar from blurring the text input before the button's own
  // click handler runs (which would commit the text as a side effect anyway).
  toolbar.addEventListener("mousedown", (event) => event.preventDefault());

  window.addEventListener("keydown", (event) => {
    if (event.key === "Escape") {
      invoke("cancel_selection");
      return;
    }
    if (mode !== "annotate") return;
    if (event.key === "Enter") {
      finish();
    } else if ((event.ctrlKey || event.metaKey) && event.key.toLowerCase() === "z") {
      event.preventDefault();
      undo();
    } else if (event.key === "r" || event.key === "R") {
      setTool("rect");
    } else if (event.key === "t" || event.key === "T") {
      setTool("text");
    }
  });
})();
