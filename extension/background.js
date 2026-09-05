const BrowserActivity = Object.freeze({
  YouTube: "YouTube",
  YouTubeShorts: "YouTubeShorts",
  Instagram: "Instagram",
  InstagramReels: "InstagramReels",
  Other: "Other",
});

const NATIVE_HOST_NAME = "com.milo.desktop";
const CLOSE_ACTIVE_DISTRACTION_TAB = "close_active_distraction_tab";
const NativeActivity = Object.freeze({
  [BrowserActivity.YouTube]: "youtube",
  [BrowserActivity.YouTubeShorts]: "youtube_shorts",
  [BrowserActivity.Instagram]: "instagram",
  [BrowserActivity.InstagramReels]: "instagram_reels",
  [BrowserActivity.Other]: "other",
});

let activeWindowId = null;
let activeTabId = null;
let selectionGeneration = 0;
let previousActivity = null;
let nativePort = null;

function connectNativeHost() {
  if (nativePort !== null) {
    return nativePort;
  }

  try {
    const port = browser.runtime.connectNative(NATIVE_HOST_NAME);
    nativePort = port;
    console.log("[milo-extension] native host connected");
    port.onMessage.addListener((message) => {
      void handleBrowserCommand(port, message);
    });
    port.onDisconnect.addListener(() => {
      if (nativePort === port) {
        nativePort = null;
        console.warn("[milo-extension] native host disconnected");
      }
    });
    return port;
  } catch (error) {
    console.warn("[milo-extension] could not connect to native host", error);
    return null;
  }
}

function isCloseActiveDistractionTabCommand(message) {
  return (
    message !== null &&
    typeof message === "object" &&
    !Array.isArray(message) &&
    Object.keys(message).length === 2 &&
    message.type === "browser_command" &&
    message.command === CLOSE_ACTIVE_DISTRACTION_TAB
  );
}

function sendCommandResult(port, result) {
  console.log(`[milo-extension] close command result: ${result}`);
  try {
    port.postMessage({
      type: "browser_command_result",
      command: CLOSE_ACTIVE_DISTRACTION_TAB,
      result,
    });
  } catch (error) {
    console.warn("[milo-extension] could not send browser command result", error);
  }
}

async function handleBrowserCommand(port, message) {
  if (!isCloseActiveDistractionTabCommand(message)) {
    console.warn("[milo-extension] invalid native message");
    return;
  }

  console.log(
    `[milo-extension] browser command received: ${CLOSE_ACTIVE_DISTRACTION_TAB}`,
  );
  console.log("[milo-extension] close command: querying active tab");

  let result = "no_active_tab";
  try {
    const focusedWindow = await browser.windows.getLastFocused();
    if (!focusedWindow || typeof focusedWindow.id !== "number") {
      sendCommandResult(port, result);
      return;
    }

    const [tab] = await browser.tabs.query({
      active: true,
      lastFocusedWindow: true,
    });
    if (
      !tab ||
      typeof tab.id !== "number" ||
      tab.windowId !== focusedWindow.id
    ) {
      sendCommandResult(port, result);
      return;
    }

    const currentActivity = classifyUrl(tab.url);
    console.log(
      `[milo-extension] close command classification: ${currentActivity}`,
    );
    if (
      currentActivity !== BrowserActivity.YouTubeShorts &&
      currentActivity !== BrowserActivity.InstagramReels
    ) {
      sendCommandResult(port, "ignored_not_distracting");
      return;
    }

    console.log("[milo-extension] closing active distraction tab");
    try {
      await browser.tabs.remove(tab.id);
    } catch (error) {
      const errorName =
        error !== null && typeof error === "object" && "name" in error
          ? error.name
          : "Firefox API error";
      console.warn(`[milo-extension] tab close failed: ${errorName}`);
      sendCommandResult(port, "error");
      return;
    }
    console.log("[milo-extension] tab closed");
    result = "closed";
  } catch (error) {
    const errorName =
      error !== null && typeof error === "object" && "name" in error
        ? error.name
        : "Firefox API error";
    console.warn(`[milo-extension] close command query failed: ${errorName}`);
    result = "error";
  }

  sendCommandResult(port, result);
}

function sendActivity(activity) {
  const port = connectNativeHost();
  if (port === null) {
    return;
  }

  try {
    port.postMessage({
      type: "browser_activity",
      activity: NativeActivity[activity],
    });
  } catch (error) {
    console.warn("[milo-extension] could not send browser activity", error);
  }
}

function classifyUrl(rawUrl) {
  if (typeof rawUrl !== "string" || rawUrl.length === 0) {
    return BrowserActivity.Other;
  }

  let url;
  try {
    url = new URL(rawUrl);
  } catch {
    return BrowserActivity.Other;
  }

  if (url.protocol !== "http:" && url.protocol !== "https:") {
    return BrowserActivity.Other;
  }

  const hostname = url.hostname.toLowerCase();
  if (hostname === "youtube.com" || hostname === "www.youtube.com") {
    return url.pathname.startsWith("/shorts/")
      ? BrowserActivity.YouTubeShorts
      : BrowserActivity.YouTube;
  }

  if (hostname === "instagram.com" || hostname === "www.instagram.com") {
    return url.pathname.startsWith("/reel/") ||
      url.pathname.startsWith("/reels/")
      ? BrowserActivity.InstagramReels
      : BrowserActivity.Instagram;
  }

  return BrowserActivity.Other;
}

function observeUrl(rawUrl) {
  const activity = classifyUrl(rawUrl);
  if (activity === previousActivity) {
    return;
  }

  previousActivity = activity;
  const displayUrl =
    typeof rawUrl === "string" && rawUrl.length > 0 ? rawUrl : "(no URL)";
  console.log(`[milo-extension] ${activity}\n${displayUrl}`);
  sendActivity(activity);
}

async function selectTab(tabId, windowId) {
  activeWindowId = windowId;
  activeTabId = tabId;
  const generation = ++selectionGeneration;

  try {
    const tab = await browser.tabs.get(tabId);
    if (
      generation !== selectionGeneration ||
      tab.id !== activeTabId ||
      tab.windowId !== activeWindowId
    ) {
      return;
    }

    observeUrl(tab.url);
  } catch {
    // The tab may have closed before tabs.get() completed.
  }
}

async function selectActiveTabInWindow(windowId) {
  activeWindowId = windowId;
  activeTabId = null;
  const generation = ++selectionGeneration;

  try {
    const [tab] = await browser.tabs.query({ active: true, windowId });
    if (
      !tab ||
      generation !== selectionGeneration ||
      windowId !== activeWindowId
    ) {
      return;
    }

    activeTabId = tab.id;
    observeUrl(tab.url);
  } catch {
    // The window may have closed before tabs.query() completed.
  }
}

browser.tabs.onActivated.addListener(({ tabId, windowId }) => {
  if (windowId === activeWindowId) {
    void selectTab(tabId, windowId);
  }
});

browser.tabs.onUpdated.addListener(
  (tabId, changeInfo) => {
    if (tabId === activeTabId && typeof changeInfo.url === "string") {
      observeUrl(changeInfo.url);
    }
  },
  { properties: ["url"] },
);

browser.windows.onFocusChanged.addListener((windowId) => {
  if (windowId === browser.windows.WINDOW_ID_NONE) {
    activeWindowId = null;
    activeTabId = null;
    selectionGeneration += 1;
    return;
  }

  void selectActiveTabInWindow(windowId);
});

async function initializeActiveTab() {
  const generation = ++selectionGeneration;

  try {
    const [tab] = await browser.tabs.query({ active: true, currentWindow: true });
    if (!tab || generation !== selectionGeneration) {
      return;
    }

    activeWindowId = tab.windowId;
    activeTabId = tab.id;
    observeUrl(tab.url);
  } catch {
    // Firefox may not have a selectable browser tab during startup.
  }
}

void initializeActiveTab();
