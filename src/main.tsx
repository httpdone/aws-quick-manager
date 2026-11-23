import React from "react";
import ReactDOM from "react-dom/client";
import App from "./App";
import { UIProvider } from "./components/ui";
import { applyTheme } from "./lib/prefs";
import "./styles.css";

applyTheme();

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>
    <UIProvider>
      <App />
    </UIProvider>
  </React.StrictMode>,
);
