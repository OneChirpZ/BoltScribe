import type { DailyInputStats, InputStats } from "../types";
import type { TextBundle } from "../domain/i18n";

const heatmapDays = 91;

export default function InputStatsCard({
  stats,
  text,
}: {
  stats: InputStats | null;
  text: TextBundle;
}) {
  const cells = heatmapCells(stats?.daily ?? []);
  const maxCount = Math.max(0, ...cells.map((cell) => cell.record_count));

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
        <div className="input-heatmap" aria-label={text.stats.heatmapLabel}>
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
      <strong>{value}</strong>
      <span>{label}</span>
    </div>
  );
}

function heatmapCells(daily: DailyInputStats[]) {
  const byDate = new Map(daily.map((day) => [day.date, day]));
  const today = startOfLocalDay(new Date());
  const start = new Date(today);
  start.setDate(start.getDate() - (heatmapDays - 1));
  const leadingEmptyDays = start.getDay();
  start.setDate(start.getDate() - leadingEmptyDays);
  const totalDays = heatmapDays + leadingEmptyDays;

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
