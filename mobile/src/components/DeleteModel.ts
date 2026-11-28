import { describeError, type RelayClient } from '../lib/api';
import { formatBytes } from '../lib/format';
import { alert } from './Dialogs';

/**
 * Asks before deleting a chat model's files from the PC, then deletes them.
 * Calls `onDeleted` once the files are gone.
 */
export function confirmDeleteModel(
  client: RelayClient,
  model: { path: string; name: string; size_bytes: number },
  onDeleted: () => void,
  setBusy?: (path: string | null) => void,
) {
  alert(
    `Delete ${model.name}?`,
    `This deletes the model from your PC and frees ${formatBytes(model.size_bytes)}. Chats that use it switch to the default model. You can download it again later.`,
    [
      { text: 'Cancel', style: 'cancel' },
      {
        text: 'Delete',
        style: 'destructive',
        onPress: async () => {
          setBusy?.(model.path);
          try {
            await client.deleteModel(model.path);
            onDeleted();
          } catch (e) {
            alert("Couldn't delete it", describeError(e));
          } finally {
            setBusy?.(null);
          }
        },
      },
    ],
  );
}
