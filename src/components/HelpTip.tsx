export default function HelpTip({ content }: { content: string }) {
  return (
    <span className="help-tip" tabIndex={0} aria-label={content} data-tooltip={content}>
      ?
    </span>
  );
}
