import { cva, type VariantProps } from "class-variance-authority";
import type { HTMLAttributes } from "react";
import { cn } from "@/lib/utils";

const badgeVariants = cva(
  "inline-flex items-center gap-1 rounded-full px-2 py-0.5 text-xs font-medium whitespace-nowrap [&_svg]:size-3",
  {
    variants: {
      variant: {
        secondary: "bg-zinc-100 text-zinc-600 dark:bg-zinc-800 dark:text-zinc-300",
        warning: "bg-amber-100 text-amber-800 dark:bg-amber-500/15 dark:text-amber-300",
      },
    },
    defaultVariants: { variant: "secondary" },
  },
);

type BadgeProps = HTMLAttributes<HTMLSpanElement> & VariantProps<typeof badgeVariants>;

export function Badge({ className, variant, ...props }: BadgeProps) {
  return <span className={cn(badgeVariants({ variant }), className)} {...props} />;
}
