import { createRoot } from "react-dom/client";
import { App } from "./App";
import { initLanguage } from "./i18n";
import { initTheme } from "./theme";
import "@fontsource-variable/inter";
import "@fontsource-variable/jetbrains-mono";
import "@fontsource-variable/orbitron";
import "./index.css";

initTheme();
initLanguage();

createRoot(document.getElementById("root")!).render(<App />);
