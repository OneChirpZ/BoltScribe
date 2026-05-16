import type { ReactNode } from "react";
import type { PermissionRequestState } from "../domain/permissions";
import type { TextBundle } from "../domain/i18n";

export default function PermissionGuide({
  accessibilityGranted,
  requiresAccessibility,
  microphonePermission,
  onClose,
  onRefreshAccessibility,
  onOpenAccessibility,
  onRequestMicrophone,
  text,
}: {
  accessibilityGranted: boolean | null;
  requiresAccessibility: boolean;
  microphonePermission: PermissionRequestState;
  onClose: () => void;
  onRefreshAccessibility: () => void;
  onOpenAccessibility: () => void;
  onRequestMicrophone: () => void;
  text: TextBundle;
}) {
  const allPermissionsGranted = (!requiresAccessibility || accessibilityGranted === true) && microphonePermission === "granted";

  return (
    <div className="modal-backdrop" role="presentation">
      <section className="permission-guide" role="dialog" aria-modal="true" aria-labelledby="permission-guide-title">
        <div className="panel-header">
          <div>
            <p className="eyebrow">{text.permission.guideEyebrow}</p>
            <div className="permission-guide-heading">
              <h1 id="permission-guide-title">{text.permission.guideTitle}</h1>
              {allPermissionsGranted ? <span className="permission-complete">{text.permission.complete}</span> : null}
            </div>
          </div>
          <button className="notice-close" type="button" onClick={onClose} aria-label={text.permission.closeGuide}>×</button>
        </div>
        <div className="permission-guide-grid">
          {requiresAccessibility ? (
            <PermissionCard
              title={text.permission.accessibility}
              status={accessibilityGranted ? text.permission.statusEnabled : text.permission.statusDisabled}
              tone={accessibilityGranted ? "ok" : "danger"}
              description={text.permission.accessibilityDescription}
              actions={(
                <>
                  <button className="secondary small" type="button" onClick={onRefreshAccessibility}>{text.permission.recheck}</button>
                  <button className="primary small" type="button" onClick={onOpenAccessibility}>{text.permission.openSettings}</button>
                </>
              )}
            />
          ) : null}
          <PermissionCard
            title={text.permission.microphone}
            status={microphoneStatusLabel(microphonePermission, text)}
            tone={microphonePermission === "granted" ? "ok" : microphonePermission === "denied" ? "danger" : "muted"}
            description={text.permission.microphoneDescription}
            actions={(
              <button
                className="primary small"
                type="button"
                disabled={microphonePermission === "checking"}
                onClick={onRequestMicrophone}
              >
                {microphonePermission === "checking" ? text.common.checking : text.permission.requestMicrophone}
              </button>
            )}
          />
        </div>
        <div className="modal-footer">
          <button className="secondary" type="button" onClick={onClose}>{text.common.later}</button>
        </div>
      </section>
    </div>
  );
}

function microphoneStatusLabel(status: PermissionRequestState, text: TextBundle) {
  switch (status) {
    case "checking":
      return text.common.checking;
    case "granted":
      return text.permission.statusEnabled;
    case "denied":
      return text.permission.statusDisabled;
    default:
      return text.permission.statusNotRequested;
  }
}

function PermissionCard({
  title,
  status,
  tone,
  description,
  actions,
}: {
  title: string;
  status: string;
  tone: "ok" | "danger" | "muted";
  description: string;
  actions: ReactNode;
}) {
  return (
    <article className="permission-card">
      <div className="permission-card-header">
        <h2>{title}</h2>
        <span className={`permission-status ${tone}`}>{status}</span>
      </div>
      <p>{description}</p>
      <div className="permission-card-actions">{actions}</div>
    </article>
  );
}
