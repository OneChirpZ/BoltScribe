import { useEffect } from "react";

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
    <div className="overlay-snap-guide-shell" aria-hidden="true">
      <div className="overlay-snap-guide" />
    </div>
  );
}
