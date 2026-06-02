"use client";
import { Button } from "@/components/ui/button";
import { useTranslation } from "react-i18next";

interface WizardNavigationProps {
  currentStep: number;
  totalSteps: number;
  onBack: () => void;
  onNext: () => void;
  onSkip: () => void;
}

export default function WizardNavigation({
  currentStep,
  totalSteps,
  onBack,
  onNext,
  onSkip,
}: WizardNavigationProps) {
  const { t } = useTranslation("commons", { keyPrefix: "init-page.navigation" });
  const isFirst = currentStep === 0;
  const isLast = currentStep === totalSteps - 1;

  return (
    <div className="flex items-center justify-between pt-6">
      <Button
        variant="ghost"
        onClick={onBack}
        disabled={isFirst}
        className={isFirst ? "invisible" : ""}
      >
        {t("back")}
      </Button>

      <div className="flex items-center gap-2">
        <Button variant="ghost" onClick={onSkip} className="text-muted-foreground">
          {t("skip")}
        </Button>
        <Button onClick={onNext}>
          {isLast ? t("finish") : t("next")}
        </Button>
      </div>
    </div>
  );
}
