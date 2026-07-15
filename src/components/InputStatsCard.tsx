import { useEffect, useRef, useState, type CSSProperties } from "react";
import type { InputStats } from "../types";
import type { TextBundle } from "../domain/i18n";
import { buildInputHeatmapCells } from "../domain/inputStatsHeatmap";

const heatmapCellSize = 9;
const heatmapGap = 3;
const defaultHeatmapWeeks = 16;
const minHeatmapWeeks = 7;

export default function InputStatsCard({
  stats,
  text,
}: {
  stats: InputStats | null;
  text: TextBundle;
}) {
  const heatmapRef = useRef<HTMLDivElement>(null);
  const [heatmapWeeks, setHeatmapWeeks] = useState(defaultHeatmapWeeks);
  const cells = buildInputHeatmapCells(stats?.daily ?? [], heatmapWeeks);
  const maxCount = Math.max(0, ...cells.map((cell) => cell.record_count));
  const heatmapStyle = { "--input-heatmap-weeks": heatmapWeeks } as CSSProperties;

  useEffect(() => {
    const element = heatmapRef.current;
    if (!element) {
      return;
    }

    const updateWeeks = (width: number) => {
      setHeatmapWeeks(heatmapWeekCount(width));
    };

    updateWeeks(element.clientWidth);
    if (typeof ResizeObserver === "undefined") {
      const handleResize = () => updateWeeks(element.clientWidth);
      window.addEventListener("resize", handleResize);
      return () => window.removeEventListener("resize", handleResize);
    }

    const observer = new ResizeObserver((entries) => {
      updateWeeks(entries[0]?.contentRect.width ?? element.clientWidth);
    });
    observer.observe(element);
    return () => observer.disconnect();
  }, []);

  return (
    <section className="input-stats-card" aria-label={text.stats.title}>
      <div className="input-stats-header">
        <span>{text.stats.title}</span>
      </div>
      <div className="input-stats-metrics">
        <StatValue label={text.stats.totalChars} value={formatInteger(stats?.total_character_count ?? 0)} />
        <StatValue label={text.stats.totalTime} value={formatDuration(stats?.total_audio_duration_ms ?? 0)} />
        <StatValue label={text.stats.charsPerMinute} value={formatRate(stats?.average_chars_per_minute ?? 0)} />
      </div>
      <div className="input-heatmap-block">
        <div className="input-heatmap-title">{text.stats.heatmapLabel}</div>
        <div ref={heatmapRef} className="input-heatmap" style={heatmapStyle} aria-label={text.stats.heatmapLabel}>
          {cells.map((cell) => (
            <span
              key={cell.date}
              className="input-heatmap-cell"
              data-level={heatmapLevel(cell.record_count, maxCount)}
              title={text.stats.dayTitle(cell.date, cell.record_count, cell.character_count)}
            />
          ))}
        </div>
        {(stats?.daily.length ?? 0) === 0 ? <p>{text.stats.noData}</p> : null}
      </div>
    </section>
  );
}

function StatValue({ label, value }: { label: string; value: string }) {
  return (
    <div className="input-stat-value">
      <span>{label}</span>
      <strong>{value}</strong>
    </div>
  );
}

function heatmapWeekCount(width: number) {
  const columns = Math.floor((width + heatmapGap) / (heatmapCellSize + heatmapGap));
  return Math.max(minHeatmapWeeks, columns);
}

function heatmapLevel(count: number, maxCount: number) {
  if (count <= 0 || maxCount <= 0) {
    return 0;
  }
  return Math.max(1, Math.min(4, Math.ceil((count / maxCount) * 4)));
}

function formatInteger(value: number) {
  return Math.round(value).toLocaleString();
}

function formatRate(value: number) {
  if (!Number.isFinite(value)) {
    return "0";
  }
  return Math.round(value).toLocaleString();
}

function formatDuration(ms: number) {
  const totalSeconds = Math.round(ms / 1000);
  const hours = Math.floor(totalSeconds / 3600);
  const minutes = Math.floor((totalSeconds % 3600) / 60);
  if (hours > 0) {
    return `${hours}h ${minutes}m`;
  }
  if (minutes > 0) {
    return `${minutes}m`;
  }
  return `${totalSeconds}s`;
}
