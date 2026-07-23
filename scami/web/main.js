const romInput = document.getElementById("rom-input");
const romName = document.getElementById("rom-name");
const status = document.getElementById("status");
const launcher = document.getElementById("launcher");
const launcherCard = document.getElementById("launcher-card");
const launcherClose = document.getElementById("launcher-close");
const loadBtn = document.getElementById("load-btn");
const resetBtn = document.getElementById("reset-btn");
const launcherBtns = document.querySelectorAll(".launcher-btn");
const fetchBar = document.getElementById("fetch-bar-fill");
const toast = document.getElementById("toast");

let wasmReady = false;
let emulationStarted = false;
let hasLoadedRom = false;
let fileDragDepth = 0;

let toastTimer = null;

const emulatorKeyCodes = new Set([
  "KeyW",
  "KeyA",
  "KeyS",
  "KeyD",
  "ArrowUp",
  "ArrowLeft",
  "ArrowDown",
  "ArrowRight",
  "KeyZ",
  "KeyJ",
  "KeyX",
  "KeyK",
  "KeyC",
  "Enter",
  "KeyV",
  "ShiftRight",
]);

// --- Tab-visibility pause state ---
// True when the emulator was running but we auto-paused due to tab hide.
let autoSuspended = false;

function setPaused(paused) {
  const b = window.wasmBindings;
  if (b && typeof b.set_paused === "function") b.set_paused(paused);
}

function setFetchBarActive(active) {
  if (!fetchBar) return;
  fetchBar.classList.toggle("hidden", !active);
}

function showToast(message) {
  if (!toast) return;
  toast.textContent = message;
  toast.classList.add("visible");
  if (toastTimer) clearTimeout(toastTimer);
  toastTimer = setTimeout(() => {
    toast.classList.remove("visible");
  }, 3200);
}

function isFileDrag(event) {
  return Array.from(event.dataTransfer?.types || []).includes("Files");
}

function setDropOverlayVisible(visible) {
  document.body.classList.toggle("rom-drag-active", visible);
}

function resetFileDrag() {
  fileDragDepth = 0;
  setDropOverlayVisible(false);
}

function isNesFile(file) {
  return file.name.toLowerCase().endsWith(".nes");
}

function shouldCaptureEmulatorKey(event) {
  const canvas = document.getElementById("nes-canvas");
  if (!canvas) return false;

  const canvasHasEvent =
    event.target === canvas || document.activeElement === canvas;
  const hasBrowserShortcutModifier =
    event.ctrlKey || event.metaKey || event.altKey;

  return (
    canvasHasEvent &&
    !hasBrowserShortcutModifier &&
    emulatorKeyCodes.has(event.code)
  );
}

// ---------- tab-visibility guard ----------
// When the page becomes hidden we pause the emulator so it doesn't spin
// in the background (browsers throttle rAF anyway, which causes the
// giant time-debt that makes it skip when you return).
// We resume automatically when the page becomes visible again, but only
// if *we* were the ones who paused it (don't cancel a user-initiated
// pause or launcher-pause).
document.addEventListener("visibilitychange", () => {
  if (!hasLoadedRom || !emulationStarted) return;

  if (document.hidden) {
    // Only auto-suspend if the emulator is currently running (launcher gone,
    // status "running") so we don't clobber a deliberate user pause.
    const isRunning = launcher.classList.contains("gone");
    if (isRunning) {
      autoSuspended = true;
      setPaused(true);
      // No need to update the status text — the user can't see it anyway.
    }
  } else {
    if (autoSuspended) {
      autoSuspended = false;
      // Reset the WASM-side timestamp so it doesn't accumulate a huge
      // delta covering the whole background period.
      const b = window.wasmBindings;
      if (b && typeof b.reset_timestamp === "function") {
        b.reset_timestamp();
      }
      setPaused(false);
      status.textContent = "running";
    }
  }
});

// ---------- window-focus guard ----------
// Some browsers keep rAF alive even when another window has focus but
// the tab is still visible (e.g. alt-tab to a different app on macOS).
// We pause on blur and resume on focus for full coverage.
window.addEventListener("blur", () => {
  if (!hasLoadedRom || !emulationStarted) return;
  const isRunning = launcher.classList.contains("gone");
  if (isRunning && !autoSuspended) {
    autoSuspended = true;
    setPaused(true);
  }
});

window.addEventListener("focus", () => {
  if (!hasLoadedRom || !emulationStarted) return;
  if (autoSuspended) {
    autoSuspended = false;
    const b = window.wasmBindings;
    if (b && typeof b.reset_timestamp === "function") {
      b.reset_timestamp();
    }
    setPaused(false);
    status.textContent = "running";
  }
});
// ------------------------------------------

function onWasmReady() {
  if (wasmReady) return;
  wasmReady = true;
  status.textContent = "ready";
  launcherBtns.forEach((b) => (b.disabled = false));
}
window.addEventListener("TrunkApplicationStarted", onWasmReady, {
  once: true,
});
if (window.wasmBindings) onWasmReady();

function ensureStarted() {
  if (emulationStarted) return;
  emulationStarted = true;
  const b = window.wasmBindings;
  if (b && typeof b.start_emulation === "function") b.start_emulation();
}

let currentRomBytes = null;
let currentRomName = null;

async function loadRomBytes(bytes, name) {
  const b = window.wasmBindings;
  if (!b || typeof b.load_rom !== "function") return false;
  status.textContent = "loading...";
  try {
    b.load_rom(new Uint8Array(bytes));
    currentRomBytes = bytes;
    currentRomName = name;
    romName.textContent = name;
    status.textContent = "running";
    if (!hasLoadedRom) {
      hasLoadedRom = true;
      launcherClose.classList.add("visible");
    }
    resetBtn.disabled = false;
    hideLauncher();
    return true;
  } catch (err) {
    console.error(err);
    status.textContent = "error";
    let errStr = err.toString();
    if (errStr.includes("UnknownMapperIdError")) {
      let mapper_id = errStr.split("(")[1].split(")")[0];
      showToast(`Mapper: ${mapper_id} not yet implemented in S.C.A.M.`);
    } else {
      showToast(`Could not load that file. Got unknown error: ${errStr}`);
    }
    return false;
  }
}

async function loadLocalFile(file) {
  if (!isNesFile(file)) {
    showToast("Only .nes ROM files can be loaded.");
    return;
  }

  if (!wasmReady) {
    showToast("The emulator is still starting. Try again in a moment.");
    return;
  }

  ensureStarted();
  status.textContent = "reading...";
  try {
    const buf = await file.arrayBuffer();
    await loadRomBytes(buf, file.name);
  } catch (err) {
    console.error(err);
    status.textContent = "read error";
    showToast("The ROM file could not be read.");
  }
}

window.showLauncher = function () {
  launcher.classList.remove("gone");
  setPaused(true);
  status.textContent = hasLoadedRom ? "paused" : "ready";
};

window.hideLauncher = function () {
  if (hasLoadedRom) {
    launcher.classList.add("gone");
    // Reset timestamp so whatever time was spent in the launcher
    // doesn't produce a catch-up burst.
    const b = window.wasmBindings;
    if (b && typeof b.reset_timestamp === "function") {
      b.reset_timestamp();
    }
    setPaused(false);
    status.textContent = "running";
    // Move focus away from any launcher button so that Enter/Space
    // keypresses go to the emulator canvas instead of re-triggering
    // the last-focused button.
    if (document.activeElement && document.activeElement !== document.body) {
      document.activeElement.blur();
    }
    const canvas =
      document.getElementById("nes-canvas") || document.querySelector("canvas");
    if (canvas) canvas.focus();
  }
};

launcher.addEventListener("click", (e) => {
  if (!hasLoadedRom) return;
  if (e.target === launcher) {
    hideLauncher();
  }
});

// Prevent Enter/Space from re-firing the last focused launcher button
// after the launcher has already handled the action and closed.
// Also allow Escape to dismiss the launcher.
document.addEventListener("keydown", (e) => {
  if (shouldCaptureEmulatorKey(e)) {
    e.preventDefault();
  }

  if (!launcher.classList.contains("gone")) {
    // Launcher is open — let buttons handle Enter/Space normally,
    // but swallow any stray Enter that has no focused target inside
    // the launcher (e.g. focus already blurred after a click).
    if (
      (e.key === "Enter" || e.key === " ") &&
      (!document.activeElement || !launcher.contains(document.activeElement))
    ) {
      e.preventDefault();
    }
    if (e.key === "Escape" && hasLoadedRom) {
      hideLauncher();
    }
  }
});

document.addEventListener("keyup", (e) => {
  if (shouldCaptureEmulatorKey(e)) {
    e.preventDefault();
  }
});

launcherCard.addEventListener("click", (e) => {
  e.stopPropagation();
});

window.loadPredefined = async function (path, name) {
  if (!wasmReady) return;
  ensureStarted();
  status.textContent = "fetching...";
  setFetchBarActive(true);
  try {
    const resp = await fetch(path);
    if (!resp.ok) throw new Error(`HTTP ${resp.status}`);
    const buf = await resp.arrayBuffer();
    await loadRomBytes(buf, name);
  } catch (err) {
    console.error(err);
    status.textContent = "fetch error";
    showToast("Fetch failed. Check your connection or try another ROM.");
  } finally {
    setFetchBarActive(false);
  }
};

window.openFilePicker = function () {
  if (!wasmReady) return;
  romInput.click();
};

romInput.addEventListener("change", async (e) => {
  const file = e.target.files && e.target.files[0];
  if (!file) return;
  await loadLocalFile(file);
  e.target.value = "";
});

document.addEventListener("dragenter", (event) => {
  if (!isFileDrag(event)) return;
  event.preventDefault();
  fileDragDepth += 1;
  setDropOverlayVisible(true);
});

document.addEventListener("dragover", (event) => {
  if (!isFileDrag(event)) return;
  event.preventDefault();
  event.dataTransfer.dropEffect = "copy";
});

document.addEventListener("dragleave", (event) => {
  if (!isFileDrag(event)) return;
  fileDragDepth = Math.max(0, fileDragDepth - 1);
  if (fileDragDepth === 0) setDropOverlayVisible(false);
});

document.addEventListener("drop", async (event) => {
  const files = Array.from(event.dataTransfer?.files || []);
  if (!isFileDrag(event) && files.length === 0) return;

  event.preventDefault();
  resetFileDrag();

  if (files.length !== 1) {
    showToast("Drop one .nes file at a time.");
    return;
  }

  await loadLocalFile(files[0]);
});

document.addEventListener("dragend", resetFileDrag);

const WASM_PANIC_MESSAGE = "scam-wasm-message";
const WASM_CRASH_COUNT = "scam-crash-count";
const WASM_CRASH_TIME = "scam-crash-time";

function formatPanicMessage(info) {
  if (!info) return "Unknown crash";
  const lines = info
    .split("\n")
    .map((l) => l.trim())
    .filter((l) => l.length > 0);

  if (lines.length > 1 && lines[0].startsWith("panicked at ")) {
    const message = lines.slice(1).join(" ");
    return message.substring(0, 150) + (message.length > 150 ? "..." : "");
  }

  const joined = lines.join(" ");
  return joined.substring(0, 150) + (joined.length > 150 ? "..." : "");
}

window.recordRustPanic = (info) => {
  const now = Date.now();
  const lastCrash = parseInt(
    sessionStorage.getItem(WASM_CRASH_TIME) || "0",
    10,
  );
  let crashCount = parseInt(
    sessionStorage.getItem(WASM_CRASH_COUNT) || "0",
    10,
  );

  if (now - lastCrash < 10000) {
    crashCount++;
  } else {
    crashCount = 1;
  }

  const shortInfo = formatPanicMessage(info);

  sessionStorage.setItem(WASM_CRASH_TIME, now.toString());
  sessionStorage.setItem(WASM_CRASH_COUNT, crashCount.toString());
  sessionStorage.setItem(WASM_PANIC_MESSAGE, shortInfo);

  if (crashCount >= 3) {
    showToast(`Repeated crashes detected. Halting emulator.\n${shortInfo}`);
    return;
  }

  location.reload();
};

const previousPanic = sessionStorage.getItem(WASM_PANIC_MESSAGE);

if (previousPanic) {
  showToast(`Emulator restarted after a panic:\n${previousPanic}`);
  sessionStorage.removeItem(WASM_PANIC_MESSAGE);
}

window.resetGame = async function () {
  if (!currentRomBytes || !currentRomName) return;
  await loadRomBytes(currentRomBytes, currentRomName);
};

// ---------- Touch Controls ----------
const touchControls = document.getElementById("touch-controls");
const touchToggle = document.getElementById("touch-toggle");
const keybinds = document.getElementById("keybinds");

const buttonMap = {
  up: ["KeyW", "ArrowUp"],
  down: ["KeyS", "ArrowDown"],
  left: ["KeyA", "ArrowLeft"],
  right: ["KeyD", "ArrowRight"],
  a: ["KeyJ", "KeyZ"],
  b: ["KeyK", "KeyX"],
  start: ["Enter", "KeyC"],
  select: ["ShiftRight", "KeyV"],
};

let touchEnabled = localStorage.getItem("touchControls") !== "false";

function isTouchDevice() {
  return window.matchMedia("(hover: none) and (pointer: coarse)").matches;
}

function updateTouchVisibility() {
  const isTouch = isTouchDevice();
  touchToggle.classList.toggle("hidden", !isTouch);

  if (isTouch && touchEnabled) {
    touchControls.classList.add("visible");
    keybinds.style.display = "none";
  } else {
    touchControls.classList.remove("visible");
    keybinds.style.display = "";
  }
  touchToggle.textContent = touchEnabled ? "hide controls" : "show controls";
}

function dispatchKeyEvent(code, type) {
  const canvas = document.getElementById("nes-canvas");
  if (!canvas) return;

  const event = new KeyboardEvent(type, {
    code: code,
    key: code,
    bubbles: true,
  });
  canvas.dispatchEvent(event);
}

Object.keys(buttonMap).forEach((btn) => {
  const el = document.querySelector(`[data-btn="${btn}"]`);
  if (!el) return;

  const codes = buttonMap[btn];

  el.addEventListener("touchstart", (e) => {
    e.preventDefault();
    el.classList.add("pressed");
    codes.forEach((code) => dispatchKeyEvent(code, "keydown"));
  });

  el.addEventListener("touchend", (e) => {
    e.preventDefault();
    el.classList.remove("pressed");
    codes.forEach((code) => dispatchKeyEvent(code, "keyup"));
  });

  el.addEventListener("touchcancel", (e) => {
    el.classList.remove("pressed");
    codes.forEach((code) => dispatchKeyEvent(code, "keyup"));
  });
});

touchToggle.addEventListener("click", () => {
  touchEnabled = !touchEnabled;
  localStorage.setItem("touchControls", touchEnabled);
  updateTouchVisibility();
});

updateTouchVisibility();
window.addEventListener("resize", updateTouchVisibility);
// ------------------------------------
