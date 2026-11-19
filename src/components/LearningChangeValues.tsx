import { learningChangeText } from "../domain/learningActivity";
import type { LearningChange, LearningEntryValue } from "../types";

export function LearningChangeValues({ change, language }: { change: LearningChange; language: string }) {
  const text = learningChangeText(language);
  function describe(value: LearningEntryValue) {
    return <>
      {value.from ? <>{value.from} <span aria-hidden="true">→</span> </> : null}
      <strong>{value.term}</strong>
      {value.enabled ? null : <span className="learning-change-paused">{text.paused}</span>}
    </>;
  }
  return (
    <dl className="learning-change-values">
      {change.before ? <div><dt>{text.before}</dt><dd>{describe(change.before)}</dd></div> : null}
      {change.after ? <div><dt>{text.after}</dt><dd>{describe(change.after)}</dd></div> : null}
    </dl>
  );
}
