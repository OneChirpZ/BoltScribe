export default function HelpTip({ content }: { content: string }) {
  return (
    <button className="help-tip" type="button" aria-label={content} data-tooltip={content}>
      ?
    </button>
  );
}
