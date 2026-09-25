const UNITS = ["B", "KB", "MB", "GB", "TB", "PB"];

export function formatSize(bytes: number, lang = "de"): string {
  if (!Number.isFinite(bytes) || bytes < 0) return "";
  if (bytes < 1024) return `${bytes} B`;
  let v = bytes;
  let i = 0;
  while (v >= 1024 && i < UNITS.length - 1) {
    v /= 1024;
    i++;
  }
  const digits = v < 10 ? 1 : 0;
  return `${v.toLocaleString(lang, { maximumFractionDigits: digits, minimumFractionDigits: digits })} ${UNITS[i]}`;
}

export function formatSpeed(bps: number, lang = "de"): string {
  if (!bps || bps < 1) return "";
  return `${formatSize(Math.round(bps), lang)}/s`;
}

export function formatEta(remaining: number, bps: number): string {
  if (!bps || bps < 1 || remaining <= 0) return "";
  const s = Math.round(remaining / bps);
  if (s < 60) return `${s}s`;
  const m = Math.floor(s / 60);
  if (m < 60) return `${m}m ${s % 60}s`;
  return `${Math.floor(m / 60)}h ${m % 60}m`;
}

export function formatDate(ms: number | null, lang = "de"): string {
  if (!ms) return "";
  const d = new Date(ms);
  const now = new Date();
  const sameYear = d.getFullYear() === now.getFullYear();
  return d.toLocaleString(lang, {
    day: "2-digit",
    month: "2-digit",
    year: sameYear ? undefined : "numeric",
    hour: "2-digit",
    minute: "2-digit",
  });
}

export function formatTime(ms: number, lang = "de"): string {
  return new Date(ms).toLocaleTimeString(lang, { hour: "2-digit", minute: "2-digit", second: "2-digit" });
}

export function modeString(mode: number | null): string {
  if (mode === null || mode === undefined) return "";
  let s = "";
  for (const shift of [6, 3, 0]) {
    const bits = (mode >> shift) & 7;
    s += bits & 4 ? "r" : "-";
    s += bits & 2 ? "w" : "-";
    s += bits & 1 ? "x" : "-";
  }
  return s;
}

export function fileExt(name: string): string {
  const i = name.lastIndexOf(".");
  return i > 0 ? name.slice(i + 1).toLowerCase() : "";
}

// ---------------------------------------------------------------------------
// Remote paths ("/" separated)
// ---------------------------------------------------------------------------

export function rjoin(dir: string, name: string): string {
  if (!dir) return name;
  return dir.endsWith("/") ? dir + name : `${dir}/${name}`;
}

export function rparent(path: string): string {
  const trimmed = path.replace(/\/+$/, "");
  const i = trimmed.lastIndexOf("/");
  if (i <= 0) return "/";
  return trimmed.slice(0, i);
}

export function rcrumbs(path: string): { label: string; path: string }[] {
  const parts = path.split("/").filter(Boolean);
  const crumbs = [{ label: "/", path: "/" }];
  let acc = "";
  for (const p of parts) {
    acc += `/${p}`;
    crumbs.push({ label: p, path: acc });
  }
  return crumbs;
}

// ---------------------------------------------------------------------------
// Local paths (Windows or POSIX)
// ---------------------------------------------------------------------------

export function isWindowsPath(path: string): boolean {
  return /^[A-Za-z]:/.test(path) || path.includes("\\");
}

export function ljoin(dir: string, name: string): string {
  if (!dir || dir === "/") {
    // Windows drive list or POSIX root
    return isWindowsPath(name) ? name : `/${name}`;
  }
  const sep = isWindowsPath(dir) ? "\\" : "/";
  return dir.endsWith(sep) ? dir + name : dir + sep + name;
}

export function lparent(path: string): string {
  if (isWindowsPath(path)) {
    const trimmed = path.replace(/\\+$/, "");
    if (/^[A-Za-z]:$/.test(trimmed)) return "/"; // drive root -> drive list
    const i = trimmed.lastIndexOf("\\");
    if (i < 0) return "/";
    const parent = trimmed.slice(0, i);
    return /^[A-Za-z]:$/.test(parent) ? `${parent}\\` : parent;
  }
  return rparent(path);
}

export function lname(path: string): string {
  const trimmed = path.replace(/[\\/]+$/, "");
  const i = Math.max(trimmed.lastIndexOf("/"), trimmed.lastIndexOf("\\"));
  return i >= 0 ? trimmed.slice(i + 1) : trimmed;
}

export function lcrumbs(path: string, isWindowsOs: boolean): { label: string; path: string }[] {
  if (isWindowsPath(path) || (isWindowsOs && path === "/")) {
    const crumbs = isWindowsOs ? [{ label: "PC", path: "/" }] : [];
    if (path === "/") return crumbs;
    const parts = path.split("\\").filter(Boolean);
    let acc = "";
    parts.forEach((p, idx) => {
      acc = idx === 0 ? `${p}\\` : acc.endsWith("\\") ? acc + p : `${acc}\\${p}`;
      crumbs.push({ label: p, path: acc });
    });
    return crumbs;
  }
  return rcrumbs(path);
}

export function isHidden(name: string): boolean {
  return name.startsWith(".");
}
