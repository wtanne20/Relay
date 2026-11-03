import { useState } from 'react';

import { TextEditor } from '../components/TextEditor';
import { useConnection } from '../lib/connection';

export default function SystemPromptScreen() {
  const { client } = useConnection();
  const [defaultText, setDefaultText] = useState<string | undefined>(undefined);
  if (!client) return null;
  return (
    <TextEditor
      intro="Instructions the agent gets at the start of every chat, in every project. Project details, AGENTS.md and skills are added after this automatically."
      load={async () => {
        const r = await client.systemPrompt();
        setDefaultText(r.default);
        return r.content;
      }}
      save={async (t) => (await client.saveSystemPrompt(t)).content}
      defaultText={defaultText}
    />
  );
}
