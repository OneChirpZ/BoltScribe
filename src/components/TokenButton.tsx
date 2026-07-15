export default function TokenButton({ token, onClick, label }: { token: string; onClick: (token: string) => void; label?: string }) {
  return (
    <button
      className="token-button"
      type="button"
      onMouseDown={(event) => event.preventDefault()}
      onClick={() => onClick(token)}
      aria-label={label}
      title={label ?? `插入 ${token}`}
    >
      {token}
    </button>
  );
}
