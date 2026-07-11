import type { Page } from "../domain/navigation";

export default function NavButton({
  current,
  page,
  label,
  onClick,
}: {
  current: Page;
  page: Page;
  label: string;
  onClick: (page: Page) => void;
}) {
  return (
    <button
      className={current === page ? "nav-button active" : "nav-button"}
      type="button"
      aria-current={current === page ? "page" : undefined}
      onClick={() => onClick(page)}
    >
      {label}
    </button>
  );
}
