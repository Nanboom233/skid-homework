"use client";
import { isTauriAndroidTarget } from "@/platform";
import InitWizard from "@/components/init/InitWizard";
import MobileInitFallback from "@/components/init/MobileInitFallback";

export default function InitPage() {
  if (isTauriAndroidTarget) {
    return <MobileInitFallback />;
  }

  return <InitWizard />;
}
