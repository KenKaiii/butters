

const MB = 1024 * 1024;
const GB = 1024 * MB;

export function bytes(n: number): string {
  if (!(n > 0)) return "0 MB"; // nothing is nothing, not "1 MB"
  if (n >= GB) return `${(n / GB).toFixed(1)} GB`;
  return `${Math.max(1, Math.round(n / MB))} MB`;
}

export function duration(secs: number): string {
  if (secs < 3600) return `${Math.max(1, Math.round(secs / 60))} min`;
  const h = Math.floor(secs / 3600);
  const m = Math.round((secs % 3600) / 60);
  return m === 0 || h >= 10 ? `${h} h` : `${h} h ${m} min`;
}

