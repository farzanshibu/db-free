// SOT: icon-button, tooltip-button
import { Button } from "@/components/ui/button";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/tooltip";
import { Icon, type IconName } from "@/lib/icons";
import { cn } from "@/lib/cn";

interface IconButtonProps {
  icon: IconName;
  label: string;
  onClick?: () => void;
  disabled?: boolean;
  active?: boolean;
  size?: number;
  className?: string;
}

// WHAT:  An icon-only button whose tooltip carries its accessible label.
// WHY:   Toolbars are icons alone; without the tooltip the label exists only for
//        screen readers, and without `aria-label` it exists only for sighted users.
//        Pairing them here means neither can be forgotten at a call site.
// WHERE: https://ui.shadcn.com/docs/components/tooltip
export function IconButton({ icon, label, onClick, disabled = false, active = false, size = 15, className }: IconButtonProps) {
  return (
    <Tooltip>
      <TooltipTrigger asChild>
        <Button
          aria-label={label}
          variant={active ? "secondary" : "ghost"}
          size="icon-sm"
          disabled={disabled}
          {...(onClick ? { onClick } : {})}
          className={cn("rounded-lg liquid-hover", active ? "glass-pill border-0 text-accent" : "", className)}
        >
          <Icon name={icon} size={size} />
        </Button>
      </TooltipTrigger>
      <TooltipContent>{label}</TooltipContent>
    </Tooltip>
  );
}
