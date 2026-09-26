const { invoke } = window.__TAURI__.core;
const { open } = window.__TAURI__.dialog;

const WEEK_MS = 7 * 24 * 60 * 60 * 1000;
const POLL_MS = 5 * 60 * 1000; // usage API is rate limited; don't hammer it
const TICK_MS = 30 * 1000; // move the "now" line between polls
const STORAGE_KEY = "tokenPath";

const $ = (id) => document.getElementById(id);
const pathInput = $("path");
let lastUsage = null; // { utilization, resetsAt: Date }
let fetching = false;

function loadSavedPath() {
  try { return localStorage.getItem(STORAGE_KEY); } catch { return null; }
}
function savePath(value) {
  try {
    if (value) localStorage.setItem(STORAGE_KEY, value);
    else localStorage.removeItem(STORAGE_KEY);
  } catch {}
}

function setStatus(text, isError = false) {
  $("status").textContent = text;
  $("status").classList.toggle("error", isError);
}

function render() {
  if (!lastUsage) return;
  const { utilization, resetsAt } = lastUsage;
  const used = Math.max(0, Math.min(100, utilization));
  $("pct").textContent = `${Math.round(utilization)}%`;
  $("fill").style.width = `${used}%`;
  $("bar").setAttribute("aria-valuenow", String(used));

  if (!resetsAt) {
    $("now").hidden = true;
    $("elapsed").textContent = "No active weekly window";
    $("resets").textContent = "";
    $("fill").classList.remove("over");
    return;
  }

  const start = resetsAt.getTime() - WEEK_MS;
  const frac = Math.max(0, Math.min(1, (Date.now() - start) / WEEK_MS));
  $("now").hidden = false;
  $("now").style.left = `${frac * 100}%`;
  $("nowLabel").textContent = "now";
  $("fill").classList.toggle("over", used > frac * 100);

  $("elapsed").textContent = `Day ${(frac * 7).toFixed(1)} of 7 · ${Math.round(frac * 100)}% of week`;
  $("resets").textContent = "Resets " + resetsAt.toLocaleString(undefined, {
    weekday: "short", hour: "numeric", minute: "2-digit",
  });
}

async function refresh() {
  if (fetching) return;
  fetching = true;
  $("refresh").disabled = true;
  setStatus("Loading…");
  try {
    const path = pathInput.value.trim() || null;
    const { usage, source } = await invoke("get_usage", { path });
    const week = usage.seven_day;
    lastUsage = {
      utilization: week?.utilization ?? 0,
      resetsAt: week?.resets_at ? new Date(week.resets_at) : null,
    };
    render();
    const t = new Date().toLocaleTimeString(undefined, { hour: "numeric", minute: "2-digit" });
    setStatus(`Updated ${t} · ${source}`);
  } catch (err) {
    setStatus(String(err), true);
  } finally {
    fetching = false;
    $("refresh").disabled = false;
  }
}

async function validatePath() {
  const value = pathInput.value.trim();
  const ok = !value || (await invoke("path_exists", { path: value }));
  pathInput.classList.toggle("bad", !ok);
  return ok;
}

async function onPathChanged() {
  savePath(pathInput.value.trim());
  if (await validatePath()) refresh();
  else setStatus("That file doesn't exist.", true);
}

async function browse() {
  const defaultPath = await invoke("default_token_dir");
  const file = await open({
    multiple: false,
    directory: false,
    defaultPath: defaultPath ?? undefined,
    title: "Select Claude Code .credentials.json",
  });
  if (typeof file === "string" && file) {
    pathInput.value = file;
    onPathChanged();
  }
}

async function init() {
  const saved = loadSavedPath();
  if (saved && (await invoke("path_exists", { path: saved }))) {
    pathInput.value = saved;
  } else {
    pathInput.value = (await invoke("detect_token_path")) ?? "";
  }
  pathInput.addEventListener("change", onPathChanged);
  pathInput.addEventListener("keydown", (e) => { if (e.key === "Enter") pathInput.blur(); });
  $("browse").addEventListener("click", browse);
  $("refresh").addEventListener("click", refresh);

  refresh();
  setInterval(refresh, POLL_MS);
  setInterval(render, TICK_MS);
}

init();
