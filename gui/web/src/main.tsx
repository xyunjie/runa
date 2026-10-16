import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { App } from "./App";
import { applyTheme } from "./lib/theme";
import "./index.css";

applyTheme();
// Screenshots (?static=1): disable animations so headless captures show the final state.
if (new URLSearchParams(location.search).get("static") === "1") {
  const st = document.createElement("style");
  st.textContent = "*,*::before,*::after{animation-duration:0s!important;animation-delay:0s!important;transition:none!important}";
  document.head.appendChild(st);
}
createRoot(document.getElementById("root")!).render(
  <StrictMode>
    <App />
  </StrictMode>,
);
