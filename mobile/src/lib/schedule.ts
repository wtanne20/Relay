/** Wording for scheduled tasks and the queue. */
import type { QueueItem, Repeat } from './types';

export const DAY_NAMES = ['Mon', 'Tue', 'Wed', 'Thu', 'Fri', 'Sat', 'Sun'];

/** The repeat intervals offered for "Every…". */
export const EVERY_CHOICES = [
  { minutes: 15, label: '15 min' },
  { minutes: 30, label: '30 min' },
  { minutes: 60, label: '1 hour' },
  { minutes: 120, label: '2 hours' },
  { minutes: 360, label: '6 hours' },
  { minutes: 720, label: '12 hours' },
];

function pad(n: number): string {
  return String(n).padStart(2, '0');
}

/** "08:00" from a Date, in local time. */
export function hhmm(d: Date): string {
  return `${pad(d.getHours())}:${pad(d.getMinutes())}`;
}

/** "2026-11-12" from a Date, in local time. */
export function ymd(d: Date): string {
  return `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())}`;
}

/** Accepts "8", "8:5", "08:00", "0800" and "8.30"; returns "08:00" or null. */
export function cleanTime(input: string): string | null {
  const s = input.trim().replace('.', ':');
  const m = s.match(/^(\d{1,2})(?::?(\d{2}))?$/) ?? s.match(/^(\d{1,2}):(\d{1,2})$/);
  if (!m) return null;
  const h = Number(m[1]);
  const min = Number(m[2] ?? 0);
  if (h > 23 || min > 59) return null;
  return `${pad(h)}:${pad(min)}`;
}

/** Accepts "2026-11-12" or "2026-11-12"-like input; returns it or null. */
export function cleanDate(input: string): string | null {
  const m = input.trim().match(/^(\d{4})-(\d{1,2})-(\d{1,2})$/);
  if (!m) return null;
  const d = new Date(Number(m[1]), Number(m[2]) - 1, Number(m[3]));
  if (d.getMonth() !== Number(m[2]) - 1) return null;
  return ymd(d);
}

function plural(n: number, word: string): string {
  return `${n} ${word}${n === 1 ? '' : 's'}`;
}

/** "Weekdays at 08:00", "Every 2 hours", "Once, Thu 12 Nov at 08:00". */
export function describeRepeat(r: Repeat): string {
  switch (r.kind) {
    case 'once': {
      const d = new Date(r.at);
      return Number.isNaN(d.getTime()) ? `Once, ${r.at}` : `Once, ${dayLabel(d)} at ${hhmm(d)}`;
    }
    case 'daily': {
      const days = [...new Set(r.days)].sort();
      const key = days.join(',');
      let which: string;
      if (days.length === 0 || days.length === 7) which = 'Every day';
      else if (key === '1,2,3,4,5') which = 'Weekdays';
      else if (key === '6,7') which = 'Weekends';
      else which = days.map((d) => DAY_NAMES[d - 1]).join(', ');
      return `${which} at ${r.time}`;
    }
    case 'every': {
      const m = r.minutes;
      if (m % 1440 === 0) return m === 1440 ? 'Every day' : `Every ${plural(m / 1440, 'day')}`;
      if (m % 60 === 0) return m === 60 ? 'Every hour' : `Every ${plural(m / 60, 'hour')}`;
      return `Every ${plural(m, 'minute')}`;
    }
  }
}

/** "Today", "Tomorrow" or "Thu 12 Nov". */
function dayLabel(d: Date, now = new Date()): string {
  const start = (x: Date) => new Date(x.getFullYear(), x.getMonth(), x.getDate()).getTime();
  const days = Math.round((start(d) - start(now)) / 86400000);
  if (days === 0) return 'today';
  if (days === 1) return 'tomorrow';
  if (days === -1) return 'yesterday';
  return d.toLocaleDateString(undefined, { weekday: 'short', day: 'numeric', month: 'short' });
}

/** "in 25 min", "today at 18:30", "tomorrow at 08:00", "Thu 12 Nov at 08:00". */
export function whenLabel(iso: string, now = new Date()): string {
  const d = new Date(iso);
  if (Number.isNaN(d.getTime())) return '';
  const mins = Math.round((d.getTime() - now.getTime()) / 60000);
  if (mins >= 0 && mins < 1) return 'in a moment';
  if (mins > 0 && mins < 60) return `in ${mins} min`;
  return `${dayLabel(d, now)} at ${hhmm(d)}`;
}

/** What a queue row says about its state. */
export function queueStatusText(item: QueueItem, place: number): string {
  switch (item.status) {
    case 'queued':
      if (item.follow_up) return 'Next message, sent when the chat is free';
      return place === 1 ? 'Up next' : `Waiting · #${place} in line`;
    case 'running':
      return item.needs_approval ? 'Needs your approval' : 'Running';
    case 'done':
      return 'Done';
    case 'error':
      return item.error ? `Failed: ${item.error}` : 'Failed';
    case 'stopped':
      return item.error ?? 'Stopped';
  }
}
