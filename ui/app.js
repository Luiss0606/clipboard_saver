const { invoke } = window.__TAURI__.core;
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

function clearSelection() {
  selectedIds = [];
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
  filtered = q
    ? state.items.filter((it) => it.preview.toLowerCase().includes(q))
    : state.items;
  if (selected >= filtered.length) selected = Math.max(0, filtered.length - 1);

  listEl.innerHTML = "";
  emptyEl.classList.toggle("hidden", state.items.length > 0);
  listEl.classList.toggle("hidden", state.items.length === 0);

  filtered.forEach((item, idx) => {
    const isChecked = selectedIds.includes(item.id);
    const row = document.createElement("div");
    row.className =
      "row" +
      (idx === selected ? " selected" : "") +
      (isChecked ? " checked" : "");
    row.setAttribute("role", "option");

    let leading;
    if (item.kind === "image" && item.thumb) {
      leading = `<img class="thumb" src="${item.thumb}" alt="" />`;
    } else if (item.isUrl) {
      leading = `<div class="chip url">${LINK_SVG}</div>`;
    } else {
      leading = '<div class="chip text">Aa</div>';
    }

    const key = idx < 9 ? `<span class="row-key">⌘${idx + 1}</span>` : "";
    row.innerHTML = `
      <input type="checkbox" class="row-check" ${isChecked ? "checked" : ""} tabindex="-1" />
      ${leading}
      <div class="row-body">
        <div class="row-text"></div>
        <div class="row-meta">${item.ago}</div>
      </div>
      ${key}`;
    row.querySelector(".row-text").textContent = item.preview;

    const checkbox = row.querySelector(".row-check");

    checkbox.addEventListener("click", (e) => {
      e.stopPropagation();
      toggleIdInSelection(item.id);
      row.classList.toggle("checked", selectedIds.includes(item.id));
    });

    row.addEventListener("click", (e) => {
      if (e.target === checkbox) return;
      if (selectedIds.length > 0) {
        toggleIdInSelection(item.id);
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
  searchEl.focus();
  refresh();
});

listen("state-changed", refresh);
refresh();
searchEl.focus();
