import { useEffect, useRef, useState, type CSSProperties } from "react";
import type { DailyInputStats, InputStats } from "../types";
import type { TextBundle } from "../domain/i18n";

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
  const cells = heatmapCells(stats?.daily ?? [], heatmapWeeks);
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

function heatmapCells(daily: DailyInputStats[], weekCount: number) {
  const byDate = new Map(daily.map((day) => [day.date, day]));
  const dates = daily.map((day) => parseDateKey(day.date)).filter((date): date is Date => Boolean(date));
  const today = startOfLocalDay(new Date());
  const lastStatsDay = dates.length > 0 ? new Date(Math.max(...dates.map((date) => date.getTime()))) : today;
  const end = new Date(Math.max(today.getTime(), lastStatsDay.getTime()));
  const trailingEmptyDays = 6 - end.getDay();
  end.setDate(end.getDate() + trailingEmptyDays);
  const totalDays = Math.max(minHeatmapWeeks, weekCount) * 7;
  const start = new Date(end);
  start.setDate(end.getDate() - totalDays + 1);

  return Array.from({ length: totalDays }, (_, index) => {
    const date = new Date(start);
    date.setDate(start.getDate() + index);
    const key = formatDateKey(date);
    return byDate.get(key) ?? {
      date: key,
      record_count: 0,
      character_count: 0,
      audio_duration_ms: 0,
    };
  });
}

function heatmapWeekCount(width: number) {
  const columns = Math.floor((width + heatmapGap) / (heatmapCellSize + heatmapGap));
  return Math.max(minHeatmapWeeks, columns);
}

function parseDateKey(value: string) {
  const match = /^(\d{4})-(\d{2})-(\d{2})$/.exec(value);
  if (!match) {
    return null;
  }

  const year = Number(match[1]);
  const month = Number(match[2]);
  const day = Number(match[3]);
  const date = new Date(year, month - 1, day);
  if (date.getFullYear() !== year || date.getMonth() !== month - 1 || date.getDate() !== day) {
    return null;
  }
  return date;
}

function heatmapLevel(count: number, maxCount: number) {
  if (count <= 0 || maxCount <= 0) {
    return 0;
  }
  return Math.max(1, Math.min(4, Math.ceil((count / maxCount) * 4)));
}

function startOfLocalDay(date: Date) {
  return new Date(date.getFullYear(), date.getMonth(), date.getDate());
}

function formatDateKey(date: Date) {
  const year = date.getFullYear();
  const month = String(date.getMonth() + 1).padStart(2, "0");
  const day = String(date.getDate()).padStart(2, "0");
  return `${year}-${month}-${day}`;
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
