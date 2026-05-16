export default function TokenButton({ token, onClick }: { token: string; onClick: (token: string) => void }) {
  return (
    <button
      className="token-button"
      type="button"
      onMouseDown={(event) => event.preventDefault()}
      onClick={() => onClick(token)}
      title={`插入 ${token}`}
    >
      {token}
    </button>
  );
}
