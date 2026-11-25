// Outlines drawn over the photo (brush circle, rectangle being dragged): a dark line under a
// light one, so they show on light and dark photos alike.

type Ctx = CanvasRenderingContext2D;

function twoTone(ctx: Ctx, path: () => void): void {
  ctx.save();
  ctx.lineWidth = 3;
  ctx.strokeStyle = 'rgb(0 0 0 / 60%)';
  ctx.beginPath();
  path();
  ctx.stroke();
  ctx.lineWidth = 1;
  ctx.strokeStyle = 'rgb(255 255 255)';
  ctx.beginPath();
  path();
  ctx.stroke();
  ctx.restore();
}

export function outlineCircle(ctx: Ctx, x: number, y: number, r: number): void {
  twoTone(ctx, () => ctx.arc(x, y, Math.max(1, r), 0, Math.PI * 2));
}

export function outlineBox(ctx: Ctx, x0: number, y0: number, x1: number, y1: number): void {
  twoTone(ctx, () => ctx.rect(Math.min(x0, x1), Math.min(y0, y1), Math.abs(x1 - x0), Math.abs(y1 - y0)));
}

/** A small cross, for the Rectangle tool in the magnifier. */
export function outlineCross(ctx: Ctx, x: number, y: number, r: number): void {
  twoTone(ctx, () => {
    ctx.moveTo(x - r, y);
    ctx.lineTo(x + r, y);
    ctx.moveTo(x, y - r);
    ctx.lineTo(x, y + r);
  });
}
