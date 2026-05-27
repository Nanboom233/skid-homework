"use client";

import {SerwistProvider} from "@/app/serwist";
import {shouldEnableSerwist} from "@/platform";

/**
 * Conditionally wraps children with SerwistProvider.
 * Skips Service Worker registration when running inside Tauri,
 * since native desktop apps do not need PWA capabilities.
 */
export function TauriAwareSerwist({
  children,
}: {
  children: React.ReactNode;
}) {
  if (!shouldEnableSerwist()) {
    return <>{children}</>;
  }

  return (
    <SerwistProvider
      swUrl="/sw.js"
      disable={process.env.NODE_ENV !== "production"}
      options={{ updateViaCache: "none" }}
    >
      {children}
    </SerwistProvider>
  );
}
