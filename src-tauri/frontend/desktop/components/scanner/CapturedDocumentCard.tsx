import {AlertCircle, Check, Loader2, Trash2} from "lucide-react";
import {useTranslation} from "react-i18next";
import {PhotoView} from "react-photo-view";

import {Badge} from "@/components/ui/badge";
import {Button} from "@/components/ui/button";
import {Card} from "@/components/ui/card";
import {useBlobDataUrl} from "@/hooks/use-blob-data-url";
import type {ScannerCapturedDocument} from "../../store/scanner-store";
import {cn} from "@/lib/utils";

interface CapturedDocumentCardProps {
  document: ScannerCapturedDocument;
  index: number;
  onRemove: (documentId: string) => void;
}

export function CapturedDocumentCard({
  document,
  index,
  onRemove,
}: CapturedDocumentCardProps) {
  const {t} = useTranslation("commons", {keyPrefix: "document-scanner.captured"});
  const previewUrl = useBlobDataUrl(document.file);
  const isProcessing = document.status === "processing";
  const isFailed = document.status === "failed";

  return (
    <Card className={cn(
      "group relative flex shrink-0 overflow-hidden transition-all hover:shadow-md lg:w-full lg:flex-col",
      "w-24 flex-col lg:h-auto",
      isProcessing && "opacity-80",
    )}>
      <div
        className={cn(
          "relative block w-full bg-muted/40",
          "h-32 lg:h-48",
          !isProcessing && "cursor-pointer",
        )}
      >
        {previewUrl ? (
          <PhotoView src={previewUrl}>
            {/* eslint-disable-next-line @next/next/no-img-element */}
            <img
              src={previewUrl}
              alt={t("preview-alt", {index: index + 1, defaultValue: `Document ${index + 1}`})}
              className="h-full w-full object-contain"
            />
          </PhotoView>
        ) : (
          <div className="flex h-full w-full items-center justify-center">
            {isProcessing ? (
              <Loader2 className="h-6 w-6 animate-spin text-muted-foreground" />
            ) : (
              <div className="h-full w-full bg-muted/40" />
            )}
          </div>
        )}

        <div className="absolute inset-x-0 bottom-0 bg-gradient-to-t from-black/60 to-transparent p-2 pt-6 lg:p-3 lg:pt-8">
          <Badge
            variant={isFailed ? "destructive" : isProcessing ? "secondary" : "default"}
            className="gap-1 border-0 bg-black/40 text-white shadow-none hover:bg-black/40"
          >
            {isProcessing && <Loader2 className="h-3 w-3 animate-spin" />}
            {isFailed && <AlertCircle className="h-3 w-3" />}
            {!isProcessing && !isFailed && <Check className="h-3 w-3" />}
            <span className="text-[10px] lg:text-xs">
              {isProcessing ? t("status.processing", "Processing") :
               isFailed ? t("status.failed", "Failed") :
               t("status.ready", "Ready")}
            </span>
          </Badge>
        </div>
      </div>

      <div className="absolute right-1 top-1 flex flex-col gap-1 transition-opacity lg:right-2 lg:top-2 lg:flex-row opacity-100 lg:opacity-0 lg:group-hover:opacity-100 lg:group-focus-within:opacity-100">
        <Button
          variant="destructive"
          size="icon"
          className="h-6 w-6 rounded-full shadow-sm lg:h-8 lg:w-8"
          onClick={(e) => {
            e.stopPropagation();
            onRemove(document.id);
          }}
        >
          <Trash2 className="h-3 w-3 lg:h-4 lg:w-4" />
          <span className="sr-only">{t("actions.remove", "Remove")}</span>
        </Button>
      </div>
    </Card>
  );
}
