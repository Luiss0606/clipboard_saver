const { invoke, Channel } = window.__TAURI__.core;
const { listen } = window.__TAURI__.event;

const $ = (id) => document.getElementById(id);
const listEl = $("list");
const emptyEl = $("empty");
const searchEl = $("search");
const actionBarEl = $("action-bar");
const selCountEl = $("sel-count");

let state = { items: [], autostart: false, version: "dev", pendingUpdate: null, maxItems: 40 };
let filtered = [];
let selected = 0;
// Ordered by click time — sent to backend in this order for concatenation.
let selectedIds = [];
// Type filter: "all" | "text" | "image".
let typeFilter = "all";
// Last item picked individually — the pivot for Shift+click range selection.
let anchorId = null;

const LINK_SVG =
  '<svg viewBox="0 0 16 16"><path d="M8 1.5a6.5 6.5 0 1 0 0 13 6.5 6.5 0 0 0 0-13Zm4.9 6H10.8a10.6 10.6 0 0 0-1-4.2A5.3 5.3 0 0 1 12.9 7.5ZM8 2.8c.6.7 1.3 2.1 1.5 4.7h-3C6.7 4.9 7.4 3.5 8 2.8ZM3.1 8.5h2.1a10.6 10.6 0 0 0 1 4.2 5.3 5.3 0 0 1-3.1-4.2Zm2.1-1H3.1a5.3 5.3 0 0 1 3.1-4.2 10.6 10.6 0 0 0-1 4.2ZM8 13.2c-.6-.7-1.3-2.1-1.5-4.7h3c-.2 2.6-.9 4-1.5 4.7Zm1.8-.5a10.6 10.6 0 0 0 1-4.2h2.1a5.3 5.3 0 0 1-3.1 4.2Z"/></svg>';

function toggleIdInSelection(id) {
  const idx = selectedIds.indexOf(id);
  if (idx === -1) {
    selectedIds.push(id);
  } else {
    selectedIds.splice(idx, 1);
  }
  updateActionBar();
}

// Adds every item between the anchor and `targetId` (inclusive, visual order)
// to the selection, then moves the anchor to the target. Without a valid
// anchor it just selects the target and seeds it as the new anchor.
function selectRange(targetId) {
  const aIdx = anchorId === null ? -1 : filtered.findIndex((i) => i.id === anchorId);
  const tIdx = filtered.findIndex((i) => i.id === targetId);
  if (tIdx === -1) return;
  if (aIdx === -1) {
    if (!selectedIds.includes(targetId)) selectedIds.push(targetId);
  } else {
    for (let i = Math.min(aIdx, tIdx); i <= Math.max(aIdx, tIdx); i++) {
      const id = filtered[i].id;
      if (!selectedIds.includes(id)) selectedIds.push(id);
    }
  }
  anchorId = targetId;
  updateActionBar();
}

function clearSelection() {
  selectedIds = [];
  anchorId = null;
  updateActionBar();
  [...listEl.children].forEach((el) => el.classList.remove("checked"));
  [...listEl.querySelectorAll(".row-check")].forEach((cb) => (cb.checked = false));
}

function updateActionBar() {
  const n = selectedIds.length;
  actionBarEl.classList.toggle("visible", n > 0);
  if (n > 0) {
    selCountEl.textContent = `${n} elemento${n === 1 ? "" : "s"} seleccionado${n === 1 ? "" : "s"}`;
    $("copy-selected").textContent = n === 1 ? "Copiar" : `Copiar ${n}`;
    $("delete-selected").textContent = n === 1 ? "Eliminar" : `Eliminar ${n}`;
  }
}

function render() {
  const q = searchEl.value.trim().toLowerCase();
  filtered = state.items.filter(
    (it) =>
      (typeFilter === "all" || it.kind === typeFilter) &&
      (!q || it.preview.toLowerCase().includes(q))
  );
  if (selected >= filtered.length) selected = Math.max(0, filtered.length - 1);

  listEl.innerHTML = "";
  emptyEl.classList.toggle("hidden", state.items.length > 0);
  listEl.classList.toggle("hidden", state.items.length === 0);

  filtered.forEach((item, idx) => {
    const isChecked = selectedIds.includes(item.id);
    const isImage = item.kind === "image" && item.thumb;
    const row = document.createElement("div");
    row.className =
      "row" +
      (isImage ? " row-image" : "") +
      (idx === selected ? " selected" : "") +
      (isChecked ? " checked" : "");
    row.setAttribute("role", "option");

    const key = idx < 9 ? `<span class="row-key">⌘${idx + 1}</span>` : "";
    const checkInput = `<input type="checkbox" class="row-check" ${
      isChecked ? "checked" : ""
    } tabindex="-1" />`;

    if (isImage) {
      // Content-first card: large contained preview, dimensions demoted to caption.
      // Reserve the frame via aspect-ratio (parsed from "Imagen W×H") so the
      // image load doesn't shift the masonry layout.
      const dims = item.preview.match(/(\d+)\s*[×x]\s*(\d+)/);
      const ratio = dims ? `${dims[1]} / ${dims[2]}` : "4 / 3";
      row.innerHTML = `
        ${checkInput}
        <div class="img-frame" style="aspect-ratio:${ratio}"><img class="img-preview" src="${item.thumb}" alt="" /></div>
        <div class="card-foot">
          <span class="row-meta"></span>
          ${key}
        </div>`;
      row.querySelector(".row-meta").textContent = `${item.preview} · ${item.ago}`;
      const img = row.querySelector(".img-preview");
      img.addEventListener("load", layoutMasonry);
    } else {
      const chip = item.isUrl
        ? `<div class="chip url">${LINK_SVG}</div>`
        : '<div class="chip text">Aa</div>';
      row.innerHTML = `
        ${checkInput}
        <div class="card-head">${chip}</div>
        <div class="row-text"></div>
        <div class="card-foot">
          <span class="row-meta">${item.ago}</span>
          ${key}
        </div>`;
      row.querySelector(".row-text").textContent = item.preview;
    }

    const checkbox = row.querySelector(".row-check");

    checkbox.addEventListener("click", (e) => {
      e.stopPropagation();
      toggleIdInSelection(item.id);
      anchorId = item.id;
      row.classList.toggle("checked", selectedIds.includes(item.id));
    });

    row.addEventListener("click", (e) => {
      if (e.target === checkbox) return;
      if (e.shiftKey) {
        e.preventDefault();
        selectRange(item.id);
        render();
        return;
      }
      if (selectedIds.length > 0) {
        toggleIdInSelection(item.id);
        anchorId = item.id;
        row.classList.toggle("checked", selectedIds.includes(item.id));
        checkbox.checked = selectedIds.includes(item.id);
      } else {
        restore(item.id);
      }
    });

    row.addEventListener("mousemove", () => {
      if (selected !== idx) {
        selected = idx;
        updateSelection();
      }
    });

    // Native drag-out: drop the card into another app (Finder, Notes, editor).
    row.draggable = true;
    row.addEventListener("dragstart", (e) => {
      e.preventDefault(); // hand off to the native drag session
      startDragOut(item);
    });

    listEl.appendChild(row);
  });

  $("version").textContent = `Clipboard Saver ${state.version}`;
  $("count").textContent = state.items.length
    ? ` · ${state.items.length} de ${state.maxItems}`
    : "";
  $("autostart").checked = state.autostart;
  $("clear").disabled = state.items.length === 0;

  const banner = $("update-banner");
  banner.classList.toggle("hidden", !state.pendingUpdate);
  if (state.pendingUpdate) {
    $("update-text").textContent = `Nueva versión ${state.pendingUpdate} disponible`;
  }

  updateActionBar();
  layoutMasonry();
}

// Masonry: each card spans as many 1px grid rows as its height (plus the
// vertical gap), so variable-height cards pack without leaving gaps.
const MASONRY_GAP = 10;
function layoutMasonry() {
  for (const card of listEl.children) {
    const h = card.getBoundingClientRect().height;
    card.style.gridRowEnd = `span ${Math.ceil(h) + MASONRY_GAP}`;
  }
}

function updateSelection() {
  [...listEl.children].forEach((el, idx) =>
    el.classList.toggle("selected", idx === selected)
  );
}

async function refresh() {
  state = await invoke("get_state");
  render();
}

async function restore(id) {
  await invoke("restore_item", { id });
}

// Draws a small labelled chip to use as the drag ghost for text items
// (the native drag API requires a preview image).
function textDragPreview(text) {
  const scale = window.devicePixelRatio || 2;
  const w = 240;
  const h = 40;
  const canvas = document.createElement("canvas");
  canvas.width = w * scale;
  canvas.height = h * scale;
  const ctx = canvas.getContext("2d");
  ctx.scale(scale, scale);
  ctx.fillStyle = "rgba(40, 40, 40, 0.92)";
  ctx.fillRect(0, 0, w, h);
  ctx.fillStyle = "#fff";
  ctx.font = '13px -apple-system, "SF Pro Text", sans-serif';
  ctx.textBaseline = "middle";
  const line = text.replace(/\s+/g, " ").trim().slice(0, 34);
  ctx.fillText(line, 12, h / 2);
  return canvas.toDataURL("image/png");
}

// Starts a native drag-out session for the given item. The panel is kept
// open during the drag (set_dragging guard) and hidden once it ends.
async function startDragOut(item) {
  const data = await invoke("drag_data", { id: item.id });
  if (!data) return;

  let dragItem;
  let image;
  if (data.kind === "image") {
    dragItem = [data.path];
    image = item.thumb; // already a data:image/png;base64 URL
  } else {
    dragItem = { data: data.text, types: ["public.utf8-plain-text"] };
    image = textDragPreview(data.text);
  }

  await invoke("set_dragging", { on: true });

  const onEvent = new Channel();
  onEvent.onmessage = () => {
    invoke("set_dragging", { on: false });
    invoke("hide_panel");
  };

  try {
    await invoke("plugin:drag|start_drag", {
      item: dragItem,
      image,
      options: { mode: "copy" },
      onEvent,
    });
  } catch (err) {
    console.error("drag failed", err);
    invoke("set_dragging", { on: false });
  }
}

async function copySelected() {
  if (selectedIds.length === 0) return;
  const btn = $("copy-selected");
  const originalText = btn.textContent;
  await invoke("copy_selected", { ids: selectedIds });
  clearSelection();
  btn.textContent = "✓ Copiado";
  btn.classList.add("success");
  setTimeout(() => {
    btn.textContent = originalText;
    btn.classList.remove("success");
  }, 1500);
}

async function deleteSelected() {
  if (selectedIds.length === 0) return;
  const ids = [...selectedIds];
  clearSelection();
  await invoke("delete_items", { ids });
}

searchEl.addEventListener("input", () => {
  selected = 0;
  clearSelection();
  render();
});

function setTypeFilter(filter) {
  if (filter === typeFilter) return;
  typeFilter = filter;
  [...$("type-filter").children].forEach((btn) => {
    const active = btn.dataset.filter === filter;
    btn.classList.toggle("active", active);
    btn.setAttribute("aria-selected", active ? "true" : "false");
  });
  selected = 0;
  clearSelection();
  render();
}

$("type-filter").addEventListener("click", (e) => {
  const btn = e.target.closest(".seg");
  if (btn) setTypeFilter(btn.dataset.filter);
});

document.addEventListener("keydown", (e) => {
  if (e.key === "Escape") {
    if (selectedIds.length > 0) {
      clearSelection();
      render();
    } else {
      invoke("hide_panel");
    }
    return;
  }
  if (e.key === "ArrowDown" || e.key === "ArrowUp") {
    e.preventDefault();
    const delta = e.key === "ArrowDown" ? 1 : -1;
    selected = Math.min(Math.max(selected + delta, 0), filtered.length - 1);
    updateSelection();
    listEl.children[selected]?.scrollIntoView({ block: "nearest" });
    return;
  }
  if (e.key === " " && filtered[selected]) {
    e.preventDefault();
    toggleIdInSelection(filtered[selected].id);
    anchorId = filtered[selected].id;
    render();
    return;
  }
  if (e.key === "Enter" && filtered[selected]) {
    if (selectedIds.length > 0) {
      copySelected();
    } else {
      restore(filtered[selected].id);
    }
    return;
  }
  if ((e.key === "Delete" || e.key === "Backspace") && selectedIds.length > 0) {
    e.preventDefault();
    deleteSelected();
    return;
  }
  if (e.metaKey && e.key === "c" && selectedIds.length > 0) {
    e.preventDefault();
    copySelected();
    return;
  }
  if (e.metaKey && e.key >= "1" && e.key <= "9") {
    const item = filtered[Number(e.key) - 1];
    if (item) {
      e.preventDefault();
      restore(item.id);
    }
  }
});

$("copy-selected").addEventListener("click", copySelected);
$("delete-selected").addEventListener("click", deleteSelected);
$("clear").addEventListener("click", () => {
  clearSelection();
  invoke("clear_history");
});
$("quit").addEventListener("click", () => invoke("quit_app"));
$("autostart").addEventListener("change", () => invoke("toggle_autostart"));
$("update-btn").addEventListener("click", async () => {
  $("update-btn").textContent = "Instalando…";
  try {
    await invoke("install_update");
  } catch (err) {
    $("update-text").textContent = `Error: ${err}`;
    $("update-btn").textContent = "Reintentar";
  }
});

window.addEventListener("focus", () => {
  searchEl.value = "";
  selected = 0;
  clearSelection();
  setTypeFilter("all");
  searchEl.focus();
  refresh();
});

window.addEventListener("resize", layoutMasonry);

listen("state-changed", refresh);
refresh();
searchEl.focus();
