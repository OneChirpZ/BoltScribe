import type { AppApi } from "./contracts";
import { createBrowserPreviewAppApi } from "./browserPreviewAppApi";
import { nativeAppApi } from "./nativeAppApi";

function browserPreviewEnabled() {
  return import.meta.env.DEV && typeof window !== "undefined" && !("__TAURI_INTERNALS__" in window);
}

export const appApi: AppApi = browserPreviewEnabled()
  ? createBrowserPreviewAppApi({ appVersion: __APP_VERSION__ })
  : nativeAppApi;

export type {
  AppApi,
  AudioApi,
  ConfigApi,
  HistoryApi,
  LearningApi,
  OverlayApi,
  PermissionsApi,
  SystemApi,
  Unlisten,
  WorkflowApi,
} from "./contracts";
export { createBrowserPreviewAppApi } from "./browserPreviewAppApi";
