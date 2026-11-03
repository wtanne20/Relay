import { Stack, useLocalSearchParams } from 'expo-router';
import { useState } from 'react';

import { TextEditor } from '../../components/TextEditor';
import { useConnection } from '../../lib/connection';

/** Per-project notes added to the system prompt (like Bionic's project instructions). */
export default function ProjectInstructionsScreen() {
  const { id } = useLocalSearchParams<{ id: string }>();
  const { client } = useConnection();
  const [name, setName] = useState<string | null>(null);
  if (!client || !id) return null;
  return (
    <>
      <Stack.Screen options={{ title: name ? `${name} instructions` : 'Project instructions' }} />
      <TextEditor
        intro="Added to the agent's instructions for every chat in this project, on top of the project's AGENTS.md file."
        placeholder="e.g. This is a Next.js app. Run npm test before finishing. Never edit the migrations folder."
        load={async () => {
          const p = await client.project(id);
          setName(p.name);
          return p.instructions ?? '';
        }}
        save={async (t) => (await client.updateProject(id, { instructions: t })).instructions ?? ''}
      />
    </>
  );
}
