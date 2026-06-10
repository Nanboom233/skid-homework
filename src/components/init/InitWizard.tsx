"use client";
import { useCallback, useMemo } from "react";
import { useTranslation } from "react-i18next";
import { useRouter } from "next/navigation";
import { motion, AnimatePresence } from "framer-motion";
import { useInitStore } from "@/store/init-store";
import { useAvailableModels } from "@/hooks/use-available-models";
import { PlatformScannerSetupStep } from "@/platform";
import StepIndicator from "./StepIndicator";
import WizardNavigation from "./WizardNavigation";
import WelcomeStep from "./steps/WelcomeStep";
import AiConfigStep from "./steps/AiConfigStep";
import PreferencesStep from "./steps/PreferencesStep";
import AdvancedStep from "./steps/AdvancedStep";

const TOTAL_STEPS = 5;

const variants = {
  enter: (direction: number) => ({
    opacity: 0,
    y: direction > 0 ? 24 : -24,
  }),
  center: {
    opacity: 1,
    y: 0,
  },
  exit: (direction: number) => ({
    opacity: 0,
    y: direction > 0 ? -24 : 24,
  }),
};

const transition = {
  duration: 0.25,
  ease: [0.25, 0.1, 0.25, 1] as const,
};

export default function InitWizard() {
  const currentStep = useInitStore((s) => s.currentStep);
  const direction = useInitStore((s) => s.direction);
  const prevStep = useInitStore((s) => s.prevStep);
  const setCurrentStep = useInitStore((s) => s.setCurrentStep);
  const setInitCompleted = useInitStore((s) => s.setInitCompleted);
  const { t } = useTranslation("commons", { keyPrefix: "init-page.navigation" });
  const router = useRouter();

  // Lift model fetching to share between steps
  const { sourceModelsMap, allModels, isLoading, fetchErrors, hasFetched } = useAvailableModels();
  const hasValidConfig = allModels.length > 0;

  const completeWizard = useCallback(() => {
    setInitCompleted(true);
    router.replace("/");
  }, [setInitCompleted, router]);

  const advanceToNextStep = useCallback(() => {
    setCurrentStep(Math.min(currentStep + 1, TOTAL_STEPS - 1));
  }, [currentStep, setCurrentStep]);

  const handleNext = useCallback(() => {
    if (currentStep >= 3) {
      // Steps 3 (Preferences) and 4 (Advanced) both finish the wizard
      completeWizard();
    } else {
      advanceToNextStep();
    }
  }, [currentStep, advanceToNextStep, completeWizard]);

  const handleSkip = useCallback(() => {
    // Skip applies to step 1 (AI Config) and step 2 (Scanner Setup)
    advanceToNextStep();
  }, [advanceToNextStep]);

  const stepContent = useMemo(() => {
    switch (currentStep) {
      case 0:
        return <WelcomeStep />;
      case 1:
        return <AiConfigStep allModels={allModels} isLoadingModels={isLoading} fetchErrors={fetchErrors} hasFetched={hasFetched} />;
      case 2:
        return <PlatformScannerSetupStep />;
      case 3:
        return (
          <PreferencesStep
            sourceModelsMap={sourceModelsMap}
            allModels={allModels}
            isLoadingModels={isLoading}
          />
        );
      case 4:
        return (
          <AdvancedStep
            sourceModelsMap={sourceModelsMap}
            allModels={allModels}
            isLoadingModels={isLoading}
          />
        );
      default:
        return <WelcomeStep />;
    }
  }, [currentStep, sourceModelsMap, allModels, isLoading, fetchErrors, hasFetched]);

  return (
    <div className="flex min-h-screen items-center justify-center bg-background p-4">
      <div className="w-full max-w-xl rounded-xl border bg-card p-8 shadow-md">
        <StepIndicator totalSteps={TOTAL_STEPS} currentStep={currentStep} />

        <div className="mt-6 min-h-[360px]">
          <AnimatePresence mode="wait" custom={direction}>
            <motion.div
              key={currentStep}
              custom={direction}
              variants={variants}
              initial="enter"
              animate="center"
              exit="exit"
              transition={transition}
            >
              {stepContent}
            </motion.div>
          </AnimatePresence>
        </div>

        <WizardNavigation
          currentStep={currentStep}
          totalSteps={TOTAL_STEPS}
          onBack={prevStep}
          onNext={handleNext}
          onSkip={handleSkip}
          showSkip={currentStep === 1 || currentStep === 2}
          showAdvanced={currentStep === 3}
          onAdvanced={advanceToNextStep}
          finishLabel={currentStep >= 3 ? t("finish") : undefined}
          disableNext={currentStep === 1 && hasFetched && !hasValidConfig && !isLoading}
        />
      </div>
    </div>
  );
}
