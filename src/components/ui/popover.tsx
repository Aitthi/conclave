import * as React from "react";
import { Popover as PopoverPrimitive } from "@base-ui-components/react/popover";

const Popover = PopoverPrimitive.Root;
const PopoverTrigger = PopoverPrimitive.Trigger;

type PopoverContentProps = React.ComponentProps<typeof PopoverPrimitive.Popup> &
  Pick<
    React.ComponentProps<typeof PopoverPrimitive.Positioner>,
    "align" | "side" | "sideOffset"
  >;

const PopoverContent = React.forwardRef<HTMLDivElement, PopoverContentProps>(
  ({ align = "center", side = "bottom", sideOffset = 4, className = "", ...props }, ref) => (
    <PopoverPrimitive.Portal>
      <PopoverPrimitive.Positioner align={align} side={side} sideOffset={sideOffset}>
        <PopoverPrimitive.Popup
          ref={ref}
          className={`z-50 w-72 rounded-lg bg-surface p-3 text-[11.5px] text-text-secondary shadow-lg ring-hair outline-none ${className}`}
          {...props}
        />
      </PopoverPrimitive.Positioner>
    </PopoverPrimitive.Portal>
  ),
);
PopoverContent.displayName = "PopoverContent";

const PopoverTitle = React.forwardRef<
  HTMLHeadingElement,
  React.ComponentProps<typeof PopoverPrimitive.Title>
>(({ className = "", ...props }, ref) => (
  <PopoverPrimitive.Title
    ref={ref}
    className={`text-xs font-semibold text-text-primary ${className}`}
    {...props}
  />
));
PopoverTitle.displayName = "PopoverTitle";

const PopoverDescription = React.forwardRef<
  HTMLParagraphElement,
  React.ComponentProps<typeof PopoverPrimitive.Description>
>(({ className = "", ...props }, ref) => (
  <PopoverPrimitive.Description
    ref={ref}
    className={`text-[11px] leading-relaxed text-text-secondary ${className}`}
    {...props}
  />
));
PopoverDescription.displayName = "PopoverDescription";

export { Popover, PopoverContent, PopoverDescription, PopoverTitle, PopoverTrigger };
