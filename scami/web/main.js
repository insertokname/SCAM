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

if (toast) {
  toast.addEventListener("click", () => {
    toast.classList.remove("visible");
    if (toastTimer) clearTimeout(toastTimer);
  });
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

let cachedNesCanvas = null;
function getNesCanvas() {
  if (!cachedNesCanvas) cachedNesCanvas = document.getElementById("nes-canvas");
  return cachedNesCanvas;
}

function shouldCaptureEmulatorKey(event) {
  const canvas = getNesCanvas();
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
const fullscreenBtn = document.getElementById("fullscreen-btn");
const fullscreenExitBtn = document.getElementById("fullscreen-exit-btn");
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

  const mainElement = document.querySelector("main");
  const launcher = document.getElementById("launcher");

  if (isTouch && touchEnabled) {
    touchControls.classList.add("visible");
    keybinds.style.display = "none";
    
    const touchHeight = touchControls.getBoundingClientRect().height;
    
    // In portrait mode, we can pad the entire main element because width is the limiting 
    // factor for the game canvas, so padding simply pushes the game and menus up.
    // In landscape mode, height is the limiting factor. Shrinking main makes the game 
    // tiny! Instead, we only pad the launcher menu so it remains scrollable, allowing
    // the game canvas to use the full screen height (controls naturally sit on the sides).
    if (window.innerHeight > window.innerWidth) {
      if (mainElement) mainElement.style.paddingBottom = `${touchHeight + 12}px`;
      if (launcher) launcher.style.paddingBottom = "";
    } else {
      if (mainElement) mainElement.style.paddingBottom = "";
      if (launcher) launcher.style.paddingBottom = `${touchHeight + 12}px`;
    }
    
    // Defer updating rects until after the layout changes have been fully applied
    requestAnimationFrame(updateButtonRects);
  } else {
    touchControls.classList.remove("visible");
    keybinds.style.display = "";
    if (mainElement) mainElement.style.paddingBottom = "";
    if (launcher) launcher.style.paddingBottom = "";
  }
  touchToggle.textContent = touchEnabled ? "hide controls" : "show controls";
}

function dispatchKeyEvent(code, type) {
  const canvas = getNesCanvas();
  if (!canvas) return;

  const event = new KeyboardEvent(type, {
    code: code,
    key: code,
    bubbles: true,
  });
  canvas.dispatchEvent(event);
}

let currentlyPressedBtns = new Set();
let newlyPressedBtns = new Set();
let buttonRects = new Map();
let buttonElements = new Map();

function updateButtonRects() {
  buttonRects.clear();
  buttonElements.clear();
  const buttons = document.querySelectorAll("[data-btn]");
  buttons.forEach(btn => {
    const rect = btn.getBoundingClientRect();
    if (rect.width > 0 && rect.height > 0) {
      buttonRects.set(btn.dataset.btn, rect);
      buttonElements.set(btn.dataset.btn, btn);
    }
  });
}

function handleTouches(e) {
  if (e.cancelable) {
    e.preventDefault();
  }
  
  newlyPressedBtns.clear();

  for (let i = 0; i < e.touches.length; i++) {
    const touch = e.touches[i];
    const x = touch.clientX;
    const y = touch.clientY;
    
    for (const [btnName, rect] of buttonRects.entries()) {
      if (x >= rect.left && x <= rect.right && y >= rect.top && y <= rect.bottom) {
        newlyPressedBtns.add(btnName);
      }
    }
  }

  // Release buttons that are no longer pressed
  for (const btn of currentlyPressedBtns) {
    if (!newlyPressedBtns.has(btn)) {
      const el = buttonElements.get(btn);
      if (el) el.classList.remove("pressed");
      const codes = buttonMap[btn];
      if (codes) {
        for (let i = 0; i < codes.length; i++) {
          dispatchKeyEvent(codes[i], "keyup");
        }
      }
    }
  }

  // Press buttons that are newly pressed
  for (const btn of newlyPressedBtns) {
    if (!currentlyPressedBtns.has(btn)) {
      const el = buttonElements.get(btn);
      if (el) el.classList.add("pressed");
      const codes = buttonMap[btn];
      if (codes) {
        for (let i = 0; i < codes.length; i++) {
          dispatchKeyEvent(codes[i], "keydown");
        }
      }
    }
  }

  const temp = currentlyPressedBtns;
  currentlyPressedBtns = newlyPressedBtns;
  newlyPressedBtns = temp;
}

if (touchControls) {
  touchControls.addEventListener("touchstart", handleTouches, { passive: false });
  touchControls.addEventListener("touchmove", handleTouches, { passive: false });
  touchControls.addEventListener("touchend", handleTouches, { passive: false });
  touchControls.addEventListener("touchcancel", handleTouches, { passive: false });
}

touchToggle.addEventListener("click", () => {
  touchEnabled = !touchEnabled;
  localStorage.setItem("touchControls", touchEnabled);
  updateTouchVisibility();
});

function toggleFullscreen() {
  const canvasHost = document.getElementById("canvas-host");
  if (!canvasHost) return;

  if (!document.fullscreenElement) {
    if (canvasHost.requestFullscreen) {
      canvasHost.requestFullscreen();
    } else if (canvasHost.webkitRequestFullscreen) { /* Safari */
      canvasHost.webkitRequestFullscreen();
    } else if (canvasHost.msRequestFullscreen) { /* IE11 */
      canvasHost.msRequestFullscreen();
    }
    fullscreenBtn.textContent = "exit fullscreen";
  } else {
    if (document.exitFullscreen) {
      document.exitFullscreen();
    } else if (document.webkitExitFullscreen) { /* Safari */
      document.webkitExitFullscreen();
    } else if (document.msExitFullscreen) { /* IE11 */
      document.msExitFullscreen();
    }
    fullscreenBtn.textContent = "fullscreen";
  }
}

if (fullscreenBtn) {
  fullscreenBtn.addEventListener("click", () => {
    toggleFullscreen();
    fullscreenBtn.blur();
  });
  
  if (fullscreenExitBtn) {
    fullscreenExitBtn.addEventListener("click", () => {
      if (document.fullscreenElement) {
        toggleFullscreen();
      }
    });
  }

  document.addEventListener("fullscreenchange", () => {
    if (!document.fullscreenElement) {
      fullscreenBtn.textContent = "fullscreen";
    } else {
      fullscreenBtn.textContent = "exit fullscreen";
    }
  });
  
  document.addEventListener("webkitfullscreenchange", () => {
    if (!document.webkitFullscreenElement) {
      fullscreenBtn.textContent = "fullscreen";
    } else {
      fullscreenBtn.textContent = "exit fullscreen";
    }
  });
}

updateTouchVisibility();
window.addEventListener("resize", () => {
  updateTouchVisibility();
  requestAnimationFrame(updateButtonRects);
});
// ------------------------------------
