export default function AudioWaveform({
  samples,
  className = "",
}: {
  samples: number[];
  className?: string;
}) {
  return (
    <svg
      className={`voice-bars ${className}`.trim()}
      viewBox="0 0 68 42"
      preserveAspectRatio="none"
      focusable="false"
      aria-hidden="true"
    >
      {samples.map((sample, index) => {
        const level = Number.isFinite(sample) ? Math.min(1, Math.max(0, sample)) : 0;
        const height = 4 + level * 38;
        return (
          <rect
            key={index}
            x={index * 5}
            y={(42 - height) / 2}
            width="3"
            height={height}
            rx="1.5"
            opacity={0.5 + level * 0.5}
          />
        );
      })}
    </svg>
  );
}
