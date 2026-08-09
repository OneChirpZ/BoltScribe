export const minVadInitialSilenceTimeoutSecs = 5;
export const maxVadInitialSilenceTimeoutSecs = 60;
export const vadInitialSilenceTimeoutStepSecs = 1;

export const minVadNoiseMarginDb = 1;
export const maxVadNoiseMarginDb = 40;
export const vadNoiseMarginStepDb = 1;

export const minVadConfirmationMs = 240;
export const maxVadConfirmationMs = 2000;
export const vadConfirmationStepMs = 20;

export const minVadNoiseWindowMs = 400;
export const maxVadNoiseWindowMs = 3000;
export const vadNoiseWindowStepMs = 100;

export const defaultVadNoiseMarginDb = 12;
export const defaultVadConfirmationMs = 480;
export const defaultVadNoiseWindowMs = 2000;
export const defaultVadInitialSilenceTimeoutSecs = 15;

export function normalizeSteppedInt(value: number, min: number, max: number, step: number) {
  const finiteValue = Number.isFinite(value) ? value : min;
  const clamped = Math.max(min, Math.min(max, Math.round(finiteValue)));
  return Math.max(min, Math.min(max, Math.round(clamped / step) * step));
}

export function normalizeVadInitialSilenceTimeoutSecs(value: number) {
  return normalizeSteppedInt(
    value,
    minVadInitialSilenceTimeoutSecs,
    maxVadInitialSilenceTimeoutSecs,
    vadInitialSilenceTimeoutStepSecs,
  );
}

export function normalizeVadNoiseMargin(value: number) {
  return normalizeSteppedInt(value, minVadNoiseMarginDb, maxVadNoiseMarginDb, vadNoiseMarginStepDb);
}

export function normalizeVadConfirmationMs(value: number) {
  return normalizeSteppedInt(value, minVadConfirmationMs, maxVadConfirmationMs, vadConfirmationStepMs);
}

export function normalizeVadNoiseWindowMs(value: number) {
  return normalizeSteppedInt(value, minVadNoiseWindowMs, maxVadNoiseWindowMs, vadNoiseWindowStepMs);
}
