import { useEffect, useRef, useState, useId, type CSSProperties } from "react"
import "./GlassSurface.css"

type GlassSurfaceProps = {
  children?: React.ReactNode
  width?: number | string
  height?: number | string
  borderRadius?: number
  borderWidth?: number
  brightness?: number
  brightnessDark?: number
  opacity?: number
  opacityDark?: number
  blur?: number
  blurDark?: number
  displace?: number
  backgroundOpacity?: number
  backgroundOpacityDark?: number
  saturation?: number
  saturationDark?: number
  distortionScale?: number
  redOffset?: number
  greenOffset?: number
  blueOffset?: number
  xChannel?: string
  yChannel?: string
  mixBlendMode?: string
  className?: string
  style?: CSSProperties
}

const isDarkMode = () => {
  if (typeof document === "undefined") return false
  return document.documentElement.classList.contains("dark")
}

const GlassSurface = ({
  children,
  width = 200,
  height = 80,
  borderRadius = 20,
  borderWidth = 0.07,
  brightness = 50,
  brightnessDark,
  opacity = 0.93,
  opacityDark,
  blur = 11,
  blurDark,
  displace = 0,
  backgroundOpacity = 0,
  backgroundOpacityDark,
  saturation = 1,
  saturationDark,
  distortionScale = -180,
  redOffset = 0,
  greenOffset = 10,
  blueOffset = 20,
  xChannel = "R",
  yChannel = "G",
  mixBlendMode = "difference",
  className = "",
  style = {},
}: GlassSurfaceProps) => {
  const uniqueId = useId().replace(/:/g, "-")
  const filterId = `glass-filter-${uniqueId}`
  const redGradId = `red-grad-${uniqueId}`
  const blueGradId = `blue-grad-${uniqueId}`

  const [svgSupported, setSvgSupported] = useState(false)
  const [dark, setDark] = useState(() => isDarkMode())

  const containerRef = useRef<HTMLDivElement>(null)
  const feImageRef = useRef<SVGFEImageElement>(null)
  const redChannelRef = useRef<SVGFEDisplacementMapElement>(null)
  const greenChannelRef = useRef<SVGFEDisplacementMapElement>(null)
  const blueChannelRef = useRef<SVGFEDisplacementMapElement>(null)
  const gaussianBlurRef = useRef<SVGFEGaussianBlurElement>(null)

  useEffect(() => {
    if (typeof window === "undefined") return

    const update = () => setDark(isDarkMode())
    const media = window.matchMedia("(prefers-color-scheme: dark)")
    const observer = new MutationObserver(update)

    media.addEventListener("change", update)
    observer.observe(document.documentElement, {
      attributes: true,
      attributeFilter: ["class"],
    })

    return () => {
      media.removeEventListener("change", update)
      observer.disconnect()
    }
  }, [])

  const effectiveBrightness = dark ? (brightnessDark ?? brightness) : brightness
  const effectiveOpacity = dark ? (opacityDark ?? opacity) : opacity
  const effectiveBlur = dark ? (blurDark ?? blur) : blur
  const effectiveBackgroundOpacity = dark
    ? (backgroundOpacityDark ?? backgroundOpacity)
    : backgroundOpacity
  const effectiveSaturation = dark ? (saturationDark ?? saturation) : saturation

  const generateDisplacementMap = () => {
    const rect = containerRef.current?.getBoundingClientRect()
    const actualWidth = rect?.width || 400
    const actualHeight = rect?.height || 200
    const edgeSize = Math.min(actualWidth, actualHeight) * (borderWidth * 0.5)

    const svgContent = `
      <svg viewBox="0 0 ${actualWidth} ${actualHeight}" xmlns="http://www.w3.org/2000/svg">
        <defs>
          <linearGradient id="${redGradId}" x1="100%" y1="0%" x2="0%" y2="0%">
            <stop offset="0%" stop-color="#0000"/>
            <stop offset="100%" stop-color="red"/>
          </linearGradient>
          <linearGradient id="${blueGradId}" x1="0%" y1="0%" x2="0%" y2="100%">
            <stop offset="0%" stop-color="#0000"/>
            <stop offset="100%" stop-color="blue"/>
          </linearGradient>
        </defs>
        <rect x="0" y="0" width="${actualWidth}" height="${actualHeight}" fill="black"></rect>
        <rect x="0" y="0" width="${actualWidth}" height="${actualHeight}" rx="${borderRadius}" fill="url(#${redGradId})" />
        <rect x="0" y="0" width="${actualWidth}" height="${actualHeight}" rx="${borderRadius}" fill="url(#${blueGradId})" style="mix-blend-mode: ${mixBlendMode}" />
        <rect x="${edgeSize}" y="${edgeSize}" width="${actualWidth - edgeSize * 2}" height="${actualHeight - edgeSize * 2}" rx="${borderRadius}" fill="hsl(0 0% ${effectiveBrightness}% / ${effectiveOpacity})" style="filter:blur(${effectiveBlur}px)" />
      </svg>
    `

    return `data:image/svg+xml,${encodeURIComponent(svgContent)}`
  }

  const updateDisplacementMap = () => {
    feImageRef.current?.setAttribute("href", generateDisplacementMap())
  }

  useEffect(() => {
    updateDisplacementMap()
    ;[
      { ref: redChannelRef, offset: redOffset },
      { ref: greenChannelRef, offset: greenOffset },
      { ref: blueChannelRef, offset: blueOffset },
    ].forEach(({ ref, offset }) => {
      if (ref.current) {
        ref.current.setAttribute("scale", (distortionScale + offset).toString())
        ref.current.setAttribute("xChannelSelector", xChannel)
        ref.current.setAttribute("yChannelSelector", yChannel)
      }
    })

    gaussianBlurRef.current?.setAttribute("stdDeviation", displace.toString())
  }, [
    width,
    height,
    borderRadius,
    borderWidth,
    effectiveBrightness,
    effectiveOpacity,
    effectiveBlur,
    displace,
    distortionScale,
    redOffset,
    greenOffset,
    blueOffset,
    xChannel,
    yChannel,
    mixBlendMode,
  ])

  useEffect(() => {
    if (!containerRef.current) return

    const resizeObserver = new ResizeObserver(() => {
      setTimeout(updateDisplacementMap, 0)
    })

    resizeObserver.observe(containerRef.current)

    return () => {
      resizeObserver.disconnect()
    }
  }, [])

  useEffect(() => {
    setTimeout(updateDisplacementMap, 0)
  }, [width, height])

  useEffect(() => {
    setSvgSupported(supportsSVGFilters())
  }, [])

  const supportsSVGFilters = () => {
    if (typeof window === "undefined" || typeof document === "undefined") {
      return false
    }

    const isWebkit =
      /Safari/.test(navigator.userAgent) && !/Chrome/.test(navigator.userAgent)
    const isFirefox = /Firefox/.test(navigator.userAgent)

    if (isWebkit || isFirefox) {
      return false
    }

    const div = document.createElement("div")
    div.style.backdropFilter = `url(#${filterId})`

    return div.style.backdropFilter !== ""
  }

  const containerStyle: CSSProperties = {
    ...style,
    width: typeof width === "number" ? `${width}px` : width,
    height: typeof height === "number" ? `${height}px` : height,
    borderRadius: `${borderRadius}px`,
    ["--glass-frost" as string]: effectiveBackgroundOpacity,
    ["--glass-saturation" as string]: effectiveSaturation,
    ["--filter-id" as string]: `url(#${filterId})`,
  }

  return (
    <div
      ref={containerRef}
      className={`glass-surface ${
        svgSupported ? "glass-surface--svg" : "glass-surface--fallback"
      } ${className}`}
      style={containerStyle}
    >
      <svg className="glass-surface__filter" xmlns="http://www.w3.org/2000/svg">
        <defs>
          <filter
            id={filterId}
            colorInterpolationFilters="sRGB"
            x="0%"
            y="0%"
            width="100%"
            height="100%"
          >
            <feImage
              ref={feImageRef}
              x="0"
              y="0"
              width="100%"
              height="100%"
              preserveAspectRatio="none"
              result="map"
            />

            <feDisplacementMap
              ref={redChannelRef}
              in="SourceGraphic"
              in2="map"
              id="redchannel"
              result="dispRed"
            />
            <feColorMatrix
              in="dispRed"
              type="matrix"
              values="1 0 0 0 0
                      0 0 0 0 0
                      0 0 0 0 0
                      0 0 0 1 0"
              result="red"
            />

            <feDisplacementMap
              ref={greenChannelRef}
              in="SourceGraphic"
              in2="map"
              id="greenchannel"
              result="dispGreen"
            />
            <feColorMatrix
              in="dispRed"
              type="matrix"
              values="0 0 0 0 0
                      0 1 0 0 0
                      0 0 0 0 0
                      0 0 0 1 0"
              result="green"
            />

            <feDisplacementMap
              ref={blueChannelRef}
              in="SourceGraphic"
              in2="map"
              id="bluechannel"
              result="dispBlue"
            />
            <feColorMatrix
              in="dispBlue"
              type="matrix"
              values="0 0 0 0 0
                      0 0 0 0 0
                      0 0 1 0 0
                      0 0 0 1 0"
              result="blue"
            />

            <feBlend in="red" in2="green" mode="screen" result="rg" />
            <feBlend in="rg" in2="blue" mode="screen" result="output" />
            <feGaussianBlur
              ref={gaussianBlurRef}
              in="output"
              stdDeviation="0.7"
            />
          </filter>
        </defs>
      </svg>

      <div className="glass-surface__content">{children}</div>
    </div>
  )
}

export default GlassSurface
