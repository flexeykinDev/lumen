// Compose a shareable image of what is playing.
//
// Drawn here rather than in Rust because this side already has the fonts, the
// decoded artwork and the accent colour. `image` cannot render text, so the host
// would need a font rasteriser and a second copy of the layout, kept in step
// with the capsule by hand.

import { host } from "./bridge";
import type { NowPlaying } from "./types";

/** 2:1, which is what chat clients preview without cropping. */
const W = 1200;
const H = 600;

const PAD = 72;
const ART = H - PAD * 2;

// Ink is always light, because the background always ends up dark.
//
// The first version picked ink by testing the *accent* for lightness, which is
// not the colour anything is drawn on: the accent is laid down and then covered
// by a 62% black wash, so a pale peach accent still composites to near-black.
// A light album cover therefore produced dark text on a dark card — the title
// was barely legible and the track length disappeared entirely.
const INK = "#ffffff";
const INK_DIM = "rgba(255, 255, 255, 0.72)";
const INK_FAINT = "rgba(255, 255, 255, 0.45)";
const RAIL = "rgba(255, 255, 255, 0.20)";

function roundRect(
  ctx: CanvasRenderingContext2D,
  x: number,
  y: number,
  w: number,
  h: number,
  r: number,
): void {
  ctx.beginPath();
  ctx.moveTo(x + r, y);
  ctx.arcTo(x + w, y, x + w, y + h, r);
  ctx.arcTo(x + w, y + h, x, y + h, r);
  ctx.arcTo(x, y + h, x, y, r);
  ctx.arcTo(x, y, x + w, y, r);
  ctx.closePath();
}

/**
 * Shrink text until it fits, rather than letting it run off the card.
 *
 * Track titles have no length limit and routinely carry "(Official Video)" and
 * a feature list. Ellipsis alone would throw that away; scaling down first keeps
 * more of it readable, and the ellipsis is the last resort.
 */
function fitText(
  ctx: CanvasRenderingContext2D,
  text: string,
  maxWidth: number,
  startPx: number,
  minPx: number,
  weight: number,
): string {
  let size = startPx;
  for (; size > minPx; size -= 2) {
    ctx.font = `${weight} ${size}px "Segoe UI Variable Display", "Segoe UI", system-ui, sans-serif`;
    if (ctx.measureText(text).width <= maxWidth) return text;
  }
  ctx.font = `${weight} ${minPx}px "Segoe UI Variable Display", "Segoe UI", system-ui, sans-serif`;
  let clipped = text;
  while (clipped.length > 1 && ctx.measureText(`${clipped}…`).width > maxWidth) {
    clipped = clipped.slice(0, -1);
  }
  return clipped === text ? text : `${clipped}…`;
}

function loadArt(src: string): Promise<HTMLImageElement | null> {
  return new Promise((resolve) => {
    const img = new Image();
    img.onload = () => resolve(img);
    // A missing cover must not fail the whole card.
    img.onerror = () => resolve(null);
    img.src = src;
  });
}

function formatTime(seconds: number): string {
  if (!Number.isFinite(seconds) || seconds < 0) return "0:00";
  const total = Math.floor(seconds);
  const m = Math.floor(total / 60);
  const s = total % 60;
  return `${m}:${String(s).padStart(2, "0")}`;
}

/** Draw the card and hand the PNG to the host. */
export async function shareCard(now: NowPlaying, accentBase: string): Promise<void> {
  const canvas = document.createElement("canvas");
  canvas.width = W;
  canvas.height = H;
  const ctx = canvas.getContext("2d");
  if (!ctx) throw new Error("no 2d context for the share card");

  // Background: the accent, darkened, so the card belongs to the track rather
  // than to the app. Opaque throughout — the clipboard format has no usable
  // alpha, and a transparent card pastes as black.
  const bg = ctx.createLinearGradient(0, 0, W, H);
  bg.addColorStop(0, accentBase);
  bg.addColorStop(1, "#0b0d14");
  ctx.fillStyle = bg;
  ctx.fillRect(0, 0, W, H);
  ctx.fillStyle = "rgba(8, 9, 14, 0.62)";
  ctx.fillRect(0, 0, W, H);

  const art = now.artDataUri ? await loadArt(now.artDataUri) : null;
  if (art) {
    ctx.save();
    roundRect(ctx, PAD, PAD, ART, ART, 28);
    ctx.clip();
    ctx.drawImage(art, PAD, PAD, ART, ART);
    ctx.restore();
  } else {
    ctx.fillStyle = "rgba(255, 255, 255, 0.08)";
    roundRect(ctx, PAD, PAD, ART, ART, 28);
    ctx.fill();
  }

  const textX = PAD + ART + 56;
  const textW = W - textX - PAD;

  ctx.textBaseline = "alphabetic";
  ctx.fillStyle = INK;
  const title = fitText(ctx, now.title || "Nothing playing", textW, 60, 34, 700);
  ctx.fillText(title, textX, 250);

  ctx.fillStyle = INK_DIM;
  const artist = fitText(ctx, now.artist || "—", textW, 38, 24, 500);
  ctx.fillText(artist, textX, 306);

  // Progress, only when the source actually knows the length. A live stream has
  // no end, and a full-width bar would be a lie.
  const { positionSec, durationSec } = now.timeline;
  if (durationSec > 0) {
    const barY = 388;
    const barH = 10;
    ctx.fillStyle = RAIL;
    roundRect(ctx, textX, barY, textW, barH, barH / 2);
    ctx.fill();

    const played = Math.max(0, Math.min(1, positionSec / durationSec));
    if (played > 0) {
      ctx.fillStyle = accentBase;
      roundRect(ctx, textX, barY, Math.max(barH, textW * played), barH, barH / 2);
      ctx.fill();
    }

    ctx.font = `500 24px "Segoe UI", system-ui, sans-serif`;
    ctx.fillStyle = INK_DIM;
    ctx.fillText(formatTime(positionSec), textX, barY + 48);
    const total = formatTime(durationSec);
    ctx.fillText(total, textX + textW - ctx.measureText(total).width, barY + 48);
  }

  // Attribution, small and out of the way.
  ctx.font = `700 20px "Segoe UI", system-ui, sans-serif`;
  ctx.fillStyle = INK_FAINT;
  const via = now.source ? `${now.source.toUpperCase()}  ·  LUMEN` : "LUMEN";
  ctx.fillText(via, textX, H - PAD);

  const blob = await new Promise<Blob | null>((resolve) => canvas.toBlob(resolve, "image/png"));
  if (!blob) throw new Error("the share card produced no image");
  const bytes = Array.from(new Uint8Array(await blob.arrayBuffer()));
  await host.shareCard(bytes);
}
