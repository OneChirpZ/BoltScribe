import Field from "../../components/Field";
import type { TextBundle } from "../../domain/i18n";
import {
  defaultRecordingOverlayScale,
  defaultRecordingOverlaySnapRangePercent,
  maxRecordingOverlayScale,
  maxRecordingOverlaySnapRangePercent,
  minRecordingOverlayScale,
  minRecordingOverlaySnapRangePercent,
} from "../../domain/overlay";
import type { AppConfig } from "../../types";

const maxOverlayOffset = 4000;

export default function OverlaySection({
  config,
  onChange,
  text,
}: {
  config: AppConfig;
  onChange: (config: AppConfig) => void;
  text: TextBundle;
}) {
  function updateOffset(axis: "x" | "y", value: string) {
    const nextValue = clampInt(Number(value), -maxOverlayOffset, maxOverlayOffset);
    onChange({
      ...config,
      ui: {
        ...config.ui,
        [axis === "x" ? "recording_overlay_offset_x" : "recording_overlay_offset_y"]: nextValue,
      },
    });
  }

  function resetAppearance() {
    onChange({
      ...config,
      ui: {
        ...config.ui,
        recording_overlay_scale: defaultRecordingOverlayScale,
        recording_overlay_snap_range_percent: defaultRecordingOverlaySnapRangePercent,
        recording_overlay_offset_x: 0,
        recording_overlay_offset_y: 0,
      },
    });
  }

  return (
    <div className="settings-section">
      <div className="section-title">
        <h2>{text.settings.overlayAppearance}</h2>
        <button className="secondary small" type="button" onClick={resetAppearance}>{text.settings.resetOverlay}</button>
      </div>
      <div className="overlay-control-row">
        <div className="overlay-control-pair">
          <Field
            label={text.settings.overlayScale(
              Math.round((config.ui.recording_overlay_scale ?? defaultRecordingOverlayScale) * 200),
            )}
            className="compact-range-field overlay-scale-control"
          >
            <div className="range-compact">
              <input
                className="range-input"
                type="range"
                min={String(minRecordingOverlayScale)}
                max={String(maxRecordingOverlayScale)}
                step="0.05"
                value={config.ui.recording_overlay_scale ?? defaultRecordingOverlayScale}
                onChange={(event) => onChange({
                  ...config,
                  ui: {
                    ...config.ui,
                    recording_overlay_scale: Number(event.target.value),
                  },
                })}
              />
            </div>
          </Field>
          <Field
            label={text.settings.overlaySnapRange(
              config.ui.recording_overlay_snap_range_percent
                ?? defaultRecordingOverlaySnapRangePercent,
            )}
            className="compact-range-field overlay-snap-range-control"
          >
            <div className="range-compact">
              <input
                className="range-input"
                type="range"
                min={String(minRecordingOverlaySnapRangePercent)}
                max={String(maxRecordingOverlaySnapRangePercent)}
                step="10"
                value={config.ui.recording_overlay_snap_range_percent
                  ?? defaultRecordingOverlaySnapRangePercent}
                onChange={(event) => onChange({
                  ...config,
                  ui: {
                    ...config.ui,
                    recording_overlay_snap_range_percent: Number(event.target.value),
                  },
                })}
              />
            </div>
          </Field>
        </div>
        <div className="overlay-control-pair">
          <Field label={text.settings.offsetX} className="field-compact">
            <input
              type="number"
              min={-maxOverlayOffset}
              max={maxOverlayOffset}
              step="1"
              value={config.ui.recording_overlay_offset_x ?? 0}
              onChange={(event) => updateOffset("x", event.target.value)}
            />
          </Field>
          <Field label={text.settings.offsetY} className="field-compact">
            <input
              type="number"
              min={-maxOverlayOffset}
              max={maxOverlayOffset}
              step="1"
              value={config.ui.recording_overlay_offset_y ?? 0}
              onChange={(event) => updateOffset("y", event.target.value)}
            />
          </Field>
        </div>
      </div>
    </div>
  );
}

function clampInt(value: number, min: number, max: number): number {
  if (!Number.isFinite(value)) {
    return min;
  }
  return Math.min(max, Math.max(min, Math.round(value)));
}
