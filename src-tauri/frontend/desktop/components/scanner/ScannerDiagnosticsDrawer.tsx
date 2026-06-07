import {useTranslation} from "react-i18next";
import {
  Sheet,
  SheetContent,
  SheetHeader,
  SheetTitle,
} from "@/components/ui/sheet";
import {ScannerDebugPanel} from "./ScannerDebugPanel";

interface ScannerDiagnosticsDrawerProps {
  open: boolean;
  onOpenChange: (open: boolean) => void;
}

export function ScannerDiagnosticsDrawer({
  open,
  onOpenChange,
}: ScannerDiagnosticsDrawerProps) {
  const {t} = useTranslation("commons", {keyPrefix: "document-scanner"});

  return (
    <Sheet open={open} onOpenChange={onOpenChange}>
      <SheetContent
        side="right"
        className="flex w-full flex-col gap-0 border-l p-0 sm:max-w-md md:max-w-lg"
      >
        <SheetHeader className="sticky top-0 z-10 shrink-0 border-b bg-background/80 px-6 py-4 backdrop-blur-sm">
          <SheetTitle>{t("diagnostics.title", "Scanner Diagnostics")}</SheetTitle>
        </SheetHeader>
        <div className="flex-1 overflow-y-auto p-4">
          <ScannerDebugPanel />
        </div>
      </SheetContent>
    </Sheet>
  );
}
