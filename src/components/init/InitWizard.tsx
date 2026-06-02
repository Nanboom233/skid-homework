"use client";
import { useCallback } from "react";
import { useRouter } from "next/navigation";
import { motion, AnimatePresence } from "framer-motion";
import { useInitStore } from "@/store/init-store";
import StepIndicator from "./StepIndicator";
import WizardNavigation from "./WizardNavigation";
import WelcomeStep from "./steps/WelcomeStep";
import AiConfigStep from "./steps/AiConfigStep";
import PreferencesStep from "./steps/PreferencesStep";
import AdvancedStep from "./steps/AdvancedStep";

const TOTAL_STEPS = 4;

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

const STEPS = [WelcomeStep, AiConfigStep, PreferencesStep, AdvancedStep];

export default function InitWizard() {
  const currentStep = useInitStore((s) => s.currentStep);
  const direction = useInitStore((s) => s.direction);
  const nextStep = useInitStore((s) => s.nextStep);
  const prevStep = useInitStore((s) => s.prevStep);
  const setInitCompleted = useInitStore((s) => s.setInitCompleted);
  const router = useRouter();

  const completeWizard = useCallback(() => {
    setInitCompleted(true);
    router.replace("/");
  }, [setInitCompleted, router]);

  const handleNext = useCallback(() => {
    if (currentStep === TOTAL_STEPS - 1) {
      completeWizard();
    } else {
      nextStep();
    }
  }, [currentStep, nextStep, completeWizard]);

  const handleSkip = useCallback(() => {
    if (currentStep === TOTAL_STEPS - 1) {
      completeWizard();
    } else {
      nextStep();
    }
  }, [currentStep, nextStep, completeWizard]);

  const StepComponent = STEPS[currentStep];

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
              <StepComponent />
            </motion.div>
          </AnimatePresence>
        </div>

        <WizardNavigation
          currentStep={currentStep}
          totalSteps={TOTAL_STEPS}
          onBack={prevStep}
          onNext={handleNext}
          onSkip={handleSkip}
        />
      </div>
    </div>
  );
}
