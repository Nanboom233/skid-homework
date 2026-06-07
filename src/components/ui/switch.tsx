"use client";

import * as React from "react";

import {cn} from "@/lib/utils";

type SwitchProps = Omit<React.ButtonHTMLAttributes<HTMLButtonElement>, "onChange" | "value"> & {
  checked?: boolean;
  onCheckedChange?: (checked: boolean) => void;
};

function Switch({
  className,
  checked = false,
  disabled,
  onCheckedChange,
  onClick,
  ...props
}: SwitchProps) {
  return (
    <button
      type="button"
      role="switch"
      aria-checked={checked}
      data-slot="switch"
      data-state={checked ? "checked" : "unchecked"}
      disabled={disabled}
      className={cn(
        "peer inline-flex h-[1.15rem] w-8 shrink-0 items-center rounded-full border border-transparent shadow-xs transition-all outline-none focus-visible:border-ring focus-visible:ring-[3px] focus-visible:ring-ring/50 disabled:cursor-not-allowed disabled:opacity-50",
        checked ? "bg-primary" : "bg-input dark:bg-input/80",
        className,
      )}
      onClick={(event) => {
        onClick?.(event);
        if (!event.defaultPrevented && !disabled) {
          onCheckedChange?.(!checked);
        }
      }}
      {...props}
    >
      <span
        data-slot="switch-thumb"
        data-state={checked ? "checked" : "unchecked"}
        className={cn(
          "pointer-events-none block size-4 rounded-full bg-background ring-0 transition-transform dark:bg-foreground",
          checked
            ? "translate-x-[calc(100%-2px)] dark:bg-primary-foreground"
            : "translate-x-0",
        )}
      />
    </button>
  );
}

export {Switch};
