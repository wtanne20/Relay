import type { IconName } from '../components/ui';
import type { AuditDecision, RuleKind } from './types';

export const RULE_KINDS: { value: RuleKind; label: string; icon: IconName }[] = [
  { value: 'command', label: 'Command', icon: 'console' },
  { value: 'files', label: 'Folder', icon: 'folder-edit-outline' },
  { value: 'web', label: 'Website', icon: 'web' },
];

/** A rule in plain words, e.g. "Commands starting with git status". */
export function ruleText(r: { kind: RuleKind; pattern: string }): string {
  switch (r.kind) {
    case 'command':
      return `Commands starting with ${r.pattern}`;
    case 'files':
      return r.pattern === '.' ? 'Changes to any file' : `Changes to files in ${r.pattern.replace(/\/$/, '')}/`;
    case 'web':
      return `Pages on ${r.pattern}`;
  }
}

/** How a tool call got to run, in a word or two. */
export const DECISION_LABEL: Record<AuditDecision, string> = {
  auto: 'Not asked',
  rule: 'Always allowed',
  allowed: 'You allowed',
  allowed_all: 'You allowed all',
  denied: 'You denied',
  stopped: 'Stopped',
  off: 'Tool off',
};

export const TOOL_ICON: Record<string, IconName> = {
  run_command: 'console',
  write_file: 'file-document-edit-outline',
  edit_file: 'file-document-edit-outline',
  read_file: 'file-document-outline',
  list_dir: 'folder-outline',
  fetch_url: 'web',
  web_search: 'magnify',
};

export const TOOL_VERB: Record<string, string> = {
  run_command: 'Ran',
  write_file: 'Wrote',
  edit_file: 'Edited',
  read_file: 'Read',
  list_dir: 'Listed',
  fetch_url: 'Opened',
  web_search: 'Searched',
};
