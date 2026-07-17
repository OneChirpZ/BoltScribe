import type { HistoryRecord } from "../types";
import HistoryPage from "../components/HistoryPage";
import type { TextBundle } from "../domain/i18n";

export default function HistoryRecordsPage({
  history,
  pageIndex,
  pageSize,
  hasOlder,
  onRefresh,
  onPreviousPage,
  onNextPage,
  onCopyHistory,
  onDeleteHistory,
  canDeleteHistory,
  text,
}: {
  history: HistoryRecord[];
  pageIndex: number;
  pageSize: number;
  hasOlder: boolean;
  onRefresh: () => void;
  onPreviousPage: () => void;
  onNextPage: () => void;
  onCopyHistory: (text: string, label: string) => void;
  onDeleteHistory: (record: HistoryRecord) => Promise<void>;
  canDeleteHistory: boolean;
  text: TextBundle;
}) {
  return (
    <div className="page-stack">
      <HistoryPage
        title={text.history.pageTitle(pageIndex + 1)}
        history={history}
        onRefresh={onRefresh}
        onCopy={onCopyHistory}
        onDelete={onDeleteHistory}
        canDelete={canDeleteHistory}
        text={text}
        footer={
          <div className="history-pagination">
            <button className="secondary small" type="button" disabled={pageIndex === 0} onClick={onPreviousPage}>{text.history.previous}</button>
            <span>{text.history.perPage(pageSize)}</span>
            <button className="secondary small" type="button" disabled={!hasOlder} onClick={onNextPage}>{text.history.next}</button>
          </div>
        }
      />
    </div>
  );
}
