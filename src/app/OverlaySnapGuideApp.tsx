import { useEffect, type CSSProperties } from "react";
import { recordingOverlayEffectMargin } from "../domain/overlay";

export default function OverlaySnapGuideApp() {
  useEffect(() => {
    document.documentElement.classList.add("overlay-window", "overlay-snap-guide-window");
    document.body.classList.add("overlay-window", "overlay-snap-guide-window");

    return () => {
      document.documentElement.classList.remove("overlay-window", "overlay-snap-guide-window");
      document.body.classList.remove("overlay-window", "overlay-snap-guide-window");
    };
  }, []);

  return (
    <div
      className="overlay-snap-guide-shell"
      style={{ "--recording-overlay-effect-margin": `${recordingOverlayEffectMargin}px` } as CSSProperties}
      aria-hidden="true"
    >
      <div className="overlay-snap-guide" />
    </div>
  );
}
