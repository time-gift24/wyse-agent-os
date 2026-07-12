"use client"

import { useRef, type ReactNode } from "react"
import { motion, useInView } from "motion/react"

import { cn } from "~/lib/utils"

type AnimatedListProps = {
  children: ReactNode[]
  className?: string
  itemClassName?: string
  staggerDelay?: number
  maxDelay?: number
}

function AnimatedItem({
  children,
  index,
  staggerDelay,
  maxDelay,
}: {
  children: ReactNode
  index: number
  staggerDelay: number
  maxDelay: number
}) {
  const ref = useRef<HTMLDivElement>(null)
  const inView = useInView(ref, { amount: 0.3, once: true })

  return (
    <motion.div
      ref={ref}
      initial={{ opacity: 0, y: 8 }}
      animate={inView ? { opacity: 1, y: 0 } : { opacity: 0, y: 8 }}
      transition={{
        duration: 0.2,
        delay: Math.min(index * staggerDelay, maxDelay),
        ease: [0.16, 1, 0.3, 1] as const,
      }}
    >
      {children}
    </motion.div>
  )
}

export function AnimatedList({
  children,
  className,
  itemClassName,
  staggerDelay = 0.03,
  maxDelay = 0.24,
}: AnimatedListProps) {
  return (
    <div className={cn("flex flex-col", className)}>
      {children.map((child, index) => (
        <AnimatedItem
          key={index}
          index={index}
          staggerDelay={staggerDelay}
          maxDelay={maxDelay}
        >
          <div className={itemClassName}>{child}</div>
        </AnimatedItem>
      ))}
    </div>
  )
}
