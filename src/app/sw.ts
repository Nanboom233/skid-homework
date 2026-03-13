/// <reference lib="webworker" />

import {defaultCache} from "@serwist/next/worker";
import type {PrecacheEntry, SerwistGlobalConfig} from "serwist";
import {Serwist} from "serwist";

type SerwistWorkerGlobalScope = WorkerGlobalScope &
  SerwistGlobalConfig & {
    __SW_MANIFEST: (PrecacheEntry | string)[] | undefined;
  };

const swSelf = globalThis as typeof globalThis & SerwistWorkerGlobalScope;

const serwist = new Serwist({
  precacheEntries: swSelf.__SW_MANIFEST,
  skipWaiting: true,
  clientsClaim: true,
  runtimeCaching: defaultCache,
});

serwist.addEventListeners();
