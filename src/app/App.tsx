import MainApp from "./MainApp";
import OverlayApp from "./OverlayApp";
import OverlaySnapGuideApp from "./OverlaySnapGuideApp";

function windowKind() {
  return new URL(window.location.href).searchParams.get("window");
}

export default function App() {
  const kind = windowKind();
  if (kind === "overlay") {
    return <OverlayApp />;
  }
  if (kind === "overlay-snap-guide") {
    return <OverlaySnapGuideApp />;
  }

  return <MainApp />;
}
