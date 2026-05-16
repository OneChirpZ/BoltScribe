import MainApp from "./MainApp";
import OverlayApp from "./OverlayApp";

function isOverlayWindow() {
  return new URL(window.location.href).searchParams.get("window") === "overlay";
}

export default function App() {
  if (isOverlayWindow()) {
    return <OverlayApp />;
  }

  return <MainApp />;
}
