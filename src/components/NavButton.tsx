import type { Page } from "../domain/navigation";

export default function NavButton({
  current,
  page,
  label,
  onClick,
  ariaExpanded,
}: {
  current: Page;
  page: Page;
  label: string;
  onClick: (page: Page) => void;
  ariaExpanded?: boolean;
}) {
  return (
    <button
      className={current === page ? "nav-button active" : "nav-button"}
      type="button"
      aria-current={current === page ? "page" : undefined}
      aria-expanded={ariaExpanded}
      onClick={() => onClick(page)}
    >
      {label}
    </button>
  );
}
