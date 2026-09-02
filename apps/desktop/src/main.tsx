import React from "react";
import ReactDOM from "react-dom/client";
import { App } from "./App";
import { PanelWindow } from "./PanelWindow";
import { isPanelId } from "./panelSync";
import "./styles.css";

// A window opened with ?panel=<id> renders just that panel (a pop-out); the
// main window (no query) renders the full app.
const panel = new URLSearchParams(window.location.search).get("panel");

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>
    {isPanelId(panel) ? <PanelWindow panel={panel} /> : <App />}
  </React.StrictMode>,
);
