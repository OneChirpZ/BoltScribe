import type { AppConfig, AsrConnectionRetryConfig } from "../types";

export const defaultAsrConnectionRetry: AsrConnectionRetryConfig = {
  live_post_stop_max_attempts: 3,
  file_max_attempts: 3,
  connect_timeout_ms: 10_000,
  initial_delay_ms: 250,
  max_delay_ms: 5_000,
};

export const asrConnectionRetryLimits = {
  live_post_stop_max_attempts: { min: 1, max: 20 },
  file_max_attempts: { min: 1, max: 10 },
  connect_timeout_ms: { min: 1_000, max: 60_000 },
  initial_delay_ms: { min: 100, max: 10_000 },
  max_delay_ms: { min: 100, max: 30_000 },
} as const satisfies Record<keyof AsrConnectionRetryConfig, { min: number; max: number }>;

export type AsrConnectionRetryKey = keyof AsrConnectionRetryConfig;

export function normalizeAsrConnectionRetry(
  value: AsrConnectionRetryConfig | null | undefined,
): AsrConnectionRetryConfig {
  const source = { ...defaultAsrConnectionRetry, ...value };
  const normalized = {
    live_post_stop_max_attempts: clampInt(
      source.live_post_stop_max_attempts,
      asrConnectionRetryLimits.live_post_stop_max_attempts,
    ),
    file_max_attempts: clampInt(source.file_max_attempts, asrConnectionRetryLimits.file_max_attempts),
    connect_timeout_ms: clampInt(source.connect_timeout_ms, asrConnectionRetryLimits.connect_timeout_ms),
    initial_delay_ms: clampInt(source.initial_delay_ms, asrConnectionRetryLimits.initial_delay_ms),
    max_delay_ms: clampInt(source.max_delay_ms, asrConnectionRetryLimits.max_delay_ms),
  };
  normalized.max_delay_ms = Math.max(normalized.initial_delay_ms, normalized.max_delay_ms);
  return normalized;
}

export function updateAsrConnectionRetry(
  config: AppConfig,
  key: AsrConnectionRetryKey,
  rawValue: string,
): AppConfig {
  const value = Number(rawValue);
  if (!Number.isFinite(value)) {
    return config;
  }
  const connectionRetry = normalizeAsrConnectionRetry({
    ...(config.asr.connection_retry ?? defaultAsrConnectionRetry),
    [key]: value,
  });
  return {
    ...config,
    asr: {
      ...config.asr,
      connection_retry: connectionRetry,
    },
  };
}

function clampInt(value: number, limits: { min: number; max: number }): number {
  if (!Number.isFinite(value)) {
    return limits.min;
  }
  return Math.min(limits.max, Math.max(limits.min, Math.round(value)));
}
