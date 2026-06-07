import {Send, Sparkles} from "lucide-react";
import {PhotoProvider} from "react-photo-view";

import {Button} from "@/components/ui/button";
import {ScrollArea, ScrollBar} from "@/components/ui/scroll-area";
import type {ScannerCapturedDocument} from "../../store/scanner-store";
import {CapturedDocumentCard} from "./CapturedDocumentCard";

interface CapturedDocumentTrayProps {
  documents: ScannerCapturedDocument[];
  onRemove: (documentId: string) => void;
  onSendToAI: () => void;
  sendDisabled: boolean;
  sendLabel: string;
}

export function CapturedDocumentTray({
  documents,
  onRemove,
  onSendToAI,
  sendDisabled,
  sendLabel,
}: CapturedDocumentTrayProps) {
  if (documents.length === 0) {
    return null;
  }

  return (
    <div className="flex flex-col gap-4 rounded-xl border bg-muted/40 p-4 lg:h-full lg:w-80 lg:shrink-0 lg:p-6">
      <div className="flex items-center gap-2">
        <Sparkles className="h-5 w-5 text-primary" />
        <h3 className="font-semibold">{sendLabel}</h3>
      </div>

      <ScrollArea className="w-full lg:flex-1">
        <PhotoProvider portalContainer={typeof document !== "undefined" ? document.body : undefined}>
        <div className="flex gap-4 pb-4 lg:flex-col lg:pb-0 lg:pr-4">
          {documents.map((doc, index) => (
            <CapturedDocumentCard
              key={doc.id}
              document={doc}
              index={index}
              onRemove={onRemove}
            />
          ))}
        </div>
        </PhotoProvider>
        <ScrollBar orientation="horizontal" className="lg:hidden" />
      </ScrollArea>

      <div className="mt-auto shrink-0 pt-2 lg:pt-4">
        <Button
          onClick={onSendToAI}
          disabled={sendDisabled}
          className="w-full gap-2"
          size="lg"
        >
          <Send className="h-4 w-4" />
          {sendLabel}
        </Button>
      </div>
    </div>
  );
}
