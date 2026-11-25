import { StatusProvider } from "./lib/status";
import { TrayPopup } from "./pages/TrayPopup";

/** Окно трей-попапа (?view=tray): свой статус и только попап. */
export function TrayRoot() {
  return (
    <StatusProvider>
      <TrayPopup />
    </StatusProvider>
  );
}
