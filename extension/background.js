const BrowserActivity = Object.freeze({
  YouTube: "YouTube",
  YouTubeShorts: "YouTubeShorts",
  Instagram: "Instagram",
  InstagramReels: "InstagramReels",
  Other: "Other",
});

const NATIVE_HOST_NAME = "com.milo.desktop";
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
