import { useEffect, useRef } from "react";
import type { HistoryRecord, LearningActivity } from "../types";

const chinese = {
  title: "来源语音",
  close: "关闭",
  transcript: "原始转写",
  corrected: "纠错后输出",
  edited: "用户修改后",
  model: "学习模型输出",
  savedTerms: "已写入词典",
  noSavedTerms: "未写入词条",
  missing: "未记录",
  noEdits: "未检测到手动修改",
  pendingModel: "等待学习结果",
};

const english = {
  title: "Source dictation",
  close: "Close",
  transcript: "Original transcript",
  corrected: "Corrected output",
  edited: "User-edited text",
  model: "Learning model output",
  savedTerms: "Saved to dictionary",
  noSavedTerms: "No corrections saved",
  missing: "Not recorded",
  noEdits: "No manual edits detected",
  pendingModel: "Waiting for the learning result",
};

export default function LearningSourceDialog({ record, activity, language, onClose }: {
  record: HistoryRecord;
  activity?: LearningActivity;
  language: string;
  onClose: () => void;
}) {
  const dialog = useRef<HTMLDialogElement>(null);
  const text = language === "en-US" ? english : chinese;
  const inProgress = activity?.finished_at === null;
  const stages = [
    { title: text.transcript, value: record.raw_text, empty: text.missing },
    { title: text.corrected, value: record.corrected_text || record.pasted_text, empty: text.missing },
    { title: text.edited, value: activity?.edited_text, empty: activity?.state === "no_change" ? text.noEdits : text.missing },
  ];

  useEffect(() => {
    dialog.current?.showModal();
  }, []);

  return (
    <dialog ref={dialog} className="learning-source-dialog" aria-labelledby="learning-source-title" onClose={onClose}>
      <header className="panel-header">
        <div>
          <h1 id="learning-source-title">{text.title}</h1>
          <time dateTime={record.created_at}>{new Date(record.created_at).toLocaleString(language)}</time>
        </div>
        <button className="secondary small" type="button" autoFocus onClick={onClose}>{text.close}</button>
      </header>
      <div className="learning-source-content" tabIndex={0} role="region" aria-labelledby="learning-source-title">
        {stages.map(({ title, value, empty }) => (
          <section key={title}>
            <h2>{title}</h2>
            {value ? <p>{value}</p> : <p className="learning-source-empty">{empty}</p>}
          </section>
        ))}
        <section>
          <h2>{text.model}</h2>
          {activity?.model_output ? <pre>{activity.model_output}</pre> : <p className="learning-source-empty">{inProgress ? text.pendingModel : text.missing}</p>}
          {activity?.entries.length ? (
            <>
              <h3>{text.savedTerms}</h3>
              <ul>
                {activity.entries.map((entry) => (
                  <li key={entry.id}>{entry.from ? <>{entry.from} → </> : null}<strong>{entry.term}</strong></li>
                ))}
              </ul>
            </>
          ) : activity?.model_output && !inProgress ? <p className="learning-source-empty">{text.noSavedTerms}</p> : null}
        </section>
      </div>
    </dialog>
  );
}
