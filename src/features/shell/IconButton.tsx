import type { ReactNode } from "react";

import { Button } from "@/components/ui/button";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/tooltip";
import { strings } from "@/i18n";

type IconButtonProps = {
  label: string;
  shortcut?: string;
  onClick?: () => void;
  disabled?: boolean;
  pressed?: boolean;
  children: ReactNode;
};

/** Icon-only button with an accessible name and a tooltip that includes its shortcut. */
export function IconButton({ label, shortcut, pressed, children, ...rest }: IconButtonProps) {
  return (
    <Tooltip>
      <TooltipTrigger
        render={<Button variant="ghost" size="icon" aria-label={label} aria-pressed={pressed} {...rest} />}
      >
        {children}
      </TooltipTrigger>
      <TooltipContent>{shortcut ? strings.toolbar.shortcutTooltip(label, shortcut) : label}</TooltipContent>
    </Tooltip>
  );
}
