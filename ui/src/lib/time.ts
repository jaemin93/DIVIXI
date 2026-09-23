/**
 * When something happened, the way a list shows it: today as a time,
 * yesterday marked as such, older as a date. The system clock and time
 * zone decide what "today" is; the interface language decides the words.
 */
import type { Lang } from "./i18n.svelte";

const locales: Record<Lang, string> = { ko: "ko-KR", en: "en-US" };

function sameDay(a: Date, b: Date): boolean {
  return a.getFullYear() === b.getFullYear() && a.getMonth() === b.getMonth() && a.getDate() === b.getDate();
}

/** `오후 02:22` today, `어제 오후 09:24` yesterday, `9월 22일` this year, else with the year. */
export function whenLabel(ms: number, now: number, lang: Lang): string {
  if (!ms) return "";
  const d = new Date(ms);
  const n = new Date(now);
  const locale = locales[lang];
  const time = new Intl.DateTimeFormat(locale, { hour: "2-digit", minute: "2-digit", hour12: lang === "ko" }).format(d);
  if (sameDay(d, n)) return time;
  const yesterday = new Date(n);
  yesterday.setDate(n.getDate() - 1);
  if (sameDay(d, yesterday)) return `${lang === "ko" ? "어제" : "Yesterday"} ${time}`;
  if (d.getFullYear() === n.getFullYear()) return new Intl.DateTimeFormat(locale, { month: "long", day: "numeric" }).format(d);
  return new Intl.DateTimeFormat(locale, { year: "numeric", month: "long", day: "numeric" }).format(d);
}

/** Full date and time, for a tooltip. */
export function whenFull(ms: number, lang: Lang): string {
  if (!ms) return "";
  return new Intl.DateTimeFormat(locales[lang], { dateStyle: "medium", timeStyle: "short" }).format(new Date(ms));
}

/** Milliseconds in a recency window id. */
export const WINDOWS: Record<string, number> = {
  "1h": 60 * 60 * 1000,
  "24h": 24 * 60 * 60 * 1000,
  "7d": 7 * 24 * 60 * 60 * 1000,
};
